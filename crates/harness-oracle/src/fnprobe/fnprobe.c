/* RuHarness features probe runtime (docs/FEATURES-PROBE-REDESIGN.md §3.6).
   Harness-owned, built with -fno-builtin and without the target's include
   folders. A watched function's note is `__ruharness_seen[N] = 1;`. Until
   setup has run, notes land in a static array; setup maps the notes file the
   harness created in the run's temp dir ($TMPDIR/.ruharness-notes, N + 1 zero
   bytes), merges what the array holds, and sets the last byte (the attach
   byte) to 1. The harness reads the file after the run: the attach byte 0
   means setup did not run — never a record where nothing ran. No call at note
   time, so nothing the program defines or does to its descriptors can lose a
   note; the mapping's pages belong to the file, so a crash keeps them. Setup
   calls only open, fstat, mmap and close, and reads TMPDIR from environ. */
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef RUHARNESS_FNPROBE_N
#error "RUHARNESS_FNPROBE_N (the number of watched pairs) must be defined"
#endif

extern char **environ;

static unsigned char ruharness_early[RUHARNESS_FNPROBE_N + 1];
unsigned char *volatile __ruharness_seen = ruharness_early;

static const char ruharness_name[] = "/.ruharness-notes";

__attribute__((constructor)) static void ruharness_setup(void) {
    const char *dir = 0;
    for (char **e = environ; e && *e; e++) {
        const char *s = *e;
        if (s[0] == 'T' && s[1] == 'M' && s[2] == 'P' && s[3] == 'D' && s[4] == 'I' &&
            s[5] == 'R' && s[6] == '=') {
            dir = s + 7;
            break;
        }
    }
    if (!dir) {
        return;
    }
    char path[4096];
    unsigned long n = 0;
    while (dir[n] && n < sizeof path - sizeof ruharness_name) {
        path[n] = dir[n];
        n++;
    }
    if (dir[n]) {
        return;
    }
    for (unsigned long i = 0; i < sizeof ruharness_name; i++) {
        path[n + i] = ruharness_name[i];
    }
    int fd = open(path, O_RDWR | O_CLOEXEC | O_NOFOLLOW);
    if (fd < 0) {
        return;
    }
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size != RUHARNESS_FNPROBE_N + 1) {
        close(fd);
        return;
    }
    unsigned char *m = mmap(0, RUHARNESS_FNPROBE_N + 1, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (m == MAP_FAILED) {
        return;
    }
    __ruharness_seen = m;
    /* A merge: a second image of the program adds its notes to the first's. */
    for (unsigned long i = 0; i < RUHARNESS_FNPROBE_N; i++) {
        if (ruharness_early[i]) {
            m[i] = 1;
        }
    }
    m[RUHARNESS_FNPROBE_N] = 1;
}
