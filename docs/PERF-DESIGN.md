# C-vs-Rust performance baselines (design)

Status: **revision 4 — after the check of revision 3 (1b27d10)**, to be checked (2026-10-01).
History: the draft (0b64bba) had an adversarial review (4 lenses, 49 findings, 46 confirmed by two
verifiers); revision 1 (00f2e23) answered them; its check gave 118 changes (scratchpad
`perf/rev2-changes.md`, cited [c12]); revision 2 (cd1f79b) took them; its check gave 35 new
findings (`perf/rev2-digest.md`, cited [n12]); revision 3 (1b27d10) answered those; its check
(`wf_d21b13f6-ea1`, `perf/revision3-check.json`, digest `perf/rev3-digest.md`) found 74 of the 118
items done, 43 in part, 1 wrong, and 30 new findings confirmed by two verifiers (cited [m17]), 1 by
one, none refuted. This revision answers each. The §15 spike: DECISIONS.md "2026-09-30 — C-vs-Rust
performance baselines: §15 spike". The person's wish: "baseline the original C, then compare
migrated code side by side as the transition proceeds".

## 1. What it is, in plain words

`harness perf run` measures, on workloads the person names (the inputs and options their program is
really used with):
- **the original C alone** — its CPU time, instructions and memory on each workload: the baseline,
  from the first day, before any unit is accepted (§3.6);
- **the program as it stands** — the C with every accepted unit's Rust swapped in at once (when two
  or more units are measurable): the line that moves as the migration proceeds;
- **each accepted unit** — the C with only that unit's Rust: which unit to look at when the
  program got slower. Per-unit rows do not add up to the program as it stands (each carries Rust's
  fixed start-up once) [c1].

perf measures the **Rust in use** — the crate `verify` last passed for the unit (written by the
model and accepted, or written by hand and Re-checked). To measure a new attempt, it must first
become the crate in use (§3.11) [c91, m18].

Each comparison runs the C and the other side alternately. The answer is one sentence about time —
"about as fast as the C (within 2 %)", "slower by 6.2 % (4.1–8.3 %)", "about 2.3 % slower
(0.6–3.4 %) — too close to the 2 % line to call", "can't tell — 9 of the 30 runs ran mostly on the
slower cores: the computer was busy" — then the details: CPU time, instructions and memory.

Before any timing, the two sides must **end the same way and print the same thing** (each side's
folder written `$PROGDIR`, the run's temp dir `$TMPDIR`). perf compares what the program prints
and how it ends, not files it writes. When they differ, that is reported as what it is: the program
behaves differently with that Rust on a real input — a correctness finding `verify` cannot make (it
never runs this workload), not a speed result [c2, n34].

It is information, never a gate: `verify`, `migrate`, `promote` and `bench check` never run it or
read it; it writes no verdict and no plan status. No model is involved (Tier 0). A program that
starts other programs cannot be measured: perf's sandbox stops even a fork, where verify allows a
fork but not starting another program [c2, c114].

Words: a **side** is one program (the **C**, a **unit's program**, the **program as it stands**); a
**workload** is a command line plus at most one input file; a **run** is one execution of one side;
a **row** is one side measured on one workload — the C alone, or another side against the C.

## 2. Premise

**The spike** (DECISIONS 2026-09-30, a loaded machine) on zopfli's u001: instructions +3.24 %,
cycles +0.79 %, wall +3.8 %, the footprint equal (12.4 MB) on a 200 KB input; the oracle's 30 KB
samples are too small to time.

**Checked by the review and the checks** (scratchpad `perf/`, `design-review/`, `design-check/`,
`design-check2/`, `design-check3/`; other agents loaded the machine throughout; the load average is
given with each figure) [c3–c7]:
- **Counting the program alone under the sandbox** (`perf/premise3/`, load 8–14): a harness-owned
  launcher outside the sandbox forks; the child execs `sandbox-exec` with a profile that allows exec
  of exactly a harness-owned **trampoline** and the program, and denies fork; the trampoline signals
  "ready" on a pipe and waits; the launcher reads `proc_pid_rusage(RUSAGE_INFO_V6)` (the
  **baseline**: sandbox-exec's set-up and the trampoline's start, 7.4–7.8e7 instructions, 12–16 ms
  of CPU at load ≈ 10) and says "go"; the trampoline execs the program **with argv[0] = the
  program's bare name**. Subtracted at the end: an empty program 1.04–1.24e7 instructions, a
  1e8-iteration loop 6.13–6.16e8 with 0, 50 or 200 ms of delay before the baseline read — the read
  is exact. Identical binaries: kernel work under preemption moves instruction counts by 0.2 % at
  load 14, 1–1.5 % at load 37–52, up to 6.5 % on a low-IPC loop. (`sandbox-exec` alone cannot set
  argv[0]: it passes its operand.)
- **Revision 3's launcher mechanisms, run** (`design-check3/`, load 4.5–11.4): the child's
  `setsid()` makes the program's own `setsid`/`setpgid` fail (EPERM); perfrun killing the child's
  group survives to write a timeout record; SIGTERM and the reader's close each reach the program
  within 5 ms; a 4-thread program is not judged crashed; `pti_csw` at *ready* and `ru_nivcsw` at the
  end are one running total across exec (an empty program 5–12 switches, 200 sleeps 206–219).
- **Time under load**: cycles mix performance- and efficiency-core time; normalising to the
  performance-core cost per instruction cuts the per-run spread from 6–8 % to 2.3–3.3 % (load
  13–53), but the cost per instruction rises on runs mostly off the performance cores (runs with
  under half their cycles there read 2–16 % high). On normalised rows the interval rule never said
  slower or faster for identical programs (48 live trials; 82 recorded A-vs-A rows); on rows that
  fell to raw cycles it did (identical zopfli binaries read "slower by 9.9 %") [m3]. **Answer
  rates of §3.8's rule**, re-derived on the 82 recorded rows (load 13–53): at 15 runs 32 of 164
  windows answer (all "about as fast"), 81 fall to raw cycles (busy), none a false difference; at 31
  runs 4 of 8 rows answer, 3 busy, 1 "no clear difference". Live at load ≈ 10, about 4 rows in 10
  answer at 15 runs. So on a busy Mac most rows ask for 31 runs, at §6's cost; on a quiet machine
  (spread ≤ 0.75 %) seven runs answer about 98 % of rows [m11, n12].
- **Memory** (load 13–53): the lifetime maximum footprint clusters (≈ 12.7 / 14.7 MB on zopfli) and
  its minimum is not stable under load (identical binaries read different in 12 of 20 rows); an
  interval with a margin of max(5 %, 1 MiB) gave no false difference in 70+ rows. The footprint
  starts over at exec (the end maximum read 950 584 bytes, below the baseline read's 966 968; a
  program that touched 64 MB then exec'd itself read 966 968): it is recorded as is, never minus
  the baseline [c5].
- **Rust's fixed start-up** (a unit linked, never called; load 4.5–6 and 24–27): a std-using unit
  +1.04e7 instructions (zopfli's u001 and a heavy-std test unit alike), +2.8e6 cycles, +3 pages
  (48 KiB); a unit using no std adds nothing [c4, c31].
- **Linking** (macOS, checked): two Rust staticlibs from one rustc link into one C program (ld64
  rescans archives; one rustc gives them one std); with different panic strategies the first
  archive's runtime silently wins; a unit whose crate sets `lto = "fat"` links alone but not beside
  another std-using unit (duplicate std symbols, either order). **Unchecked here** (no Linux
  machine): GNU ld scans each archive once, so plan order alone can fail and `--start-group …
  --end-group` is needed; two rustc versions collide [c6, c22, c111].
- **First execs** (macOS): a newly written binary's first exec waits on the system's check — 0.25 s
  at load 4–6, 2.6–2.9 s at load 6–7, 7–14 s at load 9–11, 23–27 s for a run that starts three new
  ones. The wait is off-CPU (instructions and CPU time unchanged) and comes **after** the exec
  starts, so it falls after *go* [c7, n4].
- **The launcher's build** through `/usr/bin/cc` put its object in `/var/folders`; a same-user
  process swapped it after a passed hash in 23 of 30 tries. `/usr/bin/cc` is an `xcrun` shim that
  finds the real compiler through a per-user cache under `/var/folders` (not `TMPDIR`), which every
  tool profile may write [m1]. The launcher's build is a trust boundary.
- **Process control**: perfrun's forked child stays in perfrun's process group unless moved
  [n3]; `RLIMIT_CPU` on macOS only sends SIGXCPU [n2]; a failed exec after "go" exits 127 with no
  message [n1]; kqueue `EVFILT_WRITE` with `EV_CLEAR` reports the reader's close as `EV_EOF` [n11];
  `rusage_info_v6` has no switch counts, and wait4's `ru_nvcsw` reads 0 [n8]; Rust's `Command`
  keeps its copy of a pipe's write end until dropped (no end-of-file while it lives) [c40].

## 3. The design

### 3.1 Workloads: `migration/perf/workloads.toml` (`ruharness-perf-workloads`, v1)

```toml
schema_version = 1

[[workload]]
id = "big-text"            # [a-z0-9-], ≤ 24, unique
args = ["-c", "{input}"]   # ≤ 8 strings, each ≤ 256 bytes; {input} once, as a whole argument
input = "bench/big.txt"    # optional: a regular file in the target, ≤ 64 MiB
runs = 15                  # optional: runs a side, 5 to 31 (default 15)
```

- At most 16 workloads; the file at most 64 KiB, read with `read_regular`; unknown keys refused —
  every refusal of the file names the line and column (its own span for a rule beyond the syntax,
  through `toml`'s spans, no new dependency) and the workload [c8, c9, c104].
- `{input}` is a whole argument (`"{input}"`, never `"--file={input}"`), exactly once in `args`
  when `input` is set, and refused without `input` [c9].
- `input` as written: relative, UTF-8, no control character, no component starting with `.` or `-`,
  not under `migration/`. Resolved: its canonical path lies in the canonical target root, outside
  the canonical ledger folder and any `.git`; a regular file, at most 64 MiB, read **once** per
  `harness perf run`; every run's copy and the digest come from those bytes; a change to the file
  after that read changes nothing in the run [c11, c12].
- The workload's digest: blake3 over id, args, the input's name and bytes — **not `runs`** (a row
  records the n it used) [c10].
- An input perf cannot use affects only its workload: outcome **input-unusable**, with the reason's
  own words, the same in the CLI and the cockpit [c13]: "bench/big.txt is not here — put the file
  back or remove the workload"; "… is a link — copy the file in instead"; "… leads outside the
  project through a linked folder — copy the file in"; "… is a folder (or a pipe, or a device) —
  name a file"; "… is over 64 MiB — use a smaller input"; "… is under migration/ — move it". Never
  stored over an earlier row (§3.7).
- **Written** with `harness perf init` (a starter) or the cockpit's **Write / Edit your workloads
  file**: the features Edit flow generalised — its own draft slot (`harness-tui-workloads`),
  "Continue my workloads draft", `harness perf save --expect … --bytes …` with the 64 KiB cap.
  Saving checks the text only (syntax, keys, limits, the name rules); the input's existence and
  size are checked when perf reads it [c12, c16].
- **States**, each with its words in the CLI and the cockpit [c16, c82]: no file ("write your
  workloads file first — harness perf init gives a starter"); a file with no workload, which is
  what the starter is ("add a [[workload]] to migration/perf/workloads.toml"); a file with an error
  ("migration/perf/workloads.toml line 7, column 3: … — fix it, or Edit the workloads file"). Each
  is exit 1 for `perf run`; in the cockpit Measure is greyed with the same words.
- The starter teaches [c15, n34]:
  ```toml
  schema_version = 1
  # Workloads: runs of the whole program long enough to time — half a second or more of the C.
  # Use the options and an input your program is really used with. Put the input file in the
  # project (a real file, not a link; at most 64 MiB; not under migration/ or .git; no part of
  # its path starting with "." or "-") and commit it, or a fresh clone cannot measure.
  # perf compares what the program prints and how it ends, not files it writes.
  # A program that starts other programs cannot be measured.
  #
  # [[workload]]
  # id = "big-text"
  # args = ["{input}"]          # your program's own options; "{input}" once, as its own argument
  # input = "bench/big.txt"
  # runs = 15                   # 5 to 31
  ```

### 3.2 What is measured, and how it is built

**Units** [c17, c18, n26, m30]: status `verified` or `merged` **and**
`status::unit_report(…).fresh_green()`, read under perf's own writer lock, **and** no half-done
Accept: `status::promotion_marker` (made public) is called directly after the lock is taken —
`unit_report` skips the marker while a live holder holds the lock, and under perf's lock that holder
is perf — and a legacy `.<crate>.prev` is treated the same. Its words match the cockpit's: "an
Accept of a-1234 was interrupted — Re-check u001 (or run harness verify u001) to finish or undo it;
Measure does not". The plan's `replaces` must equal the verdict's `inputs.replaces` ("its replaced
files changed since verify — Re-check it"). After perf builds a unit's crate, its
`unit_crate_file_set_hash` must equal the verdict's `inputs.rust_crate` ("its Rust changed since
verify — Re-check it"). Every stale reason of `unit_report` has its words ("its test driver changed
since verify", "its C changed since verify", "verify it first"). A pending or blocked unit is never
measured, fresh or not.

**Sides** [c19–c22, n27, m19, m22]:
- the **C**: every top-level `.c` of `source_dir` (a non-regular one refused by name), linked with
  `extra_link_args`;
- **a unit's program**: less the unit's `replaces`, plus its staticlib. A shared helper checks the
  `replaces` entries (each matches a collected top-level `.c`; an empty list refused): a mismatch,
  a crate that does not build, or a program that does not link is that unit's outcome
  (`replaces-mismatch`, `crate-does-not-build`, `does-not-link`, with the tool's first lines), and
  the next unit is measured;
- **the program as it stands**: with two or more measurable units — every one's staticlib, in plan
  order (`--start-group … --end-group` on GNU ld, unchecked) — and only when every unit's staticlib
  carries the same panic runtime. perf reads it from the archive itself (its member names:
  `panic_abort-…` or `panic_unwind-…`, a small `ar` reader beside `objsyms`), never from the
  manifest (a `.cargo/config.toml` or another spelling can set it); a crate with no setting
  unwinds. Mixed → outcome **mixed-panic**: "u001 (written by hand) unwinds; u002 (made by the
  harness) aborts — the program as it stands would silently use one. Add `[profile.release] panic =
  "abort"` to u001's Cargo.toml, then run `harness verify u001`" (an edit the cockpit cannot make:
  the words say the terminal command). Each unit links alone but the program as it stands does not
  (a crate with `lto` set, two std copies) → **does-not-link** on the as-it-stands heading with the
  linker's first lines and "a crate built with lto cannot be linked beside another Rust unit — the
  units it holds: u001, u002" [c22].
- The as-it-stands row records the units it holds (`units: [{id, crate}]`, plan order) and those
  left out (`left_out: [{id, crate, reason}]`, the reason from the closed set: not-verified,
  replaces-mismatch, crate-does-not-build, does-not-link), as inputs of the row. Currency compares
  today's fresh-green set with held plus left out: "u-hash was accepted since", "u-tree is left out
  now", "the plan's order changed" [c21, m22]. The CLI names the left-out units on every run: "the
  program as it stands — u001, u002 (u-tree left out: its crate does not build; u-old: verify it
  first)". Fewer than two measurable units: no as-it-stands row; the CLI says "one unit measured
  (u001) — u-tree left out: verify it first" or "no accepted unit to compare yet".

**Build** — "as verify builds them: C -O2 -ffp-contract=off, Rust release as each crate's own
profile sets it (verify builds the same)", said in every result; no dead-strip (verify's programs
are not dead-stripped, and the results must be its programs); Rust's fixed start-up is part of
what is measured (§6) [c31, c33]:
1. **The launcher and trampoline** (§3.3) are built from their embedded sources into a private
   cache outside the target and every temporary folder: `~/Library/Caches/ruharness/perf/` on
   macOS, `$XDG_CACHE_HOME/ruharness/perf/` (else `~/.cache/…`) on Linux; a 0700 folder named
   `<PERF_LAUNCHER>-<blake3 of the sources and the compiler's version>`, made by a helper that
   refuses links; an `flock` on `…/perf/.lock` while one harness builds or checks it (the cache is
   shared by every target and harness version on the computer; a stale version folder is removed
   when a new one is built) [c24]. **The compiler is found without `xcrun`'s per-user cache** [m1]:
   the developer folder is the target of the root-owned link `/var/db/xcode_select_link` (else
   `/Library/Developer/CommandLineTools`), the compiler `<that>/usr/bin/clang` (Command Line Tools)
   or `<that>/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang` (Xcode), refused unless the file
   and its folders are root-owned and not writable by others; it is run with `-isysroot` set to the
   SDK under the same folder, and links with the `ld` beside it — exactly how is settled with a test
   in step (a): a process under a crate build's tool profile that rewrites `xcrun`'s cache does not
   change the compiler the launcher is built with. Linux: `/usr/bin/cc` (unchecked). `cc -c` then a
   link, `TMPDIR` set to a fresh 0700 folder in the cache. No tool profile and no run lists the
   cache as writable; perf refuses when the home folder or `TMPDIR` lies under `/tmp` or
   `/var/folders` ("your home folder is inside a temporary folder every build tool may write — perf
   cannot keep its launcher safe there") [c24]. Kept between runs (no new first exec each run
   [n4]); both binaries' blake3 recorded at the build and checked before each run. The hash guards
   against a change after the build; the build is trusted because its compiler is found through
   root-owned paths and it runs before any target code is built in this `perf run` [c25].
2. `whole_cc_into(base, link_args, runner, out, inputs)`, shared with `verify`; the C objects
   compiled once into `.perf/obj/`, each side one link into its slot; every object, staticlib and
   binary hashed after its build and checked before the first run, and every link finished before
   the first run [c26]. A C that does not build is a set-up failure with the compiler's first
   lines (and "the program needs one main()" when the link says so) [c27].
3. each step's write set is its own folder (compiles `.perf/obj`, each link its slot, cargo the
   crate's `target/` and `Cargo.lock`), and `.perf/` is made by a `scratch_dir`-style helper that
   refuses links [c23]. Each tool's process group is killed when the tool ends (as runs' are).

**Slots** [c28, n33]: `.perf/bin/<slot>/<name>`, `<name>` the target's program name
(`features::program_name`), `<slot>` four characters: `p000` the C, `p001`…`p999` the unit at plan
position 1…999 (1-based), `pall` the program as it stands (a plan of more than 999 units is refused
by name). Every side runs by its full path from a slot of the same width, with **argv[0] =
`<name>`** (the trampoline sets it), in the run's own working folder `<tmp>/run` (as scenarios;
a relative output-file argument lands there) [n34]. What still differs per side: the executable's
own path (`_NSGetExecutablePath`) [c29, c30].

Not measured: the unit's differential driver (mostly `printf` and start-up) — a revisit item. A
per-side do-nothing program measuring Rust's start-up per unit [c32] is not built: it costs a link
and 2n runs a side to measure a share the instruction margin (§3.8) covers at the floor; the
start-up is named on the rows it can explain (§3.8).

### 3.3 The launcher (`perfrun`) and the trampoline (`perfgo`) — harness-owned C

`crates/harness-oracle/src/perf/{perfrun,perfgo}.c`, embedded with `include_str!`; a version
constant `PERF_LAUNCHER` in `harness-core`, pinned to the sources by a test. The launcher is the one
harness-built binary that runs unsandboxed; the program runs only inside the sandbox. On a platform
other than macOS and Linux, `perf run` refuses by name [c44]. Linux mechanisms below are from
documentation and **unchecked here** [c111].

`perfrun MODE PROFILE PERFGO DEADLINE PROGRAM NAME ARGS…` (MODE `run` or `facts`):
1. **Its descriptors** [c40]: stdin (fd 0) is the write end of the **record pipe** the harness made
   (`std::io::pipe`, `Stdio::from`); stdout (fd 1) is the read end of the **ack pipe**, written by
   the harness; stderr `/dev/null`. The harness drops perfrun's `Command` (and with it its copies of
   the pipe ends) right after the spawn, so end-of-file comes when perfrun dies. On Linux perfrun
   first sets `prctl(PR_SET_DUMPABLE, 0)` on itself, so a same-user program cannot open
   `/proc/<perfrun>/fd/0` [c44, n6].
2. Three more pipes (*ready*, *go*, *exec-status*); `fork`. The child: `setsid()` — it leads a new
   session and group whose id is its own pid, and as a session leader the program cannot leave it
   (`setsid` and `setpgid` fail) [n3]; stdin `/dev/null`; stdout and stderr the run's capture pipes
   (step 1 runs) or `/dev/null` (timed runs); every other descriptor closed but the three pipes
   (bounded: `close_range`/`PROC_PIDLISTFDS`); then `execv("/usr/bin/sandbox-exec", ["-p", PROFILE,
   PERFGO, ready, go, status, PROGRAM, NAME, ARGS…])`. On Linux (no sandbox) it execs PERFGO
   directly, after `PR_SET_PDEATHSIG(SIGKILL)` and a `getppid()` check against perfrun's pid.
   Right after the fork perfrun writes `child <pid>` on the record pipe (one write, under
   `PIPE_BUF`) [n3].
3. **PERFGO** (inside the sandbox): marks *status* close-on-exec, writes one byte on *ready* (if
   that fails it exits 125), reads one on *go* — **end-of-file instead of a byte means perfrun is
   gone: it exits 125 and the program never runs** — closes both, and `execv(PROGRAM, [NAME,
   ARGS…])`; if the exec fails it writes `errno` on *status* and exits 127. No CPU-time limit: on
   macOS it only sends SIGXCPU, which a program may ignore, and it would kill a legitimate threaded
   program as a crash [n2].
4. On *ready*: macOS `proc_pid_rusage(child, RUSAGE_INFO_V6)` (else V4) — the baseline:
   instructions, cycles, performance-core instructions and cycles, user and system time (mach ticks
   → `mach_timebase_info`) — and `proc_pidinfo(PROC_PIDTASKINFO)`'s context switches (`pti_csw`);
   Linux: the counters were opened on the child before it was released (`perf_event_open`,
   `instructions:u`, `cycles:u`, `inherit = 1` — a child's counts arrive only when it exits; on a
   hybrid CPU one event per core type, summed, with `TOTAL_TIME_RUNNING` summed over them against
   `TOTAL_TIME_ENABLED`: below it → no time words, never scaled; all zero → no counter) [c34–c36,
   c45, c46]. Then perfrun waits for the **ack**: one byte from the harness, which has registered
   the child's group (end-of-file: the harness is gone — perfrun kills the child's group, reaps, and
   exits). Then *go*; then *status* is read: end-of-file means the exec happened, bytes mean "the
   program never started (exec failed: <errno>)" [n1]. The monotonic clock starts at *go*.
5. **perfrun owns the program** [c37, n3, n11]: it registers, before *go*, every event in one wait
   (macOS kqueue: `EVFILT_PROC`/`NOTE_EXIT` on the child — `ESRCH` means it already exited —,
   `EVFILT_TIMER` for DEADLINE seconds after *go*, `EVFILT_SIGNAL` for SIGTERM, `EVFILT_WRITE` with
   `EV_CLEAR` on the record pipe acting on `EV_EOF`; Linux: a pidfd, a timerfd, a signalfd and
   `EPOLLERR` on the record pipe in one epoll). Its SIGTERM handling is installed at the fork, so a
   SIGTERM before *go* also kills the child's group. Deadline, SIGTERM or the reader's close →
   `SIGKILL` to the child's group (`-pid`), reap, record `timeout` or `stopped`. perfrun is not in
   that group, so it survives to write the record. On Linux perfrun also kills the group after the
   child exits normally (forked members may remain; on macOS fork is denied) [c117].
6. `waitid(P_PID, child, WEXITED | WNOWAIT)`, then the end counters (same fields, plus
   `ri_lifetime_max_phys_footprint`, `ri_child_*`); `wait4` for `ru_maxrss` and the switch counts.
   Recorded as **end minus baseline**: instructions, cycles, P-core instructions and cycles, CPU
   time (user + system, `cpu_us`; on Linux from `wait4`, including the moments before exec, named);
   context switches — macOS one total (`ru_nivcsw` at the end minus `pti_csw` at *ready*), Linux
   voluntary and involuntary from `wait4` (including the moments before exec) [c39, n8]. Recorded
   **as is**: the memory (macOS lifetime maximum footprint, which starts over at exec; Linux
   `ru_maxrss` × 1024 in bytes, which includes the time before exec — about perfrun's size, §6
   [n10]); wall from *go* to `waitid`.
7. The record: built in memory, one `write()`, `key value` lines, ASCII, at most 4 KiB, last line
   `end`: status (`ok`, `timeout`, `stopped`, `never-started <errno>`, or the launcher's failure in
   words), the program's exit code or signal, whether perfrun sent the kill, the counters, the CPU's
   name, the OS product version and build (`sysctlbyname`: `machdep.cpu.brand_string`,
   `kern.osproductversion`, `kern.osversion`; Linux: the harness reads `/proc/cpuinfo` and
   `/etc/os-release`), the 1-minute load average in hundredths; perfrun exits 0 [c40, c47]. In
   MODE `facts` perfrun writes only the computer's lines and `end` (for `perf show`, §3.9) [m24].
   **The harness reads it strictly** [c40]: at most 4 KiB + 1 read beyond the child line (more is
   "over 4 KiB"), the `child` line first, then each known key once, every number bounded (counts
   ≤ 2^63, load ≤ 100 000), unknown keys and a second record refused, `end` last.

**Judging a run**, in this order [c41–c43, n1, n9, m17]:
1. The harness's own end first: its timeout → `run-failed: timeout` (the C in step 1:
   `c-timed-out`); output over the cap → `output-too-large`; cancellation → no row.
2. perfrun did not exit 0, wrote no record, no `end`, over 4 KiB, a malformed line →
   **unmeasurable** "the launcher stopped before measuring" (that row only).
3. The record says `timeout` → `run-failed: timeout` (the C: `c-timed-out`); `stopped` → no row
   (the harness stopped it). Never a crash, never behaves-differently [m17].
4. No *ready* (the profile was refused, perfgo failed), sandbox-exec's own exit (65, 71), perfgo's
   125, or `never-started` → "the program never started" with the reason: on the C
   `c-could-not-start`, on the other side `could-not-start` — never behaves-differently or
   too-short.
5. macOS only: a `SIGKILL` perfrun did not send → **stopped-by-sigkill**, worded without asserting
   the cause: "the program was stopped by SIGKILL — perf's sandbox does this when a program tries
   to start another; if yours does not, it may have stopped itself or run out of memory".
   `ri_child_*` non-zero → the same (unreachable while fork is denied) [n9].
6. Otherwise the program's own end (exit code or signal) and its counters. A zero baseline is
   never subtracted.

**The harness side** [c38, c27, n3, n4, m2, m17, disputed: the window]: a new runner entry
`run_measured(launcher, record_pipe, ack_pipe, run_dir, input, collect)`, all of it under the
`LIVE` registry's lock as spawns are. It spawns perfrun (registering perfrun's group), drops the
`Command`, reads the `child <pid>` line (bounded: 64 bytes, within the step-1 allowance below),
registers the child's group, and only then writes the ack — so the program never runs before both
groups are registered, and a cancel before the ack ends perfrun, whose death ends perfgo before the
program starts. **The child's group is dropped from the registry when perfrun's record is read or
perfrun is reaped** (perfrun reaps the child before writing its record or dying of a signal it
handles), and the harness never signals it after that: a cancel after perfrun's end signals only
groups whose leaders are not yet reaped [m2]. Deadlines: perfrun's DEADLINE is the target's
`[oracle] timeout_secs` for timed runs and `timeout_secs` + 60 s for step-1 runs — every new
binary's first exec, which comes after *go*, falls in step 1 [c7, n4, m17]; the harness's own
timeout for a perfrun is its DEADLINE + 15 s, so perfrun's fires first and its record says
`timeout`. On the harness's timeout, overflow or cancellation it sends SIGTERM to perfrun, waits a
bounded grace (250 ms), then SIGKILL to perfrun's group and (while registered) the program's. perf's
run temp dirs are 0700 and registered with the signal handler's live-folder list (as the map's
random folder; verify's run temp dirs get the same).

### 3.4 The perf profile

The scenario profile (no network, no signal but to itself, reads denied under the home folder and
the target root except the side's binary, perfgo and the run's temp dir, writes only the temp dir),
with exec allowed for exactly two literals — perfgo and the side's program — and its fork rule
`(deny process-fork (with send-signal SIGKILL))`, so a program that tries to start another is
stopped at once rather than timed failing [c42, c94]. A run cannot read the other side's binary. A
program may exec **itself** (its path is allowed): counts carry on across the exec, the memory reads
only the last image (§6) [c34].

### 3.5 Running a row

**Per side against the C, per workload** [c48–c56, n20, n25]:
1. **Same end and output**: C, other, C — for every row, with its own two C runs (no C run is
   shared between rows): streams captured, rewritten as a scenario's (`$` → `$$`, the temp dir →
   `$TMPDIR`, **each side's own slot folder** → `$PROGDIR`). The two C runs differ →
   **c-unstable** ("the C prints something different each time on big-text: stdout, byte 41 — two
   matching runs do not prove it is stable"). A C-side failure (crashed, timed out, over the output
   cap — "prints more than 64 MiB; give it an output-file argument — then perf times it but no
   longer compares its output" — could not start) is the C's: that workload's remaining rows take
   the same outcome without running it again, marked `copied_from: "c"`. Otherwise the other side
   differs in its end or its streams → **behaves-differently** ("exits 1 where the C exits 0";
   "stdout differs at byte 40 961"; "the Rust ends by signal 11 where the C exits 0"). A non-zero C
   exit is not refused — it is named on the row from its stored exit code and in the progress line:
   "the C exits 1 on big-text: its error path is timed"; when both exit non-zero, both codes [c51].
2. **Too short**, decided per row from step 1 with a floor of **1e9 instructions** (≈ 0.1–0.5 s of
   CPU for code that mostly computes; memory-bound code can take seconds) [c53, c54, m6]:
   - **both sides under 1e9 and the C under 0.5 s of CPU** → **too-short**, quoting both sides'
     step-1 CPU times and the fix: "too short to time: the C ran 40 ms of CPU and the Rust 360 ms on
     small-text — use an input at least about 7× bigger (13× for half a second)": the first factor
     is the floor over the smaller side's instructions, the second half a second over the C's CPU
     time, both rounded up, and a factor of 1 or less is never printed; "check the workload's
     options first" when the C wrote nothing or exited non-zero [c98];
   - **either side under 1e9** (and not too-short) → measured, marked **short run**: never "about as
     fast"; inside the margin "can't tell on a run this short — use a bigger input"; no memory line
     (its footprint is mostly start-up);
   - otherwise measured in full.
   Step 1's streams go through pipes (≈ 4–5 % more work than `/dev/null`); the floor is judged on
   them, the timed words on step 3 [c54].
3. **n timed runs a side, interleaved** C, other, C, other, … (n = `--runs` if given, else the
   workload's `runs`), streams to `/dev/null`. No separate warm-up: every binary's first exec was in
   step 1. A timed run that ends differently from its side's step-1 runs ends the row:
   **run-failed** with the side, the run's index and how it ended recorded (`failed_run`) [c55, c56,
   m28].

**The C alone, per workload** (the baseline row; §3.6): C, C (the same check of end and output,
c-unstable or a C-side failure as above), the floor on the C alone (too-short when it is under 1e9
instructions and 0.5 s, with the same fix words — the day-one warning [n27]), then n timed runs of
the C.

### 3.6 The C alone [n27]

Every `perf run` without `--unit` or `--as-it-stands-only` measures the C alone on each workload
first: outcome **baseline** with n runs, or a C-side outcome, or too-short. It is stored in
`program.json` under `c_alone` (§3.9), one row per workload, with `other` absent; its `compilers`
input is `cc`'s line only (rustc never builds it) [m22]. Its words: "the C: CPU about 1.2 s here
today (varies with load) · 12.4 MB · 1.21e10 instructions". The View's "The original C" lines read
these rows, and every C-side outcome is shown there once, never on a unit (§3.11) [m23]. Event side
`c`.

### 3.7 Outcomes (closed set) [c57, m20]

- **Measured**: `baseline` (the C alone), `measured` (a side against the C; a short run is
  `measured` with `short: true`).
- **What the code does**: `behaves-differently`, `stopped-by-sigkill`, `c-unstable`, `c-crashed`,
  `c-timed-out`, `output-too-large`, `too-short`, `run-failed: timeout | exit | signal`.
- **What the set-up could not do**: `not-verified: <reason>`, `replaces-mismatch`,
  `crate-does-not-build`, `does-not-link`, `mixed-panic`, `input-unusable`, `c-could-not-start`,
  `could-not-start`, `run-failed: unmeasurable`.

**One rule for set-up outcomes** (cited by §3.1 and §3.9): a set-up outcome is **never stored over
any earlier row** of any outcome. It is kept beside that row as `last_try: {outcome, units}` (numbers
and closed values only), so the View says "measured before; the last Measure could not run: u002's
crate does not link — see the run". With no earlier row, it is stored as the row, so the person sees
it [c77, m20].

Every outcome is a result (exit 0); a set-up failure (the launcher or the C does not build, the lock
is held, the workloads file is missing, empty or has an error, an unsupported platform, a home folder
under a temp root) is an error (exit 1); a clap usage error (`--unit` with `--as-it-stands-only`) is
exit 2 [c80].

### 3.8 The words (std only, in `harness-core`) [c58–c70, m26]

Per metric, the **Hodges–Lehmann shift**: the median of every pairwise log-ratio
`ln(other_i / c_j)`, its distribution-free interval `[D(c+1), D(m·n−c)]` from the sorted pairs, `c`
the largest u with `P(U ≤ u) ≤ 0.025` under the exact (m, n) Mann–Whitney null (a dynamic program
in u128) for the m and n **runs that have a value** — a run whose value is missing, not finite or
not above zero is dropped whole before any pair is formed; with fewer than 5 values on either side,
no words for that metric [c60, m14]. At m = n: c = 2, 8, 23, 64, 127, 341 at n = 5, 7, 10, 15, 20,
31; confidence 96.8, 96.2, 95.7, 95.5, 95.1, 95.0 % (computed exactly). It is at least 95 %
confident at every m, n for time and instructions; memory's clustered footprints are covered by its
margin instead [n24]. Percentages are rounded to one decimal under 10 % and to whole numbers above,
the interval's ends **outward** (the lower end down, the upper up), so the shown ends never
contradict the branch chosen; ends that round equal show one number [c69, m15].

- **Time** (the headline). The metric, per row [c59, n12]: **performance-core-normalised cycles**
  (P-core cycles per P-core instruction × all instructions) when the platform gives a P/E split —
  macOS V6 with the P-core fields present — and at least **three quarters** of the row's 2n timed
  runs have at least half their cycles on the performance cores; a run with no P-core cycles has no
  value (dropped, above). Otherwise **raw cycles**: on a platform with no split (Linux, macOS V4,
  Intel Macs), said ("cycles"); or because the share rule moved the row (fewer than three quarters
  qualified) — then the row **always** reads "can't tell — K of the 30 runs ran mostly on the
  slower cores: the computer was busy; close other work and measure again", whatever the raw
  interval says (it is in the details), since raw cycles then measure where the runs were scheduled
  [m3, m10, c46, c62, n15]. No counters at all → CPU time, said. Never mixed in a row. The headline
  includes the kernel instructions preemption adds (0.2–1.5 % at load 14–52, §2). The assumption is
  stated: an efficiency-core stretch is counted at the performance-core cost. With margin
  **M = 2 %**, the interval [a, b]:
  - inside ±M → **"about as fast as the C (within 2 %)"**;
  - a > +M → **"slower by X % (a–b %)"**; X ≥ 100 % → **"3.7× as slow (3.4–4.0×)"**;
  - b < −M → **"faster: takes X % less time (a–b %)"**;
  - **near the line** — b − a ≤ 2M and the interval excludes 0 (a > 0 or b < 0) →
    **"about X % slower (a–b %) — too close to the 2 % line to call"** (the mirror for faster),
    X the shift [c61, n13];
  - otherwise, the interval excludes 0 → **"probably slower, by about X % (a–b %) — not clearly
    past the 2 % line"** (or faster), at any n [m4, n14];
  - otherwise (the interval holds 0):
    - n < 31 and the **projected interval** gives an answer → "can't tell yet — measure again with
      more runs: <command> (about 101 s a row)". The projected interval is centred on the shift with
      the width scaled by the exact ratio of interval widths at n and at 31 (0.32, 0.41, 0.53, 0.67,
      0.79 at n = 5, 7, 10, 15, 20: a simulation of normal samples with the exact critical values,
      pinned by a test); "an answer" is any branch above [m4, c62];
    - n < 31 otherwise → "can't tell: the estimate is ±Y % — more runs would not settle it; measure
      on a quieter computer";
    - n = 31 → **"no clear difference: within ±Y %"**;
    Y = max(|a|, |b|) [n23, m16]. The command is per side: `harness perf run --unit u001 --workload
    big-text --runs 31`, `harness perf run --as-it-stands-only --workload big-text --runs 31` [n22].
- **Instructions** (details; never speed words; **M = 1.5 %**, above Rust's start-up at the floor
  [n19, c53]): "about the same instructions (within 1.5 %)", "X % more instructions (a–b %)", "X %
  fewer", near the line "about X % more instructions — too close to the 1.5 % line to call",
  otherwise "instructions: can't tell". A unit row whose median difference in instructions is
  between 0.5e7 and 2e7 adds "(about Rust's fixed start-up)" — the start-up is a fixed ≈ 1e7, not a
  share [c31, m12]. No counters → "instructions not counted" [c70].
- **Memory** (details; macOS lifetime maximum footprint, Linux maximum RSS — named; no counter
  needed): the same interval with margin **max(5 %, min(1 MiB, 20 %))** of the C's median — about 8 %
  on zopfli, 20 % for a 1 MB program [n18]: inside → "about the same memory (within 8 %)"; past →
  "uses about X % more memory (a–b %)" or "less"; near the line only when the interval excludes 0
  and is at most one margin wide → "about X % more memory — too close to the 8 % line to call";
  **every other interval** → "can't tell — memory varied from run to run" [c66, n19, m5]. No memory
  line on a short run.
- **CPU time shown** [c67, n16]: the C's median CPU time a run (user + system, the sandbox's set-up
  subtracted), labelled as varying with load ("CPU about 1.21 s here today"); the other side's
  seconds come from the headline — the C's × (1 + shift), "→ about 1.28 s (from the estimate)" —
  so the two never contradict. With no counters, CPU time is the headline itself and both medians
  are shown.
- **Several cores** [n17, m9]: when either side's median CPU time exceeds 1.2 × its median clock
  time, the row says "uses several cores: the words compare total CPU work, not waiting — clock
  time 0.70 s → 0.25 s", and its short form gains "· parallel".
- A unit row that says "about as fast" adds "(perf cannot tell whether big-text runs this unit's
  code)" [c68].
- Not worded, with reasons: the P-core share of each run and the context switches (kept in the
  file; words about scheduling would say more than perf knows) [c69].

The CLI, per row: a headline line and detail lines, each wrapped at 80 columns with a 6-space
hanging indent [c69]:
```
perf: u001-katajainen on big-text — slower by 6.2 % (4.1–8.3 %)
      CPU about 1.21 s → about 1.28 s · 3.1 % more instructions
      · memory about the same · 15 runs each
```
The C alone: `perf: the C on big-text — CPU about 1.21 s here today · 12.4 MB · 15 runs`.

**Short forms** (the View and the unit header; the full sentence is the selected row's detail), each
at most 26 columns with the longest numbers (`≈2.3 %`, `±12 %`, `(4.1–8.9 %)`) [n30, m7, m8, m23]:

| outcome / words | short form (columns) |
|---|---|
| about as fast | `about as fast` (13) |
| slower | `slower 6.5 % (4.1–8.9 %)` (24) |
| ≥ 2× | `3.7× as slow` (12) |
| faster | `faster 12 % (10–14 %)` (21) |
| near the line | `close call: ≈2.3 % slower` (25), `… faster` |
| probably | `probably slower ≈3.1 %` (22), `… faster` |
| no clear difference | `no clear diff ±12 %` (19) |
| can't tell — busy | `can't tell: busy` (16) |
| can't tell — more runs | `can't tell: more runs` (21) |
| can't tell — noisy | `can't tell: noisy` (17) |
| short run, can't tell | `short run: can't tell` (21) |
| too-short | `too short to time` (17) |
| behaves-differently | `prints differently` (18), `exits differently` (17), `Rust crashed` (12) — by `first_difference` |
| stopped-by-sigkill | `stopped by SIGKILL` (18) |
| run-failed | `run failed: timeout` (19), `run failed: exit` (16), `run failed: signal` (18) |
| could-not-start, set-up | `could not start` (15), `not measured` (12) — the reason in the detail |
| C-side (under The original C only) | `C output unstable` (17), `C crashed` (9), `C timed out` (11), `C output too large` (18), `C could not start` (17) |
| several cores | the form + ` · parallel` when it fits, else `parallel` in the detail |

### 3.9 Results: `migration/perf/program.json` and `migration/perf/units/<id>.json` (`ruharness-perf`, v1)

`program.json` holds two lists, `c_alone` and `as_it_stands`, one row per workload each; a unit's
file one list of rows. A measurement replaces only the rows it measured (§3.7's rule for set-up
outcomes). Each row [c71–c79, n31, m20, m28]:
- `workload`, `outcome`; on `baseline` and `measured` rows also `short`, `runs` (5–31) and
  `platform_metrics` (`macos-v6-pnorm`, `macos-v6-busy`, `macos-v4-cycles`, `linux-cycles`,
  `linux-hybrid-summed`, `cpu-time`), absent on the others [c72];
- `inputs`: the workload digest, the program digest, the crate digest(s), the unit's `replaces` and
  `program_name`, `units` and `left_out` (as-it-stands rows), `recipe` (`perf-recipe-1`, a constant
  in `harness-core` beside `PERF_LAUNCHER`), `launcher` (`PERF_LAUNCHER`), `computer` (OS product
  version and build, arch, CPU), `compilers` (the first line of `cc --version`, and of `rustc -V`
  except on `c_alone` rows); the facts digest is not an input (the program digest covers the C)
  [c33, c78, m22];
- on `baseline` and `measured` rows: `c` (and `other`, not on `baseline`), arrays of exactly `runs`
  entries, each a run: `instructions`, `cycles`, `cpu_us`, `wall_us`, `memory` (bytes) — each ≥ 1
  when present; `p_instructions`, `p_cycles`, `switches` (macOS) or `switches_voluntary` and
  `switches_involuntary` (Linux) — each ≥ 0 when present, the P counts both present or both absent;
  `load` (hundredths, ≥ 0); `end` (`exit N`, 0–255, or `signal N`). A counter the platform does not
  give is absent, never 0 [c60, c75, n31];
- on other outcomes: `c` and `other` absent; `step1` holds what step 1 measured, by name (`c_first`,
  `other`, `c_second`: instructions, cpu_us, `end`, stdout and stderr sizes) — absent on a set-up
  row and on a row `copied_from: "c"`; a `run-failed` row adds `failed_run: {side, index, end}`
  (`end` also `timeout`) [c72, m28];
- `first_difference` (behaves-differently only): numbers and closed values only — the stream
  (`stdout`, `stderr`, `exit`), both lengths, the byte offset, both ends; the strict reader refuses
  text there;
- `last_try` (beside an earlier row): `{outcome, units}` (§3.7) [m20].

**No words are stored**: they are computed from the numbers by §3.8 on every read. Read strictly by
outcome (`read_regular` with a 4 MiB cap; arrays of the stated length; p_instructions ≤
instructions, p_cycles ≤ cycles; the ≥ 1 and ≥ 0 rules above; runs 5–31; ids from the alphabet,
once; a new key is a new `schema_version`) [c75, c79]. Written after every row with `write_atomic`
into a folder the CLI resolves with `safe_ledger_dir` and passes in (the check stays in
`harness-cli`, the rows in `harness-oracle`, the readers and the words in `harness-core`) [c76]; a
linked `migration/perf` (or a linked parent, or a linked workloads or results file) refused by the CLI
and the cockpit on read and on write [c75]. Rows of removed workloads are dropped on the next write;
a unit file of a unit no longer in the plan reads "no longer in the plan". The results files count
in the cockpit's retained-read budget [c14].

**Kept outputs** of a behaves-differently row: `migration/build/.perf-out/program/` and
`migration/build/.perf-out/units/<id>/` — a dot folder no unit id can name (ids start with a letter
or digit) [n7, n33] — `<workload>.{c,other}.{stdout,stderr}`, each capped at 64 MiB, written with
`write_atomic` through a helper that refuses links, in no tool profile; replaced when that row is
re-measured, removed when it no longer differs; the row records each file's size and blake3 [c74].
A non-zero C exit's stderr is not kept (its size is in `step1`; measure with a terminal to see it)
[c51].

**Current** iff each input equals today's, each with its own reason: "your workload changed", "the
program's C changed", "the unit's Rust changed", "its replaced files changed", "the program's name
changed", "u-hash was accepted since" / "u-tree is left out now" / "the plan's order changed", "the
system was updated (26.5.2 → 26.6)", "measured with other compilers (rustc 1.90.0 then, 1.91.0
now)", "measured on another kind of computer (Apple M1)", "made by another version of the harness".
`harness perf run` judges all. `harness perf show` judges the computer with `perfrun facts`
(harness-owned, no allowlist), the compilers with `cc --version` and `rustc -V` as tool runs when
the target's allowlist has them (else "compilers not checked"), or with `--no-check` says "computer
and compilers not checked" [n35, m24]; on Linux it reads `/proc/cpuinfo` and `/etc/os-release` and
runs the compilers with `--allow-unsandboxed`. The cockpit judges what it can without starting a
process and says the rest is not checked (§3.11). A results file is evidence, not ledger truth;
committing it keeps a history in git; a committed file whose digests match reads current — it gates
nothing, and the next measurement replaces it [c78, c79]. Stale facts: `perf run` refuses ("scan
first"), as `features map` does.

### 3.10 The CLI [c80–c84]

- `harness perf run [--target T] [--unit ID]… [--workload ID]… [--runs N (5–31)]
  [--as-it-stands-only] [--allow-unsandboxed] [--json]` — the writer lock; without `--unit` or
  `--as-it-stands-only`, the C alone, the program as it stands and every measurable unit; `--unit`
  measures only those units; `--as-it-stands-only` only the program as it stands, and with fewer than
  two measurable units says why [n27, m29]. Unknown ids: exit 1 naming the known ones.
- `harness perf init [--target T]` (resolves `migration/perf` with `safe_ledger_dir`; refuses unless
  `workloads.toml` is absent by lstat, as `features init` does) [c83]; `harness perf save [--target
  T] --expect … --bytes …`; `harness perf show [--target T] [--no-check] [--allow-unsandboxed]`
  (the words of the stored rows, current or out of date).
- Progress (`message` events): "building the launcher…" (only when its cache is stale), "building
  the C program…", "u001 — building its Rust…", "the program as it stands — u001, u002 (u-tree left
  out: …)", "the C on big-text — checking it ends the same way twice…", "the C exits 1 on big-text:
  its error path is timed", "u001 on big-text — C, u001, C…", "u001 on big-text — timed run 9 of
  30…", "keep the computer quiet while it measures". A behaves-differently row is printed first and
  in full, with where both outputs are kept. Summary: "perf: measured 4 rows, 1 too short, 0 behave
  differently — wrote migration/perf/ (commit it to keep a history; perf compares what the program
  prints and how it ends)".
- Events: `perf-row {side: "c"|"program"|"unit", unit (unit rows only), workload, outcome, words}` —
  `words` a display-only courtesy, recomputed by every reader (SCHEMAS says so beside the event)
  [c83].

### 3.11 The cockpit [c85–c92]

- A **Speed** group in the tree beside Features: `Speed (no workloads file)`, `Speed (no workloads
  yet)`, `Speed (workloads file has an error)`, `Speed (the C measured, no unit yet)`, `Speed (2 of
  3 units measured)`; acts **Write your workloads file** / **Edit the workloads file** / **Continue
  my workloads draft**, **Measure speed** (greyed, with the state's words, in the first three
  states), on a unit **Measure this unit's speed** and **Measure again with more runs**, and on the
  As-it-stands heading **Measure the program as it stands again with more runs** (`--as-it-stands-only
  --runs 31`, the workloads that could not tell) [c16, c63, n22, m29]. Each confirm dialog says what
  runs, how many times, the estimate (§6), that it writes `migration/perf/`, scratch folders under
  `migration/build/` and — when it is stale — the launcher cache in the home folder, and no
  verdict, all under the ledger's lock, that Cancel keeps finished rows, and to keep the computer
  quiet; when it ends: "Measured 4 rows — see Speed" [c92].
- **Inputs read by the cockpit** [c14, m21]: to judge "your workload changed" it hashes each input
  off the UI thread (in `Snapshot::load`, shared with the MCP reads), with the same confined,
  bounded read perf uses (`read_regular`, under the target root, 64 MiB), each digest **cached by
  (device, inode, size, modification time)** across loads; while a perf run holds the lock, no input
  is hashed (its rows read "can't check while measuring") and the 2-second reload hashes nothing new
  either. When hashing an input would pass `preflight`'s budget, that input reads "can't check:
  inputs too large to hash here" — the read itself is never refused for it; the program-digest
  budget applies with a features **or** perf file. An input perf could not use reads with the CLI's
  own reason words (§3.1), never "your workload changed" [c13].
- The **Speed View** (golden at 54 columns, inside an 80-column terminal):
  ```
  Speed — your workloads, the C against the Rust in use
  measured on Apple M3 with rustc 1.94.1 (not checked
  here) · as verify builds them · 15 runs each ·
  compares what the program prints and how it ends

  The original C
    big-text      CPU about 1.21 s · 12.4 MB
    many-small    too short to time
  As it stands (2 of 3 units — u-tree left out)
    big-text      slower 6.2 % (4.1–8.3 %)
    many-small    too short to time
  u001-katajainen
    big-text      slower 6.2 % (4.1–8.3 %)
    many-small    too short to time
  u002-hash
    big-text      about as fast
    many-small    too short to time
  ```
  The header is built from the rows: the computer and compilers they record (rustc's line cut to
  its version), "not checked here" (the cockpit cannot read the CPU or OS build without a process),
  "n runs each" or the per-row n when they differ [c78, n30]. Workloads in file order; units sorted
  worst first by their worst row, the order: behaves differently, stopped by SIGKILL, run failed,
  slower (by the shift), probably slower, close call (slower), could not start, not measured,
  can't tell, no clear difference, short run, too short, about as fast, close call (faster),
  probably faster, faster; out-of-date rows last within a unit; C-side outcomes never sort or head
  a unit (they belong to the C); ids cut with …; a stale row's short form dimmed with "out of date:
  …" under it; a row with a `last_try` adds its line [c89, m20, m23]. The selected row's full
  sentence and details show below the list, wrapped.
- A unit's header: `Speed: slower 6.2 % on big-text` and, on its own line, the interval and "1 of 2
  workloads" — wrapped to the pane, never past 54 columns [c89, m23].
- The project summary gains `Speed: 3 of 5 units measured — 1 slower, 1 about as fast, 1 parallel ·
  1 out of date — see Speed` (several-cores rows counted apart) [c90, m9].
- A **behaves-differently** row [c86–c88, n29, m25]: a fact on the unit — "With u001's Rust the
  program prints differently on big-text (stdout, byte 40 961) — verify does not run this workload"
  (or "exits differently", "the Rust crashes", from `first_difference`) — with the next step
  "Compare the outputs; then change the unit's Rust (§ next bullet) and measure this unit again".
  No Re-check (it would pass and loop). It **clears only when a re-measure ends `measured` or
  `too-short` with the same output**; after a change to a code input (the workload, the C, the
  unit's Rust, its `replaces`) or the environment it stays, as "found before the unit's Rust changed
  — measure this unit again to check" or "found on Apple M3 with rustc 1.90.0 — measure again to
  check". An as-it-stands difference goes on the As-it-stands heading and in the project summary,
  naming the units it holds; "no unit's Rust differs alone" only when every held unit's own row on
  that workload is current and measured the same, else the commands that would measure the missing
  ones (`harness perf run --unit u002 --workload big-text`) [c87, n29]; on a unit only when its own
  row differs. **Compare the outputs** shows the kept files side by side when they are regular
  files (`read_regular`, bounded) whose size and blake3 match the row, control characters shown
  escaped as in reasons (`harness_core::text`); else "the two outputs are not on this computer —
  measure again" (this unit, or the program as it stands) [c73, c88].
- **A slower row's next step**, as words from what the cockpit knows, with its own labels [c91,
  m18]: "perf times the Rust in use. If speed matters here:" then
  - the unit's crate in use matches a recorded model-made attempt: "Modify that attempt with a note
    about speed (give these numbers), then Replace u001's verified crate with the new attempt,
    measure this unit again — and if it is not faster, Replace it back with a-1234";
  - it matches no recorded attempt (written by hand): "commit the unit's crate first (git) —
    replacing it deletes it; then edit the Rust by hand, Re-check, and measure again".
  Help gains a Speed section, those paths, "perf stops even a fork; verify allows a fork but not
  starting another program", "perf compares what the program prints and how it ends", and the
  glossary (CPU time, instructions, memory, performance cores) [c90, c114, n34].
- **The MCP reads** export per unit, and for the As-it-stands heading and the C alone, a Speed fact
  of closed values and numbers: the worst outcome (from §3.7's set), the workload id, the shift and
  interval as numbers, `current: true|false` with reasons from a closed set, and
  `computer_checked: false`; fenced as the features facts are [n35, m27].
- Acts' argv: `with_sandbox_flag(harness_argv(["perf", "run", …, target_arg]))`.

### 3.12 Security (summary) [c93, c94, n2]

The program runs only under the perf profile: no fork (killed on trying), no signal out, no network,
no reads under the home folder or the target beyond its binary, perfgo and its temp dir, writes only
its temp dir. **Nothing the program starts, and not the program itself, outlives its run** on macOS
while the harness is alive or ends by its own cancel path: the program cannot fork; it leads its own
session, so it cannot leave its group; it never starts before the harness has registered that group
(the ack), and before that a dead perfrun ends perfgo; perfrun kills the group on its deadline, a
SIGTERM or the harness's end; the harness kills it on timeout, cancel and its signal handler. One
case is not covered and is named: **something outside the harness kills both the harness and
perfrun with SIGKILL while the program runs** — the program then runs on in its sandbox until it
ends (no CPU limit holds on macOS) [n2]. The launcher is the one unsandboxed harness binary: built
with a compiler found through root-owned paths into a private cache no tool profile can write, hash-
checked before each run. The counters return on a pipe the program never holds (macOS; Linux relies
on `PR_SET_DUMPABLE`, unchecked). The input is read once, confined and bounded; results carry no
words and are read strictly — but a committed results file can be forged (it gates nothing; the next
measurement replaces it). Run temp dirs under `TMPDIR` can be rewritten by a same-user process that
outlived a build (as every run's today); the program's counts are whatever its behaviour makes them.
No new dependency; `unsafe` Rust stays forbidden everywhere.

## 4. Tests and checks [c95–c107, c111]

- **The words** (in `harness-core`): the critical values and confidences at m = n (n = 5, 7, 10, 15,
  20, 31: c = 2, 8, 23, 64, 127, 341; positions D(3)/D(23), D(9)/D(41), D(24)/D(77), D(65)/D(161),
  D(128)/D(273), D(342)/D(620); 96.8–95.0 %) and at m ≠ n (dropped runs), with ±1 mutations failing;
  fewer than 5 values gives no words; the projection factors (0.32, 0.41, 0.53, 0.67, 0.79) pinned;
  a seeded synthetic A-vs-A at 0.5 % noise "about as fast" at every n; the interval narrows as n
  grows; near the line: seeded A-vs-A at loaded noise gives it in at most about 5 % of rows, a +2 %
  shift in most rows at n = 31; [−0.5, +3.5] is "probably" only when it excludes 0, else can't
  tell; a +3 % shift at n = 31 with σ ≥ 4 % "probably slower"; a +5 % shift at σ 4 %, n = 15, is
  never told "would not settle it" when 31 runs say slower [m4, m13]; the busy words exactly when
  the share rule moved a macOS V6 row (fewer than three quarters qualified), on each recorded window,
  and never on a Linux, V4 or quiet row [m10, m13]; the recorded fp-aa7 #248 row (identical
  binaries, raw cycles) never "slower" [m3]; fallbacks (V4, p_instructions = 0 read back from a
  file, a small P-share, a self-backgrounding program, a stored 0) never panic or divide by zero;
  on the recorded rows the CPU seconds shown never contradict the headline [c96, n16]; instruction
  intervals crossing +1.5 % and −1.5 %; the start-up note at 1e7 difference on a 4e9 and a 1e9 C,
  not at 0.9 % real work [m12]; outward rounding never shows a near-the-line interval inside ±2 %
  [m15]; the CLI lines at 80 columns and every short form at 26 with the longest numbers [m7].
- **Memory**: the same allocating binary on both sides under load never "more/less"; a synthetic
  two-cluster sample; a 1 MB program's margin is 20 %; the recorded windows that fell in revision
  3's gap (e.g. [−7.9 %, +2.4 %] against 7.2 %) read "varied" [c97, n18, m5].
- **Threads**: a 1-thread and a 4-thread side — "uses several cores" with the clock times, and
  "· parallel" in the short form [n17, m9].
- **The floor**: either side of 1e9; the C under and the other over, and the other under (short
  run); both under, with both step-1 times and the factors, never "1×"; a memory-bound C over 0.5 s
  of CPU under 1e9 instructions is measured, not too-short [m6]; step 1 through pipes, the timed
  words from step 3; the first row's words carry no first-exec or sandbox time [c98, n20].
- **The launcher and trampoline**: an exact baseline (a test-only delay before *go* changes no
  count); argv[0] = the bare name; a self-exec'ing program (counts carry on, memory the last image);
  a missing program and an exec the profile denies are each "never started", a real exit-127
  program is not [n1]; a refused profile, a child ending early → unmeasurable or never started as
  §3.3 says; an empty program never reads 0; perfrun's own deadline gives `run-failed: timeout` on
  both sides and `c-timed-out` on the C — never crashed, never behaves-differently [m17]; a fresh
  binary with `timeout_secs = 5` is not timed out in step 1 [m17, n4]; a `setsid()`/`setpgid()`
  spinner that ignores SIGXCPU and SIGTERM is dead after timeout, cancel, overflow, perfrun being
  killed and the harness's signal handler [n2, c93]; **a cancel between perfrun's fork and the ack
  leaves no program running, and the program never ran** [disputed]; a cancel after perfrun has
  reaped the child signals no group (the registration is gone) [m2]; killing the harness while the
  program runs kills the program, and perfrun stays idle meanwhile (no spin) [n11]; a 4-thread
  program is not judged crashed; a self-SIGKILL is worded without asserting a fork [n9]; a record
  cut short, over 4 KiB, with a key twice, an unknown key or two records; exit 0 with no record; a
  perfrun that dies before writing ends the read at once (the `Command` dropped) [c40]; macOS
  switches one total, never a 0 voluntary [n8]; no 2-s drain wait in any case (each case timed)
  [c101].
- **The launcher's build and cache**: a process under a crate build's tool profile cannot write the
  cache, with the target under `/var/folders` (the tests' own place) and outside the temp roots
  [n5]; **a tool-profile process that rewrites `xcrun`'s per-user cache does not change the compiler
  the launcher is built with** [m1]; a home folder under a temp root is refused; two harnesses
  building at once take turns (the lock); a stale version folder replaced; a changed binary refused
  by its hash.
- **The profile**: `system`, `popen`, `posix_spawn`, `fork`, `vfork` each killed; no signal out; a
  run cannot read the other side's binary.
- **Paths**: the same program, allocating nothing, at `p000` and `p012`: argv[0], PATH on both
  streams, the executable path after the `$PROGDIR` rewrite, output and memory equal; a same-named
  tool earlier on PATH never reads "could not start"; inodes unchanged [c103].
- **Linux** (CI's ubuntu job; nothing of it runs here): counters opened before release, `inherit`,
  per-type events summed, `PR_SET_DUMPABLE` on perfrun (the program cannot open
  `/proc/<ppid>/fd/0`), `PR_SET_PDEATHSIG`, a forking program killed on timeout with the record
  intact, a forked child that outlives the program killed after it exits [c117], `ru_maxrss` in
  bytes [n6, n10].
- **Workloads, results, evidence, units**: each rule of §3.1–§3.2 and §3.9 (the caps, `{input}` as
  a whole argument, a refusal naming its workload with line and column, each input-unusable reason's
  words in the CLI and the cockpit, a missing input skips only its workload, the input changed after
  the read changes nothing, the digest ignores `runs`, a 7-run row current after a `runs` edit, links
  refused at every entry point — `perf init`, `perf run`, the cockpit — for the folder, a parent, the
  workloads file and a results file; **a `crate-does-not-build` after a behaves-differently row keeps
  the finding, its kept outputs and adds `last_try`** [m20]; removed workloads dropped; the
  early-stop rows' `step1`, `failed_run` and `copied_from`; exit code 0 and load 0 accepted; units
  named `p`, `P`, `program`, `perf-out`; a half-done Accept taken under perf's own lock, a legacy
  `.prev`, a stale unit, a pending or blocked fresh unit, `replaces` edited after verify, an empty
  `replaces`, a `sub/unit.c` layout, a non-linking unit's row while the next is measured; **a
  model-made unit beside a hand-written one (mixed-panic read from the archives, the words' terminal
  path)** [m19]; two units with one built with `lto` (does-not-link on the heading); a left-out unit
  read as "left out", never "accepted since" [m22]; zero and one unit never C against C; one unit
  never measured twice) [c104–c106].
- **The cockpit**: the input digests cached across loads and none hashed while a perf run holds the
  lock [m21]; an input over the budget reads "can't check" and the target still opens; the Speed
  View's golden with a 24-character workload id, two workloads, each short form, a `last_try` line
  and a several-cores row; the worst order over every outcome; C-side outcomes only under The
  original C; the unit header within 54 columns; the next-step words for a model-made and a
  hand-written crate [m18]; a behaves-differently fact kept through an Accept as "found before …"
  and cleared by a same-output re-measure [m25]; the MCP fact's closed values [m27].
- **End to end** on a mini target with two accepted units (a test fixture: two small `.c` files
  replaced by two tiny crates), plus zopfli's u001: a generated input; a time-printing C with three
  sides is never behaves-differently on any side, and c-unstable only when the second rolls over
  between a row's two C runs [n25, c107]; a crashing C → a C-side outcome on every row, shown once;
  equal streams but exit 1 → behaves-differently ("exits differently"); output over the cap;
  Compare after another unit's measurement, and after the kept files are removed; no Re-check
  offered after a green Re-check; an as-it-stands difference with no unit's own; the C alone on day
  one (too-short warned); `perf show` with and without `--no-check`.
- **Mutation checks** of §3.2's selection (freshness, the marker, `replaces`), §3.3's judging order,
  §3.5's order, §3.7's set-up rule and §3.8's rule; re-run §2's premise with the built launcher under
  load.

## 5. Order of work [c108–c110]

Each step committed green: (b) `harness-core`: the workloads and results files and their strict
readers, `PERF_LAUNCHER`, `perf-recipe-1`, **the words, short forms and worst order** (std-only
functions of the stored numbers, used by the CLI, the cockpit and MCP), `promotion_marker` made
public [m26]; (c) the shared build refactors (`whole_cc_into`, objects once, the `replaces` helper,
the `.perf` and `.perf-out` helpers, slots, hashes, the archive reader for the panic runtime); (a)
the launcher, trampoline, cache (with the compiler found through root-owned paths) and profile, with
the harness's `run_measured`, the ack, the second registered group and the SIGTERM-then-SIGKILL
change; (d) rows and measurement in `harness-oracle`; (e) the CLI; (f) the cockpit (Speed group and
View, the generalised Edit flow, the cached input hashing, the MCP fact); (g) SCHEMAS (writer rows
for `workloads.toml`, `migration/perf/**`, `migration/build/.perf/**` and `.perf-out/**`, the crate
`target/**` and `Cargo.lock`, the launcher cache outside the target; the `perf-row` event; results
are forgeable and non-canonical), the tutorial, the testing guide's Part 11 on liblzg (make an input
and commit it, time the C by hand, write the workloads file, measure, read the words — a word table,
u-version (Step 6.3, Part 8) as the unit no workload reaches; whether `lzg -9` on a 2.5 MB
dictionary file clears the floor is unchecked: no liblzg here). Then the code review, fix passes
each checked, mutation checks, the handoff [c110].

## 6. Residuals and costs [c112–c117]

- **Rust's fixed start-up** (≈ 1.04e7 instructions for a std-using unit, 2.8e6 cycles, 48 KiB; a
  no-std unit adds nothing) is in every unit's and the program-as-it-stands's numbers: at the floor
  about 1 % of the instructions (under the 1.5 % margin, named on the row when it can explain it);
  its 48 KiB is far under the memory margin. A unit no workload reaches does not read exactly "the
  same".
- **A workload that never runs a unit's code** reads "about as fast" — said on the row; revisit with
  the features probe telling which units a workload reaches.
- **Behaves-differently is not a verify check**: scenarios take only the oracle's samples; revisit a
  file-input scenario so the finding becomes one. Output over 64 MiB cannot be compared, and output
  written to files is never compared [c116, n34].
- **Multi-process programs** cannot be measured (the profile kills a fork). A program that re-execs
  itself: memory reads only its last image.
- **Several cores**: cycles, instructions and CPU time add up all threads on macOS and Linux; the
  words compare total CPU work and say so when a side uses several cores [n17].
- **Noise**: memory rises with load; the same code can read "about as fast" one day and "can't
  tell" the next; at the default 15 runs on a busy Mac most rows ask for 31 runs (§2); two matching
  C runs do not prove the C stable; the efficiency-core assumption (§3.8); kernel instructions under
  preemption (0.2–1.5 % at load 14–52) are in the headline; the executable's path differs per side;
  context switches are recorded but never worded.
- **One kind of computer**: results compare only on the same kind (OS build, arch, CPU); history
  only through git.
- **Orphan after an outside SIGKILL**: if something outside the harness SIGKILLs both the harness
  and perfrun while a program runs, the program runs on in its sandbox until it ends (§3.12) [n2].
- **Security**: committed results can be forged; run temp dirs under `TMPDIR`; a crate's `build.rs`
  and the cargo configuration (the target root's and its parents' `.cargo/config.toml`,
  `$CARGO_HOME/config.toml`) are outside the crate digest, as for verify — they can change the
  release profile, `rustflags` or the compiler wrapper (results say "as each crate's own profile
  sets it") [c33]; every `cc` call other than the launcher's still finds its compiler through
  `xcrun`'s per-user cache, which tool profiles can write — a verify residual today, outside perf
  [m1].
- **Linux** (`--allow-unsandboxed`): no sandbox — the program can fork (its children counted by
  `inherit` only when they exit; a grandchild that calls `setsid()` outlives the run, since
  `PR_SET_PDEATHSIG` covers only the direct child and the group kill misses a new session),
  signal, rewrite binaries; the record's protection rests on `PR_SET_DUMPABLE`; counts are
  user-mode only (a syscall-heavy change can read "about as fast"); CPU time, switches and
  `ru_maxrss` include the moments before exec; hybrid CPUs' per-type counts are summed and a run
  whose events were not running all the time has no time words — **all unchecked here** (no Linux
  machine) [c45, c46, c117].
- **Cost**, the estimate the dialog shows [c112, c113, n4, n21, n32, m29]: per `perf run` — the
  launcher build only when its cache is stale; one compile of the C; per unit one crate build (a
  few seconds when warm; minutes when its `target/` is cold — the dialog then says "plus building
  u002's Rust, which may take minutes") and one link; the as-it-stands link (two or more units);
  first execs of each new binary — every side each run (`.perf/` is rebuilt), and perfrun and
  perfgo when their cache is new — 0.25–14 s each on a loaded Mac, none on Linux. Runs per
  workload: the C alone 2 + n; each row 3 + 2n; rows = units + 1 when there are two or more units.
  Seconds: runs × (the C's median clock time from its last stored rows, or 2 × its CPU time when
  only that is known, + 0.1 s) + 10 s per new binary + the builds — all under the ledger's lock.
  For a 1.2 s C (clock 1.3 s) at n = 15: the C alone 17 runs ≈ 24 s + 10 s ≈ 34 s; a row 33 runs ≈
  46 s + 10 s ≈ 56 s; at n = 31 a row 65 runs ≈ 91 s + 10 s ≈ 101 s ("measure again with more runs"
  says so). Before the C was ever measured: "the C's time is not known yet: about 2 + n runs of your
  program, then 3 + 2n for each row, per workload". A too-short row stops after its 3 step-1 runs.

## 7. What changed from revision 3

perfrun's own timeout judged as a timeout (never a crash or behaves-differently), and step-1 runs
given the first-exec allowance in perfrun's deadline; the ack — the program starts only after the
harness registered its group, a dead perfrun ends perfgo before the program runs; the child group's
registration dropped when perfrun's record is read; one kqueue wait for every event, registered
before *go*; the `Command` dropped after the spawn; the launcher's compiler found through root-owned
paths, not `xcrun`'s cache, the cache locked and refused under a temp-root home; raw cycles from the
share rule always read busy; "probably" at any n; "more runs" from the projected interval; the
spread as the estimate's ±; memory's near-the-line narrowed and its gap closed; the too-short rule
with a CPU leg and the smaller side's factor; the start-up note by its absolute size; dropped runs
with the exact (m, n) null; outward rounding; short forms that fit, for every outcome, with "·
parallel"; C-side outcomes only under the original C; the panic runtime read from the archives and
the mixed-panic path in words; lto's link failure on the heading; one rule for set-up outcomes with
`last_try`; `left_out`, the plan order and the C-alone compilers in currency; `failed_run`,
`copied_from`, exit and load fields; the cockpit's cached input hashing and its pause while
measuring; behaves-differently kept until a same-output re-measure; the next step with the cockpit's
own labels for model-made and hand-written crates; the MCP fact as closed values; `perf show` via
`perfrun facts`; the words in `harness-core`; §2's answer rates re-derived; the cost example with
first execs.

## 8. Not decided here (for the person)

None.

## 9. Review record

- **The draft (0b64bba)**: 4 lenses (measurement, security, integration, the person), 49 findings,
  46 confirmed by two verifiers, 2 by one, 1 refuted (`wf_fe7fb255-d44`; `perf/design-review-result.json`).
- **Revision 1 (00f2e23)** checked (`wf_0f90338d-b1d`; `perf/revision1-check.json`): of the draft's
  findings 7 resolved, 41 in part, 1 made worse; 33 new, all confirmed by two verifiers, 1
  disputed, 3 refuted. The 118 changes: `perf/rev2-changes.md`.
- **Revision 2 (cd1f79b)** checked (`wf_89198fcc-3e6`; `perf/revision2-check.json`, digest
  `perf/rev2-digest.md`): of the 118 changes 36 done, 73 in part, 2 not done, 7 wrong; 35 new
  findings confirmed by two verifiers (two high), 1 disputed (the order of work — taken), none
  refuted.
- **Revision 3 (1b27d10)** checked (`wf_d21b13f6-ea1`; `perf/revision3-check.json`, digest
  `perf/rev3-digest.md`): of the open items 74 done, 43 in part, 1 wrong (the short forms' width);
  30 new findings confirmed by two verifiers (one high: perfrun's own timeout read as a crash), 1
  confirmed by one (the window before the harness registers the program's group — taken: the ack),
  none refuted.
- **Choices made where the reviewers gave options**, all rounds: the trampoline (over ptrace or a
  kqueue hold); the launcher's private cache outside the target and the temp roots (over a folder
  in `.perf/`, which a temp-root target exposes, and over refusing such targets, which every test
  is), its compiler found through root-owned paths (over trusting `xcrun`'s cache or stating it as a
  residual); the hash as a check after the build (option a); four-character slots, 1-based; the 1e9
  instruction floor with a 0.5 s CPU leg (over a cycles floor or a measured start-up: instructions
  are the steadiest count, and the start-up is named instead); short runs marked and never "about
  as fast"; 15 runs by default (twice 7's cost on a quiet machine, where 7 already answers; under
  load, at the 3 % spread of normalised cycles, the exact interval is about ±3.8 % at n = 7, ±2.3 %
  at 15 and ±1.6 % at 31); the metric from three quarters of the runs (over half: half answered 20.7
  % of recorded windows against 19.5 %, but lets more efficiency-core runs into a normalised row,
  each 2–16 % high); raw cycles from the share rule always "busy" (over worded raw intervals, which
  called identical programs slower); memory by interval with a scaled margin (over ranges only);
  CPU seconds derived from the headline (over medians, which contradicted it); no warm-up (step 1 is
  the first exec, with the allowance in its deadline; over a suspended spawn to warm each binary);
  the original C as a `baseline` row (over no row; the draft's C-only floor point became its
  too-short check); C, other, C per row (over a shared pair with a fresh C run per side); no
  recovery (over recovering and saying so); no per-side start-up program (over a stub per side); no
  dead-strip (over dead-stripping: the results must be verify's programs); the instruction margin
  1.5 % (over one shared near-line wording at 1 %); `promotion_marker` made public (over a
  `unit_report` variant ignoring the caller's lock); the panic runtime from the archives (over the
  manifest); the ack before *go* (over a SIGTERM handler alone, which a SIGKILL of perfrun defeats);
  exit 1 for a missing, empty or broken workloads file; the Linux hybrid case summed and marked
  unchecked (over per-type words); the caps of the draft's integration review (≤ 16 workloads, 64
  KiB, ids ≤ 24, args ≤ 8 × 256 bytes) kept. Not taken: a fresh TMPDIR for every crate build and
  `RLIMIT_NPROC` (the launcher no longer builds where a crate build can write; nothing else in perf
  is trusted after its hash) [c25, c118].
