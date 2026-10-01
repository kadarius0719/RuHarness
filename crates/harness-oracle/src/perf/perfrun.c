/* perfrun — RuHarness's perf launcher (docs/PERF-DESIGN.md §3.3, build notes
 * 1-8), launcher version perf-launcher-2. macOS only (Linux: §6, not built).
 *
 * perfrun run PROFILE PERFGO DEADLINE PROGRAM NAME ARGS...
 * perfrun facts
 *
 * The one harness-built binary that runs unsandboxed: it starts the program
 * inside the perf profile through perfgo, reads the program's counters from
 * outside, and owns its end. fd 0 is the control socket the harness made:
 * perfrun writes `child <pid>` and, later, the record on it, and reads the
 * harness's go-ahead (`G`) and bye (`B`). stdout and stderr are the run's
 * capture pipes (or /dev/null) and the program inherits them.
 *
 * The record: `key value` lines, ASCII, at most 4 KiB, one write(), last line
 * `end`. `perfrun facts` writes only the facts lines and `end`, on stdout. */
#ifndef __APPLE__
#error "perfrun is built for macOS only"
#endif

#include <errno.h>
#include <fcntl.h>
#include <libproc.h>
#include <mach/mach_time.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/event.h>
#include <sys/proc_info.h>
#include <sys/resource.h>
#include <sys/sysctl.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define CTL 0
#define RECORD_MAX 4096
/* The longest DEADLINE, in seconds: the longest `[oracle] timeout_secs` the
 * harness accepts (a week) plus step 1's extra minute. A test beside the
 * harness's side keeps the two in step. */
#define DEADLINE_MAX 604860

static char record[RECORD_MAX];
static size_t record_len = 0;
static int record_full = 0;

static void put(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    char line[512];
    int n = vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    if (n < 0) return;
    size_t len = (size_t)n < sizeof line ? (size_t)n : sizeof line - 1;
    if (record_len + len + 1 > RECORD_MAX - 5) {
        record_full = 1;
        return;
    }
    memcpy(record + record_len, line, len);
    record_len += len;
    record[record_len++] = '\n';
}

/* A sysctl string, made safe for a record line: printable ASCII only,
 * at most 120 characters. */
static void put_text(const char *key, const char *name) {
    char buf[256];
    size_t size = sizeof buf - 1;
    if (sysctlbyname(name, buf, &size, NULL, 0) != 0) return;
    buf[size < sizeof buf ? size : sizeof buf - 1] = '\0';
    char clean[121];
    size_t k = 0;
    for (size_t i = 0; buf[i] != '\0' && k < sizeof clean - 1; i++) {
        unsigned char c = (unsigned char)buf[i];
        clean[k++] = (c >= 0x20 && c < 0x7f) ? (char)c : '?';
    }
    while (k > 0 && clean[k - 1] == ' ') k--;
    clean[k] = '\0';
    if (k > 0) put("%s %s", key, clean);
}

static int two_kinds(void) {
    int levels = 0;
    size_t size = sizeof levels;
    if (sysctlbyname("hw.nperflevels", &levels, &size, NULL, 0) != 0) return 0;
    return levels >= 2;
}

static void put_facts(void) {
    put_text("cpu", "machdep.cpu.brand_string");
    put_text("os", "kern.osproductversion");
    put_text("build", "kern.osversion");
    put_text("arch", "hw.machine");
    int fast = 0;
    size_t size = sizeof fast;
    if (sysctlbyname("hw.perflevel0.logicalcpu", &fast, &size, NULL, 0) != 0) {
        size = sizeof fast;
        if (sysctlbyname("hw.logicalcpu", &fast, &size, NULL, 0) != 0) fast = 0;
    }
    put("fast_cores %d", fast);
    put("two_kinds %d", two_kinds());
}

static void write_all(int fd, const char *buf, size_t len) {
    while (len > 0) {
        ssize_t n = write(fd, buf, len);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return;
        buf += n;
        len -= (size_t)n;
    }
}

/* The program's counters (`proc_pid_rusage`), V6 with the performance-core
 * fields, else V4 without them. */
struct counts {
    int ok, v6;
    uint64_t instructions, cycles, p_instructions, p_cycles;
    uint64_t user, system; /* mach ticks */
    uint64_t footprint;
};

static struct counts read_counts(pid_t pid) {
    struct counts c;
    memset(&c, 0, sizeof c);
    struct rusage_info_v6 v6;
    memset(&v6, 0, sizeof v6);
    if (proc_pid_rusage(pid, RUSAGE_INFO_V6, (rusage_info_t *)&v6) == 0) {
        c.ok = 1;
        c.v6 = 1;
        c.instructions = v6.ri_instructions;
        c.cycles = v6.ri_cycles;
        c.p_instructions = v6.ri_pinstructions;
        c.p_cycles = v6.ri_pcycles;
        c.user = v6.ri_user_time;
        c.system = v6.ri_system_time;
        c.footprint = v6.ri_lifetime_max_phys_footprint;
        return c;
    }
    struct rusage_info_v4 v4;
    memset(&v4, 0, sizeof v4);
    if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&v4) == 0) {
        c.ok = 1;
        c.instructions = v4.ri_instructions;
        c.cycles = v4.ri_cycles;
        c.user = v4.ri_user_time;
        c.system = v4.ri_system_time;
        c.footprint = v4.ri_lifetime_max_phys_footprint;
    }
    return c;
}

static uint64_t ticks_to_us(uint64_t ticks) {
    static mach_timebase_info_data_t tb;
    if (tb.denom == 0) mach_timebase_info(&tb);
    if (tb.denom == 0) return 0;
    return (uint64_t)((__uint128_t)ticks * tb.numer / tb.denom / 1000u);
}

static uint64_t now_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC_RAW, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

/* The program's group and the program itself: the child may not have
 * called setsid() yet (build note 8). */
static void kill_child(pid_t pid) {
    kill(-pid, SIGKILL);
    kill(pid, SIGKILL);
}

/* Wait for the harness's bye (`B`) or end-of-file, discarding a pending
 * go-ahead (build note 3). */
static void wait_bye(void) {
    for (;;) {
        char c;
        ssize_t n = read(CTL, &c, 1);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0 || c == 'B') return;
    }
}

static void close_fds_but(int keep0, int keep1, int keep2) {
    int size = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, NULL, 0);
    if (size <= 0) return;
    struct proc_fdinfo *fds = malloc((size_t)size);
    if (fds == NULL) return;
    size = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, fds, size);
    int n = size > 0 ? size / (int)sizeof(struct proc_fdinfo) : 0;
    for (int i = 0; i < n; i++) {
        int fd = fds[i].proc_fd;
        if (fd > 2 && fd != keep0 && fd != keep1 && fd != keep2) close(fd);
    }
    free(fds);
}

/* A watch the loop does not wait on: removed, so it can never fire again
 * and keep perfrun busy while it should only wait. */
static void forget_watch(int kq, const struct kevent *e) {
    struct kevent del;
    EV_SET(&del, e->ident, e->filter, EV_DELETE, 0, 0, 0);
    kevent(kq, &del, 1, NULL, 0, NULL);
}

static int launcher_failed(const char *what) {
    put("status launcher %s (%s)", what, strerror(errno));
    put("end");
    write_all(CTL, record, record_len);
    return 1;
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "facts") == 0) {
        put_facts();
        put("end");
        write_all(1, record, record_len);
        return 0;
    }
    if (argc < 7 || strcmp(argv[1], "run") != 0) {
        fprintf(stderr, "usage: perfrun run PROFILE PERFGO DEADLINE PROGRAM NAME ARGS... | perfrun facts\n");
        return 2;
    }
    const char *profile = argv[2], *perfgo = argv[3], *program = argv[5];
    char *end = NULL;
    long deadline = strtol(argv[4], &end, 10);
    if (end == argv[4] || *end != '\0' || deadline < 1 || deadline > DEADLINE_MAX) {
        errno = EINVAL;
        return launcher_failed("bad deadline");
    }

    int kq = kqueue();
    if (kq < 0) return launcher_failed("kqueue");
    struct kevent ev;
    /* The SIGTERM watch before SIGTERM is ignored, before the fork (note 8). */
    EV_SET(&ev, SIGTERM, EVFILT_SIGNAL, EV_ADD, 0, 0, 0);
    if (kevent(kq, &ev, 1, NULL, 0, NULL) < 0) return launcher_failed("kevent signal");
    signal(SIGTERM, SIG_IGN);
    signal(SIGPIPE, SIG_IGN);

    int ready[2], go[2], status[2];
    if (pipe(ready) != 0 || pipe(go) != 0 || pipe(status) != 0) return launcher_failed("pipe");

    pid_t pid = fork();
    if (pid < 0) return launcher_failed("fork");
    if (pid == 0) {
        /* The child: default dispositions and an empty mask (note 8), its own
         * session and group, stdin /dev/null, exactly its pipe ends kept
         * (note 2). */
        for (int s = 1; s < NSIG; s++) signal(s, SIG_DFL);
        sigset_t none;
        sigemptyset(&none);
        sigprocmask(SIG_SETMASK, &none, NULL);
        setsid();
        int devnull = open("/dev/null", O_RDONLY);
        if (devnull >= 0) {
            dup2(devnull, 0);
            if (devnull != 0) close(devnull);
        }
        close_fds_but(ready[1], go[0], status[1]);
        char rfd[16], gfd[16], sfd[16];
        snprintf(rfd, sizeof rfd, "%d", ready[1]);
        snprintf(gfd, sizeof gfd, "%d", go[0]);
        snprintf(sfd, sizeof sfd, "%d", status[1]);
        int extra = argc - 6; /* NAME and ARGS */
        char **av = calloc((size_t)(extra + 9), sizeof *av);
        if (av == NULL) _exit(127);
        int k = 0;
        av[k++] = "sandbox-exec";
        av[k++] = "-p";
        av[k++] = (char *)profile;
        av[k++] = (char *)perfgo;
        av[k++] = rfd;
        av[k++] = gfd;
        av[k++] = sfd;
        av[k++] = (char *)program;
        for (int i = 6; i < argc; i++) av[k++] = argv[i];
        av[k] = NULL;
        execv("/usr/bin/sandbox-exec", av);
        _exit(127);
    }
    close(ready[1]);
    close(go[0]);
    close(status[1]);
    {
        char line[64];
        int n = snprintf(line, sizeof line, "child %d\n", (int)pid);
        if (n > 0) write_all(CTL, line, (size_t)n);
    }

    /* Each watch added on its own (EV_RECEIPT): the child's exit, the control
     * socket, ready. A child already gone is an exit seen now; the other
     * watches are kept. */
    int exited = 0;
    struct kevent add[3], got[3];
    EV_SET(&add[0], pid, EVFILT_PROC, EV_ADD | EV_RECEIPT, NOTE_EXIT, 0, 0);
    EV_SET(&add[1], CTL, EVFILT_READ, EV_ADD | EV_RECEIPT, 0, 0, 0);
    EV_SET(&add[2], ready[0], EVFILT_READ, EV_ADD | EV_RECEIPT, 0, 0, 0);
    int r = kevent(kq, add, 3, got, 3, NULL);
    for (int i = 0; i < r; i++) {
        if ((got[i].flags & EV_ERROR) && got[i].data != 0) {
            if (got[i].filter == EVFILT_PROC && got[i].data == ESRCH) exited = 1;
            else if (got[i].filter != EVFILT_PROC) return launcher_failed("kevent watch");
        }
    }

    struct counts base;
    memset(&base, 0, sizeof base);
    int got_ready = 0, ready_gone = 0, got_go = 0, stopped = 0, harness_gone = 0;
    int timed_out = 0, killed = 0, started = 0, exec_errno = 0;
    uint64_t go_at = 0;

    /* Before go: one wait for everything (§3.3 step 4, note 1). */
    while (!started && !exited && !stopped && !harness_gone && !(ready_gone && !got_ready)) {
        struct kevent e;
        int n = kevent(kq, NULL, 0, &e, 1, NULL);
        if (n < 0) {
            if (errno == EINTR) continue;
            return launcher_failed("kevent wait");
        }
        if (n == 0) continue;
        if (e.filter == EVFILT_SIGNAL) {
            stopped = 1;
        } else if (e.filter == EVFILT_PROC) {
            exited = 1;
        } else if ((int)e.ident == CTL) {
            char c;
            ssize_t m = e.data > 0 ? read(CTL, &c, 1) : 0;
            if (m == 1 && c == 'G') got_go = 1;
            else if (m == 0 || (e.flags & EV_EOF)) harness_gone = 1;
        } else if (ready[0] >= 0 && (int)e.ident == ready[0]) {
            char c;
            ssize_t m = read(ready[0], &c, 1);
            if (m == 1) {
                got_ready = 1;
                base = read_counts(pid);
            } else if (m == 0) {
                ready_gone = 1;
            }
            if (m >= 0) {
                /* ready is read once. Closed now, its watch goes with it:
                 * perfgo closes its end just before the exec, and a watch
                 * left on that end-of-file would fire on every wait and keep
                 * perfrun busy for the whole run (§4: perfrun stays idle). */
                close(ready[0]);
                ready[0] = -1;
            }
        } else {
            forget_watch(kq, &e);
        }
        if (got_ready && got_go && !stopped && !harness_gone && !exited) {
            char g = 'g';
            write_all(go[1], &g, 1);
            close(go[1]);
            /* status: end-of-file means the exec happened. */
            int e2 = 0;
            ssize_t m;
            do {
                m = read(status[0], &e2, sizeof e2);
            } while (m < 0 && errno == EINTR);
            if (m == (ssize_t)sizeof e2) exec_errno = e2 > 0 ? e2 : EIO;
            started = 1;
            go_at = now_us();
            EV_SET(&ev, 1, EVFILT_TIMER, EV_ADD | EV_ONESHOT, NOTE_SECONDS, deadline, 0);
            kevent(kq, &ev, 1, NULL, 0, NULL);
        }
    }

    if (harness_gone && !started) {
        /* The harness is gone or cancelled: the program never runs. */
        kill_child(pid);
        int st;
        waitpid(pid, &st, 0);
        return 0;
    }
    if (stopped && !started) {
        kill_child(pid);
        killed = 1;
    }

    /* While the program runs: perfrun owns its end (§3.3 step 5). */
    while (started && !exited) {
        struct kevent e;
        int n = kevent(kq, NULL, 0, &e, 1, NULL);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;
        }
        if (n == 0) continue;
        if (e.filter == EVFILT_PROC) {
            exited = 1;
        } else if (e.filter == EVFILT_TIMER) {
            timed_out = 1;
            kill_child(pid);
            killed = 1;
        } else if (e.filter == EVFILT_SIGNAL) {
            stopped = 1;
            kill_child(pid);
            killed = 1;
        } else if ((int)e.ident == CTL) {
            char c;
            ssize_t m = e.data > 0 ? read(CTL, &c, 1) : 0;
            if (m == 0 || (e.flags & EV_EOF)) {
                harness_gone = 1;
                stopped = 1;
                kill_child(pid);
                killed = 1;
                EV_SET(&ev, CTL, EVFILT_READ, EV_DELETE, 0, 0, 0);
                kevent(kq, &ev, 1, NULL, 0, NULL);
            }
        } else {
            forget_watch(kq, &e);
        }
    }

    /* The child is a zombie: its pid reserved until the bye. */
    siginfo_t si;
    memset(&si, 0, sizeof si);
    int w;
    do {
        w = waitid(P_PID, (id_t)pid, &si, WEXITED | WNOWAIT);
    } while (w < 0 && errno == EINTR);
    uint64_t wall = started ? now_us() - go_at : 0;
    struct counts end_counts = read_counts(pid);

    if (exec_errno != 0) put("status never-started %d", exec_errno);
    else if (!got_ready && !stopped) put("status never-started no-ready");
    else if (timed_out) put("status timeout");
    else if (stopped) put("status stopped");
    else put("status ok");
    if (w == 0) {
        if (si.si_code == CLD_EXITED) put("ended exit %d", si.si_status);
        else put("ended signal %d", si.si_status);
    }
    put("killed %d", killed);
    if (started && exec_errno == 0 && end_counts.ok && base.ok) {
        /* End minus baseline; a zero baseline is never subtracted (§3.3). */
        if (end_counts.instructions > base.instructions)
            put("instructions %llu", (unsigned long long)(end_counts.instructions - base.instructions));
        if (end_counts.cycles > base.cycles)
            put("cycles %llu", (unsigned long long)(end_counts.cycles - base.cycles));
        if (end_counts.v6 && base.v6 && two_kinds()) {
            put("p_instructions %llu", (unsigned long long)(end_counts.p_instructions >= base.p_instructions ? end_counts.p_instructions - base.p_instructions : 0));
            put("p_cycles %llu", (unsigned long long)(end_counts.p_cycles >= base.p_cycles ? end_counts.p_cycles - base.p_cycles : 0));
        }
        uint64_t cpu_end = end_counts.user + end_counts.system;
        uint64_t cpu_base = base.user + base.system;
        if (cpu_end > cpu_base) put("cpu_us %llu", (unsigned long long)ticks_to_us(cpu_end - cpu_base));
        if (end_counts.footprint > 0) put("memory %llu", (unsigned long long)end_counts.footprint);
        if (wall > 0) put("wall_us %llu", (unsigned long long)wall);
    }
    double load[1] = {0};
    if (getloadavg(load, 1) == 1 && load[0] >= 0) put("load %u", (unsigned)(load[0] * 100.0 + 0.5));
    put_facts();
    if (record_full) {
        record_len = 0;
        record_full = 0;
        put("status launcher record too long");
    }
    put("end");
    if (!harness_gone) {
        write_all(CTL, record, record_len);
        wait_bye();
    }
    int st;
    waitpid(pid, &st, 0);
    return 0;
}
