/* perfgo — RuHarness's perf trampoline (docs/PERF-DESIGN.md §3.3 step 3),
 * launcher version perf-launcher-2.
 *
 * perfgo READY GO STATUS PROGRAM NAME ARGS...
 *
 * Runs inside the perf profile, started by perfrun through sandbox-exec. It
 * marks STATUS close-on-exec, writes one byte on READY (perfrun takes the
 * program's baseline counters while perfgo waits), reads one byte on GO —
 * end-of-file instead means perfrun is gone: it exits 125 and the program
 * never runs — closes both, and becomes PROGRAM with argv[0] = NAME. When
 * the exec fails it writes errno on STATUS and exits 127. No CPU limit. */
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

static int fd_arg(const char *s) {
    char *end = NULL;
    long v = strtol(s, &end, 10);
    if (end == s || *end != '\0' || v < 3 || v > 65535) return -1;
    return (int)v;
}

int main(int argc, char **argv) {
    if (argc < 6) return 125;
    int ready = fd_arg(argv[1]), go = fd_arg(argv[2]), status = fd_arg(argv[3]);
    if (ready < 0 || go < 0 || status < 0) return 125;
    if (fcntl(status, F_SETFD, FD_CLOEXEC) != 0) return 125;
    char c = 'r';
    if (write(ready, &c, 1) != 1) return 125;
    ssize_t n;
    do {
        n = read(go, &c, 1);
    } while (n < 0 && errno == EINTR);
    if (n != 1) return 125;
    close(ready);
    close(go);
    execv(argv[4], argv + 5);
    int e = errno;
    ssize_t w = write(status, &e, sizeof e);
    (void)w;
    return 127;
}
