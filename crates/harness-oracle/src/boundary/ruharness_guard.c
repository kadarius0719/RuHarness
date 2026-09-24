/* RuHarness boundary-check runtime (docs/ORACLE-HARDENING.md §B.3; §B.R-1).
 *
 * Harness-owned C, written into the unit build dir by the oracle and linked
 * into every boundary build. The unit's validated driver.c is compiled
 * UNMODIFIED with -D<sym>=ruharness_call_<sym>; the generated wrapper is this
 * runtime's only client. Compiled without sanitizer coverage; with
 * -DRUHARNESS_MEASURE in the measure build, where driver objects are located
 * through AddressSanitizer.
 *
 * Mode (RUHARNESS_GUARD, harness-set; the confined run's environment is
 * otherwise cleared):
 *   measure               arguments pass through; the sanitizer-coverage
 *                         callbacks record, per (call, object), the byte hull
 *                         the C unit touches; the record goes to
 *                         $TMPDIR/ruharness-guard.out
 *   learn-tail|learn-head each object a call receives is copied into a fresh
 *                         guarded shadow exposing only its window (from
 *                         RUHARNESS_GUARD_WINDOWS); an in-call fault inside a
 *                         shadow is recorded, the object opened, and the run
 *                         continues
 *   tail|head             the same, but an in-call fault ends the run with
 *                         exit status 97
 *
 * Fail-closed integrity (§B.R-1): after every call and at normal end the
 * runtime verifies signal accounting, that its own fault handler is still
 * installed, that no exception port for bad accesses was added, that a
 * private guarded page still faults into the handler, and that every
 * reservation's closed pages are still PROT_NONE and unaliased. Any
 * deviation is RH-TAMPER, exit 97. A process that ends inside a unit call
 * (the candidate calling exit) is RH-EXITED, exit 97, and a run that makes
 * fewer calls than the table is RH-DIVERGED — the candidate's doing, never
 * a harness fault.
 *
 * Single-threaded by contract. Never prints an address. */
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <setjmp.h>
#include <unistd.h>
#include <fcntl.h>
#include <pthread.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <stdio.h>
#ifdef __APPLE__
#include <mach/mach.h>
#include <mach/mach_vm.h>
#endif
#ifdef RUHARNESS_MEASURE
#include <sanitizer/asan_interface.h>
#endif
#include "ruharness_guard_internal.h"

#define RH_MAX_CALLS 65536u
#define RH_MAX_OBJS 16
#define RH_MAX_ARGS 64
#define RH_MAX_BYTES ((size_t)16 << 20)
#define RH_MAX_RES 65536
#define RH_GAP_MIN ((size_t)1 << 20) /* §B.R-12: a gap of at least max(1 MiB, size) on each side */

enum { M_MEASURE, M_LEARN_TAIL, M_LEARN_HEAD, M_TAIL, M_HEAD };
static const char *const MODE_NAME[] = {"measure", "learn-tail", "learn-head", "tail", "head"};
enum { K_NULL, K_PASS, K_OBJ };

/* one object of the current call */
typedef struct {
    uintptr_t orig;                 /* the driver's object */
    size_t size, elem;              /* bytes; bytes per element */
    size_t lo, hi;                  /* window in ELEMENTS (tight/learn) */
    long mlo, mhi;                  /* measure: byte hull of traced in-call accesses */
    uintptr_t res, res_end, shadow; /* tight/learn: reservation and the copy (0 = none yet) */
    unsigned char *snap;            /* tight/learn: the bytes as copied in */
    unsigned char opened;           /* learn: the whole object was opened after a fault */
} rh_obj;

/* every reservation ever made this run (never unmapped, never reused) */
typedef struct {
    uintptr_t res, res_end, shadow;
    size_t size;
    unsigned call;
    int obj;
    uintptr_t open_lo, open_hi; /* the pages left open (0,0 = none) */
    unsigned share_mode, ref_count; /* mach top-info as observed at creation */
} rh_res;

static rh_obj O[RH_MAX_OBJS];
static int n_obj;
static rh_res *R;
static int n_res;
static int mode = -1;
static size_t pg;
static unsigned depth, call_n;
static uintptr_t frame_lo, stack_hi;
static int out_fd = -1;
static int foreign_reported;
static volatile unsigned char probe_cell;
static unsigned long probe_hits;

/* the window table (learn and tight modes) */
typedef struct {
    int sym;
    size_t obj_start, nobj, arg_start, nargs;
} rh_tcall;
typedef struct {
    size_t size, elem, lo, hi;
} rh_tobj;
typedef struct {
    int param, kind, obj;
    size_t off;
} rh_targ;
static size_t t_calls;
static rh_tcall *t_call;
static rh_tobj *t_obj;
static rh_targ *t_arg;
static size_t args_seen; /* of the current call */

/* integrity */
static struct sigaction our_sa;
static long signals_before;         /* ru_nsignals at enter */
static volatile long our_deliveries; /* faults our handler serviced in this call */
static uintptr_t canary_page;
static sigjmp_buf canary_jb;
static volatile sig_atomic_t canary_active;
#ifdef __APPLE__
typedef struct {
    mach_msg_type_number_t count;
    exception_mask_t masks[EXC_TYPES_COUNT];
    mach_port_t ports[EXC_TYPES_COUNT];
    exception_behavior_t behaviors[EXC_TYPES_COUNT];
    thread_state_flavor_t flavors[EXC_TYPES_COUNT];
} rh_ports;
static rh_ports task_ports0, thread_ports0;
#endif

/* ---- async-signal-safe output ---- */

static void put(int fd, const char *s) {
    size_t n = strlen(s);
    while (n) {
        ssize_t w = write(fd, s, n);
        if (w <= 0) return;
        s += w;
        n -= (size_t)w;
    }
}

static void fmt_num(char *buf, size_t *len, size_t cap, long v) {
    char d[24];
    int i = 23;
    unsigned long u = v < 0 ? 0ul - (unsigned long)v : (unsigned long)v;
    d[i] = 0;
    do {
        d[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (v < 0) d[--i] = '-';
    for (const char *p = d + i; *p && *len + 1 < cap; p++) buf[(*len)++] = *p;
    buf[*len] = 0;
}

static void fmt_str(char *buf, size_t *len, size_t cap, const char *s) {
    for (; *s && *len + 1 < cap; s++) buf[(*len)++] = *s;
    buf[*len] = 0;
}

/* "<word> <v0> … <v(nv-1)>\n" to the out file */
static void rec(const char *word, const long *v, int nv) {
    if (out_fd < 0) return;
    char l[160];
    size_t n = 0;
    fmt_str(l, &n, sizeof l, word);
    for (int i = 0; i < nv; i++) {
        fmt_str(l, &n, sizeof l, " ");
        fmt_num(l, &n, sizeof l, v[i]);
    }
    fmt_str(l, &n, sizeof l, "\n");
    put(out_fd, l);
}

static void rec_words(const char *a, const char *b) {
    if (out_fd < 0) return;
    put(out_fd, a);
    put(out_fd, " ");
    put(out_fd, b);
    put(out_fd, "\n");
}

/* A harness-side failure of the runtime (a limit, a table, a syscall): the
 * check is not applicable, never candidate evidence. */
static void die(const char *why) {
    put(2, "\nRH-ERROR ");
    put(2, why);
    put(2, "\n");
    rec_words("error", why);
    _exit(96);
}

/* A red outcome with a harness-worded reason. */
static void red(const char *marker, const char *word, const char *what) {
    put(2, "\n");
    put(2, marker);
    put(2, " ");
    put(2, what);
    put(2, "\n");
    rec_words(word, what);
    _exit(97);
}

/* ---- memory ---- */

static size_t rup(size_t x) { return (x + pg - 1) / pg * pg; }
static uintptr_t pfloor(uintptr_t x) { return x / pg * pg; }
static uintptr_t pceil(uintptr_t x) { return (x + pg - 1) / pg * pg; }

static void protect(uintptr_t s, uintptr_t e, int p) {
    if (e > s && mprotect((void *)s, e - s, p) != 0) die("mprotect failed");
}

static int tight(void) { return mode >= M_LEARN_TAIL; }
static int learning(void) { return mode == M_LEARN_TAIL || mode == M_LEARN_HEAD; }
static int head_layout(void) { return mode == M_HEAD || mode == M_LEARN_HEAD; }

/* the reservation containing `a` (linear: faults are rare) */
static int find_res(uintptr_t a) {
    for (int i = n_res - 1; i >= 0; i--)
        if (a >= R[i].res && a < R[i].res_end) return i;
    return -1;
}

/* ---- the fault handler ---- */

static void fault_line(const char *what, const long *v, int nv) {
    char l[160];
    size_t n = 0;
    fmt_str(l, &n, sizeof l, "\nRH-FAULT ");
    fmt_str(l, &n, sizeof l, what);
    for (int i = 0; i < nv; i++) {
        fmt_str(l, &n, sizeof l, " ");
        fmt_num(l, &n, sizeof l, v[i]);
    }
    fmt_str(l, &n, sizeof l, "\n");
    put(2, l);
}

static void on_fault(int sig, siginfo_t *si, void *uc) {
    (void)uc;
    uintptr_t addr = (uintptr_t)si->si_addr;
    if (canary_active && addr >= canary_page && addr < canary_page + pg) {
        our_deliveries++;
        siglongjmp(canary_jb, 1);
    }
    int i = tight() ? find_res(addr) : -1;
    if (i < 0 || !depth) { /* not ours, or outside a call: die by the signal */
        signal(sig, SIG_DFL);
        return;
    }
    long byte = (long)(addr - R[i].shadow);
    if (R[i].call != call_n) { /* a shadow of an earlier call: a retained pointer */
        long v[4] = {(long)call_n, (long)R[i].call, R[i].obj, byte};
        fault_line("stale", v, 4);
        rec("stale", v, 4);
        _exit(97);
    }
    long e[3] = {(long)call_n, R[i].obj, byte};
    /* An access outside the object cannot be satisfied by opening it: terminal
     * in every mode (RT-3: re-opening would re-fault forever). */
    if (learning() && byte >= 0 && (size_t)byte < R[i].size && !O[R[i].obj].opened) {
        our_deliveries++;
        rec("learn", e, 3);
        rh_obj *o = &O[R[i].obj];
        o->opened = 1;
        protect(pfloor(o->shadow), pceil(o->shadow + o->size), PROT_READ | PROT_WRITE);
        return;
    }
    fault_line("at", e, 3);
    rec("fault", e, 3);
    _exit(97);
}

/* ---- integrity (§B.R-1) ---- */

#ifdef __APPLE__
static void snapshot_ports(rh_ports *p, int thread) {
    memset(p, 0, sizeof *p);
    p->count = EXC_TYPES_COUNT;
    kern_return_t kr;
    if (thread) {
        thread_t t = mach_thread_self();
        kr = thread_get_exception_ports(t, EXC_MASK_BAD_ACCESS, p->masks, &p->count, p->ports,
                                        p->behaviors, p->flavors);
        mach_port_deallocate(mach_task_self(), t);
    } else {
        kr = task_get_exception_ports(mach_task_self(), EXC_MASK_BAD_ACCESS, p->masks, &p->count,
                                      p->ports, p->behaviors, p->flavors);
    }
    if (kr != KERN_SUCCESS) die("exception ports unreadable");
}

static int same_ports(const rh_ports *a, const rh_ports *b) {
    if (a->count != b->count) return 0;
    for (mach_msg_type_number_t i = 0; i < a->count; i++)
        if (a->masks[i] != b->masks[i] || a->ports[i] != b->ports[i]
            || a->behaviors[i] != b->behaviors[i] || a->flavors[i] != b->flavors[i])
            return 0;
    return 1;
}
#endif

static long signals_now(void) {
    struct rusage ru;
    if (getrusage(RUSAGE_SELF, &ru) != 0) die("getrusage failed");
    return ru.ru_nsignals;
}

/* Signal accounting (§B.R-1): every signal delivered since `enter` must have
 * been serviced by our handler — a candidate that swaps the handler, survives
 * its own fault and restores the handler before returning still leaves the
 * delivery count behind. */
static void account_signals(void) {
    long now = signals_now();
    if (now - signals_before != our_deliveries) red("RH-TAMPER", "tamper", "signal");
}

static void integrity(void) {
    const int sigs[2] = {SIGSEGV, SIGBUS};
    for (int i = 0; i < 2; i++) {
        struct sigaction cur;
        memset(&cur, 0, sizeof cur);
        if (sigaction(sigs[i], NULL, &cur) != 0 || cur.sa_sigaction != our_sa.sa_sigaction
            || (cur.sa_flags & (SA_SIGINFO | SA_ONSTACK)) != (SA_SIGINFO | SA_ONSTACK))
            red("RH-TAMPER", "tamper", "handler");
    }
#ifdef __APPLE__
    rh_ports now;
    snapshot_ports(&now, 0);
    if (!same_ports(&now, &task_ports0)) red("RH-TAMPER", "tamper", "exception-port");
    snapshot_ports(&now, 1);
    if (!same_ports(&now, &thread_ports0)) red("RH-TAMPER", "tamper", "exception-port");
#endif
    canary_active = 1;
    if (sigsetjmp(canary_jb, 1) == 0) {
        volatile unsigned char *c = (volatile unsigned char *)canary_page;
        unsigned char v = *c;
        (void)v;
        canary_active = 0;
        red("RH-TAMPER", "tamper", "canary"); /* the read did not fault into us */
    }
    canary_active = 0;
}

/* ---- the window table ---- */

static int read_word(FILE *f, const char *word) {
    char w[32];
    return fscanf(f, "%31s", w) == 1 && strcmp(w, word) == 0;
}

static int read_nums(FILE *f, unsigned long *v, int nv) {
    for (int i = 0; i < nv; i++)
        if (fscanf(f, "%lu", &v[i]) != 1) return 0;
    return 1;
}

static void load_table(void) {
    const char *p = getenv("RUHARNESS_GUARD_WINDOWS");
    FILE *f = p ? fopen(p, "r") : NULL;
    if (!f) die("window table missing");
    unsigned long h[2];
    char layout[16];
    const char *want = learning() ? MODE_NAME[mode] + 6 : MODE_NAME[mode];
    if (!read_word(f, "ruharness-windows") || !read_nums(f, h, 2) || h[0] != 1
        || fscanf(f, "%15s", layout) != 1 || strcmp(layout, want) != 0)
        die("bad window table header");
    t_calls = h[1];
    if (t_calls > RH_MAX_CALLS) die("window table too large");
    t_call = calloc(t_calls + 1, sizeof *t_call);
    t_obj = calloc(t_calls * RH_MAX_OBJS + 1, sizeof *t_obj);
    t_arg = calloc(t_calls * RH_MAX_ARGS + 1, sizeof *t_arg);
    if (!t_call || !t_obj || !t_arg) die("out of memory");
    size_t io = 0, ia = 0;
    for (size_t n = 1; n <= t_calls; n++) {
        unsigned long v[4];
        if (!read_word(f, "call") || !read_nums(f, v, 4) || v[0] != n || v[2] > RH_MAX_OBJS
            || v[3] > RH_MAX_ARGS)
            die("bad window table call");
        rh_tcall *c = &t_call[n - 1];
        c->sym = (int)v[1];
        c->nobj = v[2];
        c->nargs = v[3];
        c->obj_start = io;
        c->arg_start = ia;
        for (size_t j = 0; j < c->nobj; j++) {
            unsigned long o[5];
            if (!read_word(f, "obj") || !read_nums(f, o, 5) || o[0] != j || o[2] == 0
                || o[1] > RH_MAX_BYTES || o[3] > o[4] || o[4] * o[2] > o[1] + o[2] - 1)
                die("bad window table object");
            t_obj[io].size = o[1];
            t_obj[io].elem = o[2];
            t_obj[io].lo = o[3];
            t_obj[io].hi = o[4];
            io++;
        }
        for (size_t k = 0; k < c->nargs; k++) {
            unsigned long a[4];
            if (!read_word(f, "arg") || !read_nums(f, a, 4) || a[1] > K_OBJ
                || (a[1] == K_OBJ && (a[2] >= c->nobj || a[3] > t_obj[c->obj_start + a[2]].size)))
                die("bad window table argument");
            t_arg[ia].param = (int)a[0];
            t_arg[ia].kind = (int)a[1];
            t_arg[ia].obj = (int)a[2];
            t_arg[ia].off = a[3];
            ia++;
        }
    }
    if (!read_word(f, "end")) die("bad window table end");
    fclose(f);
}

/* ---- lifecycle ---- */

static void finish(void) {
    if (depth) {
        if (tight()) { /* the candidate's doing: evidence, never a harness fault */
            long v[1] = {(long)call_n};
            put(2, "\nRH-EXITED\n");
            rec("exited", v, 1);
            _exit(97);
        }
        die("the run ended inside a unit call");
    }
    if (tight()) {
        if (call_n != t_calls) {
            char what[32];
            size_t n = 0;
            fmt_str(what, &n, sizeof what, "call ");
            fmt_num(what, &n, sizeof what, (long)call_n + 1);
            red("RH-DIVERGED", "diverged", what);
        }
        integrity();
    }
    if (out_fd >= 0) {
        put(out_fd, "end\n");
        close(out_fd);
        out_fd = -1;
    }
}

static void init(void) {
    if (mode >= 0) return;
    pg = (size_t)getpagesize();
    const char *m = getenv("RUHARNESS_GUARD");
    int chosen = -1;
    for (int i = 0; i < 5; i++)
        if (m && strcmp(m, MODE_NAME[i]) == 0) chosen = i;
    if (chosen < 0) die("RUHARNESS_GUARD is not a known mode");
    mode = chosen;
#ifndef RUHARNESS_MEASURE
    if (mode == M_MEASURE) die("measure mode needs the measure build");
#endif
    const char *t = getenv("TMPDIR");
    if (!t) die("TMPDIR unset");
    char path[1024];
    size_t n = 0;
    fmt_str(path, &n, sizeof path, t);
    fmt_str(path, &n, sizeof path, "/ruharness-guard.out");
    if (n + 1 >= sizeof path) die("TMPDIR too long");
    out_fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
    if (out_fd < 0) die("cannot create the out file");
    put(out_fd, "ruharness-guard 1 ");
    put(out_fd, MODE_NAME[mode]);
    put(out_fd, "\n");
#ifdef __APPLE__
    stack_hi = (uintptr_t)pthread_get_stackaddr_np(pthread_self());
#endif
    if (tight()) {
        R = calloc(RH_MAX_RES, sizeof *R);
        if (!R) die("out of memory");
        load_table();
        static char altstack[65536];
        stack_t ss;
        memset(&ss, 0, sizeof ss);
        ss.ss_sp = altstack;
        ss.ss_size = sizeof altstack;
        if (sigaltstack(&ss, NULL) != 0) die("sigaltstack failed");
        memset(&our_sa, 0, sizeof our_sa);
        our_sa.sa_sigaction = on_fault;
        our_sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
        sigemptyset(&our_sa.sa_mask);
        if (sigaction(SIGSEGV, &our_sa, NULL) != 0 || sigaction(SIGBUS, &our_sa, NULL) != 0)
            die("sigaction failed");
        void *c = mmap(NULL, pg, PROT_NONE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (c == MAP_FAILED) die("mmap failed");
        canary_page = (uintptr_t)c;
#ifdef __APPLE__
        snapshot_ports(&task_ports0, 0);
        snapshot_ports(&thread_ports0, 1);
#endif
        integrity();
    }
#ifdef RUHARNESS_MEASURE
    if (mode == M_MEASURE) {
        ruharness_probe(&probe_cell);
        put(out_fd, probe_hits ? "probe 1\n" : "probe 0\n");
    }
#endif
    if (atexit(finish) != 0) die("atexit failed");
}

/* ---- tight/learn: shadows ---- */

static void top_info(uintptr_t addr, unsigned *share, unsigned *refs);

static void make_shadow(int j) {
    rh_obj *o = &O[j];
    size_t size = o->size, lo = o->lo * o->elem, hi = o->hi * o->elem;
    if (hi > size) hi = size;
    if (lo > hi) lo = hi;
    size_t gap = rup(size < RH_GAP_MIN ? RH_GAP_MIN : size);
    size_t below = head_layout() ? lo : hi, above = size - below;
    size_t total = gap + rup(below) + rup(above) + gap;
    if (n_res >= RH_MAX_RES) die("more than 65536 guarded objects in one run");
    void *m = mmap(NULL, total, PROT_NONE, MAP_PRIVATE | MAP_ANON, -1, 0);
    if (m == MAP_FAILED) die("mmap failed");
    o->res = (uintptr_t)m;
    o->res_end = o->res + total;
    o->shadow = o->res + gap + rup(below) - below;
    o->snap = malloc(size ? size : 1);
    if (!o->snap) die("out of memory");
    protect(pfloor(o->shadow), pceil(o->shadow + size), PROT_READ | PROT_WRITE);
    memcpy((void *)o->shadow, (const void *)o->orig, size);
    memcpy(o->snap, (const void *)o->shadow, size);
    protect(pfloor(o->shadow), pceil(o->shadow + size), PROT_NONE);
    if (hi > lo) protect(pfloor(o->shadow + lo), pceil(o->shadow + hi), PROT_READ | PROT_WRITE);
    rh_res *r = &R[n_res++];
    r->res = o->res;
    r->res_end = o->res_end;
    r->shadow = o->shadow;
    r->size = size;
    r->call = call_n;
    r->obj = j;
    r->open_lo = hi > lo ? pfloor(o->shadow + lo) : 0;
    r->open_hi = hi > lo ? pceil(o->shadow + hi) : 0;
    top_info(r->res, &r->share_mode, &r->ref_count);
}

/* Mach top-level info of the region at `addr` (share mode, reference count):
 * an alias created by vm_remap changes both. Zero where unavailable. */
static void top_info(uintptr_t addr, unsigned *share, unsigned *refs) {
    *share = 0;
    *refs = 0;
#ifdef __APPLE__
    mach_vm_address_t a = addr;
    mach_vm_size_t sz = 0;
    vm_region_top_info_data_t info;
    mach_msg_type_number_t cnt = VM_REGION_TOP_INFO_COUNT;
    mach_port_t name;
    if (mach_vm_region(mach_task_self(), &a, &sz, VM_REGION_TOP_INFO, (vm_region_info_t)&info, &cnt, &name)
        == KERN_SUCCESS) {
        *share = (unsigned)info.share_mode;
        *refs = (unsigned)info.ref_count;
    }
#endif
}

/* §B.R-1 (RT-1): the reservations themselves. Every page outside the open
 * window must still be PROT_NONE, and the region must still be the private
 * anonymous mapping it was created as (an alias made with vm_remap shows up
 * as a changed share mode or reference count). Tight modes only: learn mode
 * opens whole objects on purpose. */
static void verify_reservations(void) {
#ifdef __APPLE__
    if (learning()) return;
    for (int j = 0; j < n_obj; j++) {
        const rh_obj *o = &O[j];
        if (!o->res) continue;
        const rh_res *r = NULL;
        for (int i = n_res - 1; i >= 0; i--)
            if (R[i].res == o->res) { r = &R[i]; break; }
        if (!r) continue;
        unsigned share, refs;
        top_info(r->res, &share, &refs);
        if (share != r->share_mode || refs != r->ref_count) red("RH-TAMPER", "tamper", "protection");
        mach_vm_address_t a = r->res;
        while (a < r->res_end) {
            mach_vm_size_t sz = 0;
            vm_region_basic_info_data_64_t info;
            mach_msg_type_number_t cnt = VM_REGION_BASIC_INFO_COUNT_64;
            mach_port_t name;
            mach_vm_address_t q = a;
            if (mach_vm_region(mach_task_self(), &q, &sz, VM_REGION_BASIC_INFO_64, (vm_region_info_t)&info, &cnt, &name)
                != KERN_SUCCESS || q >= r->res_end)
                break;
            uintptr_t s = q > a ? (uintptr_t)q : (uintptr_t)a, e = (uintptr_t)(q + sz);
            if (e > r->res_end) e = r->res_end;
            /* the closed part of this region: anything outside the open window */
            int closed_bytes = s < r->open_lo || e > r->open_hi || r->open_hi == 0;
            if (closed_bytes && info.protection != VM_PROT_NONE) red("RH-TAMPER", "tamper", "protection");
            a = q + sz;
        }
    }
#endif
}

/* `v` translated out of any shadow of this call, or unchanged. */
static uintptr_t relocate_word(uintptr_t v) {
    for (int k = 0; k < n_obj; k++) {
        const rh_obj *q = &O[k];
        if (q->res && v >= q->shadow && v <= q->shadow + q->size) return q->orig + (v - q->shadow);
    }
    return v;
}

static void close_call(void) {
    for (int j = 0; j < n_obj; j++) {
        rh_obj *o = &O[j];
        if (!o->res) continue;
        protect(pfloor(o->shadow), pceil(o->shadow + o->size), PROT_READ | PROT_WRITE);
    }
    /* relocation: pointers the C stored into an object that point into a
     * shadow — at every 8-byte offset from the OBJECT start, whatever the
     * shadow's alignment (byte-element objects are rarely 8-aligned), and
     * only where the call changed the bytes (a datum the C left alone is
     * never a pointer it stored). */
    for (int j = 0; j < n_obj; j++) {
        rh_obj *o = &O[j];
        if (!o->res) continue;
        for (size_t i = 0; i + sizeof(uintptr_t) <= o->size; i += sizeof(uintptr_t)) {
            unsigned char *at = (unsigned char *)o->shadow + i;
            if (memcmp(at, o->snap + i, sizeof(uintptr_t)) == 0) continue;
            uintptr_t w;
            memcpy(&w, at, sizeof w);
            uintptr_t w2 = relocate_word(w);
            if (w2 != w) memcpy(at, &w2, sizeof w2);
        }
    }
    /* copy-back: only bytes the call changed (writes through an unshadowed
     * path to the original, and const objects, stay as they are) */
    for (int j = 0; j < n_obj; j++) {
        rh_obj *o = &O[j];
        if (!o->res) continue;
        const unsigned char *s = (const unsigned char *)o->shadow;
        unsigned char *d = (unsigned char *)o->orig;
        for (size_t i = 0; i < o->size; i++)
            if (s[i] != o->snap[i]) d[i] = s[i];
        free(o->snap);
        o->snap = NULL;
        protect(o->res, o->res_end, PROT_NONE); /* stays mapped, never reused */
    }
}

/* ---- the API ---- */

void ruharness_enter(int sym, void *frame) {
    init();
    if (depth) die("a unit call inside a unit call");
    if (tight()) {
        signals_before = signals_now();
        our_deliveries = 0;
    }
    depth = 1;
    call_n++;
    frame_lo = (uintptr_t)frame;
    n_obj = 0;
    args_seen = 0;
    if (mode == M_MEASURE) {
        long v[2] = {(long)call_n, sym};
        rec("call", v, 2);
        return;
    }
    if (call_n > t_calls || t_call[call_n - 1].sym != sym) {
        char what[32];
        size_t n = 0;
        fmt_str(what, &n, sizeof what, "call ");
        fmt_num(what, &n, sizeof what, (long)call_n);
        red("RH-DIVERGED", "diverged", what);
    }
    const rh_tcall *c = &t_call[call_n - 1];
    n_obj = (int)c->nobj;
    for (int j = 0; j < n_obj; j++) {
        const rh_tobj *t = &t_obj[c->obj_start + j];
        rh_obj *o = &O[j];
        memset(o, 0, sizeof *o);
        o->size = t->size;
        o->elem = t->elem;
        o->lo = t->lo;
        o->hi = t->hi;
    }
}

void *ruharness_arg(int param, const void *p, size_t elem) {
    if (!depth) die("an argument outside a unit call");
    if (elem == 0) elem = 1;
    if (mode == M_MEASURE) {
        char l[96];
        size_t n = 0;
        fmt_str(l, &n, sizeof l, "arg ");
        fmt_num(l, &n, sizeof l, (long)call_n);
        fmt_str(l, &n, sizeof l, " ");
        fmt_num(l, &n, sizeof l, param);
        fmt_str(l, &n, sizeof l, " ");
        int kind = K_PASS;
        int j = -1;
        size_t off = 0;
        if (!p) kind = K_NULL;
#ifdef RUHARNESS_MEASURE
        else {
            char name[64];
            void *ra = NULL;
            size_t rs = 0;
            const char *k = __asan_locate_address((void *)p, name, sizeof name, &ra, &rs);
            uintptr_t a = (uintptr_t)p, base = (uintptr_t)ra;
            if (k && (strcmp(k, "stack") == 0 || strcmp(k, "heap") == 0 || strcmp(k, "global") == 0)
                && rs > 0 && rs <= RH_MAX_BYTES && a >= base && a - base <= rs) {
                for (int i = 0; i < n_obj; i++)
                    if (O[i].orig == base) j = i;
                if (j < 0) {
                    if (n_obj >= RH_MAX_OBJS) die("more than 16 distinct objects in one call");
                    j = n_obj++;
                    rh_obj *o = &O[j];
                    memset(o, 0, sizeof *o);
                    o->orig = base;
                    o->size = rs;
                    o->elem = elem;
                    o->mlo = (long)1 << 62;
                    o->mhi = -((long)1 << 62);
                }
                kind = K_OBJ;
                off = a - base;
            }
        }
#endif
        if (kind == K_NULL) fmt_str(l, &n, sizeof l, "null");
        else if (kind == K_PASS) fmt_str(l, &n, sizeof l, "pass");
        else {
            fmt_str(l, &n, sizeof l, "obj:");
            fmt_num(l, &n, sizeof l, j);
            fmt_str(l, &n, sizeof l, ":");
            fmt_num(l, &n, sizeof l, (long)off);
        }
        fmt_str(l, &n, sizeof l, "\n");
        put(out_fd, l);
        return (void *)p;
    }
    const rh_tcall *c = &t_call[call_n - 1];
    char what[48];
    size_t wn = 0;
    fmt_str(what, &wn, sizeof what, "arg ");
    fmt_num(what, &wn, sizeof what, (long)call_n);
    fmt_str(what, &wn, sizeof what, ":");
    fmt_num(what, &wn, sizeof what, param);
    if (args_seen >= c->nargs) red("RH-DIVERGED", "diverged", what);
    const rh_targ *t = &t_arg[c->arg_start + args_seen++];
    if (t->param != param || (t->kind == K_NULL) != (p == NULL)) red("RH-DIVERGED", "diverged", what);
    if (t->kind != K_OBJ) return (void *)p;
    rh_obj *o = &O[t->obj];
    if (!o->res) {
        o->orig = (uintptr_t)p - t->off;
        make_shadow(t->obj);
    } else if (o->orig != (uintptr_t)p - t->off) {
        red("RH-DIVERGED", "diverged", what); /* two arguments disagree about the object */
    }
    return (void *)(o->shadow + t->off);
}

void *ruharness_ret(void *p) {
    if (mode == M_MEASURE || !depth || !p) return p;
    return (void *)relocate_word((uintptr_t)p);
}

void ruharness_exit(void) {
    if (!depth) die("unbalanced ruharness_exit");
    if (mode == M_MEASURE) {
        for (int j = 0; j < n_obj; j++) {
            const rh_obj *o = &O[j];
            int t = o->mhi > o->mlo;
            long v[6] = {(long)call_n, j, (long)o->size, (long)o->elem, t ? o->mlo : 0, t ? o->mhi : 0};
            rec("obj", v, 6);
        }
    } else {
        account_signals();
        integrity();
        verify_reservations();
        close_call();
    }
    n_obj = 0;
    depth = 0;
}

/* ---- sanitizer-coverage callbacks: the instrumented unit C and the probe,
 * measure build only (defined always; referenced only there) ---- */

static void track(uintptr_t a, long n) {
    if (!depth) {
        if (a == (uintptr_t)&probe_cell) probe_hits++;
        return;
    }
    for (int j = 0; j < n_obj; j++) {
        rh_obj *o = &O[j];
        if (a >= o->orig && a < o->orig + o->size) {
            long off = (long)(a - o->orig);
            if (off < o->mlo) o->mlo = off;
            if (off + n > o->mhi) o->mhi = off + n;
            return;
        }
    }
    if (!foreign_reported && stack_hi && a >= frame_lo && a < stack_hi) {
        foreign_reported = 1;
        long v[1] = {(long)call_n};
        rec("foreign", v, 1);
    }
}

#define RH_CALLBACKS(sz)                                                          \
    void __sanitizer_cov_load##sz(void *a) { track((uintptr_t)a, sz); }           \
    void __sanitizer_cov_store##sz(void *a) { track((uintptr_t)a, sz); }
RH_CALLBACKS(1)
RH_CALLBACKS(2)
RH_CALLBACKS(4)
RH_CALLBACKS(8)
RH_CALLBACKS(16)
