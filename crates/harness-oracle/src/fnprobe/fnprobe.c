/* RuHarness features probe runtime (docs/FEATURES-DESIGN.md §5.4).
   Harness-owned. Compiled into the probed copy only, with
   -DRUHARNESS_FNPROBE_N=<number of watched functions>. The first time a
   watched function runs, its id is appended to $TMPDIR/ruharness-fnprobe as
   4 little-endian bytes — written at once, so a crash, _exit or a timeout
   keeps what ran before. */
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

#ifndef RUHARNESS_FNPROBE_N
#error "RUHARNESS_FNPROBE_N must be defined"
#endif

#if RUHARNESS_FNPROBE_N > 0
unsigned char __ruharness_seen[RUHARNESS_FNPROBE_N];
#else
unsigned char __ruharness_seen[1];
#endif

static int __ruharness_fd = -2;
/* The notes file's path, from $TMPDIR at the first note: a reopen writes
   where the first open did, whatever the program did to its environment. */
static char __ruharness_path[4096];

static int __ruharness_open(void) {
  static const char name[] = "/ruharness-fnprobe";
  int fd, floor = 900;
  struct rlimit rl;
  if (!__ruharness_path[0]) {
    const char *dir = getenv("TMPDIR");
    size_t n;
    if (!dir) return -1;
    n = strlen(dir);
    if (n + sizeof name > sizeof __ruharness_path) return -1;
    memcpy(__ruharness_path, dir, n);
    memcpy(__ruharness_path + n, name, sizeof name);
  }
  fd = open(__ruharness_path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC | O_NOFOLLOW, 0600);
  if (fd < 0) return -1;
  /* Out of the program's way: high, below the soft limit (a launchd-started
     process may have 256); the original descriptor when that fails. */
  if (getrlimit(RLIMIT_NOFILE, &rl) == 0 && rl.rlim_cur != RLIM_INFINITY &&
      (long)rl.rlim_cur - 16 < (long)floor)
    floor = (int)rl.rlim_cur - 16;
  if (floor > fd) {
    int high = fcntl(fd, F_DUPFD_CLOEXEC, floor);
    if (high >= 0) {
      close(fd);
      fd = high;
    }
  }
  return fd;
}

void __ruharness_probe(unsigned id) {
  unsigned char b[4];
  int saved;
  if (id >= (unsigned)RUHARNESS_FNPROBE_N || __ruharness_seen[id]) return;
  __ruharness_seen[id] = 1;
  /* The program never sees the probe's calls in errno (review M3). */
  saved = errno;
  if (__ruharness_fd == -2) __ruharness_fd = __ruharness_open();
  if (__ruharness_fd >= 0) {
    b[0] = (unsigned char)(id & 0xffu);
    b[1] = (unsigned char)((id >> 8) & 0xffu);
    b[2] = (unsigned char)((id >> 16) & 0xffu);
    b[3] = (unsigned char)((id >> 24) & 0xffu);
    /* A program that closed its inherited descriptors closed this one too:
       reopen (appending) and write once more. */
    if (write(__ruharness_fd, b, 4) != 4) {
      __ruharness_fd = __ruharness_open();
      if (__ruharness_fd >= 0) (void)write(__ruharness_fd, b, 4);
    }
  }
  errno = saved;
}
