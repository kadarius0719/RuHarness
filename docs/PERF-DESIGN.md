# C-vs-Rust performance baselines (design)

Status: **revision 5 — after the check of revision 4 (f7a4905); its check (no high finding) answered by §10's build notes — to be built** (2026-10-01).
History: the draft (0b64bba) had an adversarial review (4 lenses, 49 findings, 46 confirmed by two
verifiers); revision 1 (00f2e23) answered them; its check gave 118 changes (scratchpad
`perf/rev2-changes.md`, cited [c12]); revision 2 (cd1f79b) took them; its check gave 35 new
findings (`perf/rev2-digest.md`, cited [n12]); revision 3 (1b27d10) answered those; its check gave
30 more (`perf/rev3-digest.md`, cited [m17]); revision 4 (f7a4905) answered those; its check
(`wf_8aea486a-f12`, `perf/revision4-check.json`, digest `perf/rev4-digest.md`) found 38 of the open
items done, 35 in part, 2 wrong, and 31 new findings confirmed by two verifiers (cited [p1]), 2
by one, 1 refuted. This revision answers each. The §15 spike: DECISIONS.md "2026-09-30 — C-vs-Rust
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
model and accepted, or written by hand and Re-checked) — because that is the program the person
ships; an attempt not in use has no verdict to stand on. To measure a new attempt, it must first
become the crate in use (§3.11) [c91, m18].

Each comparison runs the C and the other side alternately. The answer is one sentence about time —
"about as fast as the C (within 2 %)", "slower by 6.2 % (4.1–8.3 %)", "about 2.3 % slower
(0.6–3.4 %) — too close to the 2 % line to call", "can't tell — 9 of the 30 runs ran mostly on the
slower cores" — then the details: CPU time, instructions and memory.

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

**Checked by the review and the checks** (scratchpad `perf/`, `design-review/`, `design-check/`
to `design-check4/`; other agents loaded the machine throughout; the load average is given with
each figure) [c3–c7]:
- **Counting the program alone under the sandbox** (`perf/premise3/`, load 8–14): a harness-owned
  launcher outside the sandbox forks; the child execs `sandbox-exec` with a profile that allows exec
  of exactly a harness-owned **trampoline** and the program, and denies fork; the trampoline signals
  "ready" and waits; the launcher reads `proc_pid_rusage(RUSAGE_INFO_V6)` (the **baseline**:
  sandbox-exec's set-up and the trampoline's start, 7.4–7.8e7 instructions, 12–16 ms of CPU at load
  ≈ 10) and says "go"; the trampoline execs the program **with argv[0] = the program's bare name**.
  Subtracted at the end: an empty program 1.04–1.24e7 instructions, a 1e8-iteration loop
  6.13–6.16e8 with 0, 50 or 200 ms of delay before the baseline read — the read is exact. Identical
  binaries: kernel work under preemption moves instruction counts by 0.2 % at load 14, 1–1.5 % at
  load 37–52, up to 6.5 % on a low-IPC loop. (`sandbox-exec` alone cannot set argv[0].)
- **The launcher's mechanisms, run** (`design-check3/`, `design-check4/`, load 4.5–12.5): the
  child's `setsid()` makes the program's own `setsid`/`setpgid` fail (EPERM); perfrun killing the
  child's group survives to write a timeout record; SIGTERM and the reader's close each reach the
  program within 5 ms; a 4-thread program is not judged crashed; `pti_csw` at *ready* and
  `ru_nivcsw` at the end are one running total across exec; killing perfrun before the harness's
  go-ahead leaves the program never run [c37]; perfrun SIGKILLed after *go* left the program
  running under launchd until something killed its group [p3].
- **Time under load**: cycles mix performance- and efficiency-core time; normalising to the
  performance-core cost per instruction cuts zopfli's per-run spread from 6–8 % to 2.3–3.3 % (load
  13–53; zopfli has one phase: its P cost per instruction stays at 0.26–0.29). A program whose
  phases differ in speed (a shuffle, then a memory-bound chase) breaks the assumption: its
  normalised cycles read identical binaries up to 13 % apart [p11]. The cost per instruction rises
  on runs mostly off the performance cores (runs with under half their cycles there read 2–16 %
  high). **Answer rates** of the interval rule, replayed on the recorded identical-program rows
  (load 8–53): windows of 15 runs, 26 of 64 answer (11 fall to the busy words); windows of 31,
  22 of 25 answer; at load ≥ 13 windows of 15 answer 4 of 15. On a quiet machine (spread ≤ 0.75 %)
  seven runs answer about 98 % of rows [m11]. Whether a fresh 31-run measurement will answer cannot
  be told from one row's interval (the projection tried in revision 4 split rows at random) [p9].
- **Memory** (load 13–53): the lifetime maximum footprint clusters (≈ 12.7 / 14.7 MB on zopfli;
  12.9–14.2 against 13.9–14.9 MB in another pair) and its minimum is not stable under load; an
  interval with a margin of max(5 %, 1 MiB) gave no false difference in 70+ rows, but an interval
  one margin wide that excludes 0 still comes from identical code [p21]. The footprint starts over
  at exec: recorded as is, never minus the baseline [c5].
- **Rust's fixed start-up** (a unit linked, never called; load 4.5–6 and 24–27): a std-using unit
  +1.04e7 instructions, +2.8e6 cycles, +3 pages (48 KiB); a no-std unit adds nothing [c4, c31].
- **Linking** (macOS, rustc 1.94.1, checked): two Rust staticlibs from one rustc link into one C
  program (ld64 rescans archives); with different panic strategies the first archive's runtime
  silently wins. A staticlib's archive names its panic runtime in one member (`panic_abort-…` or
  `panic_unwind-…`; under thin LTO the name gains a prefix); a crate built with fat LTO, or a
  `#![no_std]` crate, has neither. A fat-LTO unit links alone but not beside another std-using unit;
  a no-std unit beside a std unit fails in both orders (duplicate `rust_begin_unwind`); a thin-LTO
  unit links first, not second; two units exporting one `#[no_mangle]` name fail together. Cargo
  ignores `lto` for a crate with `crate-type = ["staticlib", "rlib"]` (the harness's template).
  **Unchecked here** (no Linux machine): GNU ld scans each archive once (`--start-group …
  --end-group` needed), and GNU ar's long-name table [c6, c22, p23].
- **First execs** (macOS): a newly written binary's first exec waits on the system's check — 0.25 s
  at load 4–6, 2.6–2.9 s at load 6–7, 7–14 s at load 9–11 (a new launcher's perfrun and perfgo
  together 16.4–18.9 s at load 9–11.5). The wait is off-CPU and comes **after** the exec starts —
  for the program, after *go* [c7, n4].
- **The launcher's build**: `/usr/bin/cc` is an `xcrun` shim that finds the real compiler through a
  per-user cache under `/var/folders`, which every tool profile may write [m1]; a same-user process
  swapped an object after a passed hash in 23 of 30 tries. The build is a trust boundary.
- **Process control**: perfrun's forked child stays in perfrun's group unless moved [n3];
  `RLIMIT_CPU` on macOS only sends SIGXCPU [n2]; a failed exec after "go" exits 127 silently [n1];
  `rusage_info_v6` has no switch counts [n8]; Rust's `Command` keeps its copy of a pipe end until
  dropped [c40]; safe Rust can give a child only descriptors 0–2 (`pre_exec` is `unsafe`) [p1];
  a relative `EVFILT_TIMER` counts from when it is added [disputed 1].

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
  project through a linked folder — copy the file in"; "… leads into .git through a linked folder
  — copy the file in"; "… is a folder (or a pipe, or a device) — name a file"; "… is over 64 MiB —
  use a smaller input"; "… is under migration/ — move it"; "… cannot be read (permission denied) —
  fix its permissions". Never stored over an earlier row (§3.7).
- **Written** with `harness perf init` (a starter) or the cockpit's **Write / Edit your workloads
  file**: the features Edit flow generalised — its own draft slot (`harness-tui-workloads`),
  "Continue my workloads draft", `harness perf save --expect … --bytes …` with the 64 KiB cap.
  Saving checks the text only; the input's existence and size are checked when perf reads it [c12,
  c16].
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
Accept: `status::promotion_marker` (made public) is called directly after the lock is taken (a
legacy `.<crate>.prev` the same). The words — here and in the cockpit's `Cause::PromotionInterrupted`
(harness-tui `files.rs`), changed to match, with a test — are: "an Accept of a-1234 was interrupted
— Re-check u001 (or run harness verify u001) to finish or undo it; Measure does not". The plan's
`replaces` must equal the verdict's `inputs.replaces` ("its replaced files changed since verify —
Re-check it"). After perf builds a unit's crate, its `unit_crate_file_set_hash` must equal the
verdict's `inputs.rust_crate` ("its Rust changed since verify — Re-check it"). Every stale reason of
`unit_report` has its words. **Pending and blocked units are never measured, recorded or named.**

**Each unit's archive is read** (a small `ar` reader beside `objsyms`, BSD long names; GNU's `//`
table unchecked) for three facts [p23]: its panic runtime — **aborts** (a member whose name
contains `panic_abort-`), **unwinds** (`panic_unwind-`), or **none found**; whether it carries std (a
member whose name contains `std-`); and from those its kind: ordinary, **no-std** (no std member),
or **fat LTO** (std code merged into the crate's own member: no std member, but std symbols defined
— read with `objsyms`).

**Sides** [c19–c22, n27, m19, m22, p23, p25]:
- the **C**: every top-level `.c` of `source_dir` (a non-regular one refused by name), linked with
  `extra_link_args`;
- **a unit's program**: less the unit's `replaces`, plus its staticlib. A shared helper checks the
  `replaces` entries: a mismatch, a crate that does not build, or a program that does not link is
  that unit's outcome (`replaces-mismatch`, `crate-does-not-build`, `does-not-link`), and the next
  unit is measured;
- **the program as it stands**: with two or more measurable units, every one's staticlib in plan
  order (`--start-group … --end-group` on GNU ld). Before linking, the archives decide what perf can
  say in plain words: a **no-std** unit beside a std unit → `does-not-link`, cause `no-std` ("u003
  uses no std — it cannot be linked beside units that use std"); a **fat-LTO** unit beside another
  std unit → `does-not-link`, cause `lto` ("u002 is built with lto — it cannot be linked beside
  another Rust unit; set lto = false in its Cargo.toml and run harness verify u002"); the panic
  runtimes of units whose runtime was found differ → **mixed-panic** ("u001 unwinds; u002 aborts —
  the program as it stands would silently use one. Add [profile.release] panic = "abort" to u001's
  Cargo.toml, then run harness verify u001": an edit the cockpit cannot make); a unit whose runtime
  is **none found** is never part of a mixed-panic finding. A link that fails otherwise is
  `does-not-link`, cause `unknown`, worded with the linker's first lines (a thin-LTO unit links
  only first: the plan's order decides; two units exporting one name). Cargo ignores `lto` for a
  template-shaped crate, said in §6.
- The as-it-stands row records the units it holds (`units: [{id, crate}]`, plan order) and those
  left out (`left_out: [{id, crate, reason}]`, only units whose status is verified or merged; reason
  from the closed set: not-fresh, replaces-mismatch, crate-does-not-build, does-not-link). **Currency
  is judged per unit** [p25]: a held unit not fresh-green today → "u-x is left out now"; a fresh-green
  unit in neither list → "u-x was accepted since"; a left-out unit whose reason was not-fresh and
  that is fresh-green today → "u-x was verified since"; a left-out unit whose crate digest changed →
  "u-x's Rust changed since"; a changed order → "the plan's order changed". The CLI names the left-
  out units on every run ("the program as it stands — u001, u002 (u-tree left out: its crate does
  not build)"). Fewer than two measurable units: no as-it-stands row ("one unit measured (u001) —
  u-tree left out: verify it first", or "no accepted unit to compare yet").

**Build** — "as verify builds them: C -O2 -ffp-contract=off, Rust release as each crate's own
profile sets it (verify builds the same)", said in every result; a unit whose archive shows fat
LTO says so on its row [c33]; no dead-strip (the results must be verify's programs); Rust's fixed
start-up is part of what is measured (§6) [c31, c33]:
1. **The launcher and trampoline** (§3.3) are built from their embedded sources into a private
   cache outside the target: `~/Library/Caches/ruharness/perf/` on macOS, `$XDG_CACHE_HOME/
   ruharness/perf/` (else `~/.cache/…`) on Linux; a 0700 folder `<PERF_LAUNCHER>-<blake3 of the
   sources and the compiler's version>`, made by a helper that refuses links. **perf refuses when the
   canonical cache folder lies under any tool profile's write root** — `/private/tmp`,
   `/private/var/folders`, or the canonical `TMPDIR` — and never merely because `TMPDIR` is under
   `/var/folders`, as it is on every Mac [p4, c24]: "your TMPDIR holds perf's launcher folder, which
   every build tool may write — set TMPDIR elsewhere" (or "your home folder is inside a temporary
   folder …"). **Locking** [c24, n4]: each perf run holds a shared `flock` on its version folder's
   `.lock` for the whole run, and an exclusive one on `…/perf/.lock` only while building or checking;
   a stale version folder is removed only when an exclusive lock on its own `.lock` succeeds (no
   run holds it). Both binaries' blake3 are written to `<folder>/hashes` at the build and checked
   against it (and the binaries) before each run. **The compiler is found without `xcrun`'s per-user
   cache** [m1, p8]: the developer folder is the target of the root-owned link
   `/var/db/xcode_select_link` (else `/Library/Developer/CommandLineTools`); clang, ld,
   `libLTO.dylib` and the SDK folder under it must each have every path component owned by root with
   the "other" write bit clear (`/Applications`' group admin accepted: no tool profile can write
   there); clang is run with `-isysroot` set to that SDK and links with the `ld` beside it — settled
   with a test in step (a): a process under a crate build's tool profile that rewrites `xcrun`'s
   cache does not change the compiler. A refusal names the path: "your compiler at … is not owned by
   the system — install Xcode or the Command Line Tools with Apple's installer". Linux:
   `/usr/bin/cc` (unchecked). `cc -c` then a link, `TMPDIR` a fresh 0700 folder in the cache.
   The cache is kept between runs; the build is trusted because its compiler is found through
   root-owned paths and it runs before any target code is built [c25].
2. `whole_cc_into(base, link_args, runner, out, inputs)`, shared with `verify`; the C objects
   compiled once into `.perf/obj/`, each side one link into its slot; every object, staticlib and
   binary hashed after its build and checked before the first run [c26]. A C that does not build is
   a set-up failure with the compiler's first lines (and "the program needs one main()" when the link
   says so) [c27].
3. each step's write set is its own folder (compiles `.perf/obj`, each link its slot, cargo the
   crate's `target/` and `Cargo.lock`), and `.perf/` is made by a `scratch_dir`-style helper that
   refuses links [c23]. Each tool's process group is killed when the tool ends.

**Slots** [c28, n33]: `.perf/bin/<slot>/<name>`, `<name>` the target's program name, `<slot>` four
characters: `p000` the C, `p001`…`p999` the unit at plan position 1…999, `pall` the program as it
stands (a plan of more than 999 units is refused by name). Every side runs by its full path from a
slot of the same width, with **argv[0] = `<name>`**, in the run's own working folder `<tmp>/run`
[n34]. What still differs per side: the executable's own path [c29, c30].

Not measured: the unit's differential driver — a revisit item. A per-side do-nothing program
measuring Rust's start-up per unit [c32] is not built (a link and 2n runs a side for a share the
instruction margin covers); the start-up is named on the rows it can explain (§3.8).

### 3.3 The launcher (`perfrun`) and the trampoline (`perfgo`) — harness-owned C

`crates/harness-oracle/src/perf/{perfrun,perfgo}.c`, embedded with `include_str!`; a version
constant `PERF_LAUNCHER` in `harness-core`, pinned to the sources by a test. The launcher is the one
harness-built binary that runs unsandboxed; the program runs only inside the sandbox. On a platform
other than macOS and Linux, `perf run` refuses by name [c44]. Linux mechanisms below are from
documentation and **unchecked here** [c111].

`perfrun run PROFILE PERFGO DEADLINE PROGRAM NAME ARGS…` and `perfrun facts`:
1. **Its descriptors** [p1, c40]: stdin (fd 0) is one end of a **control socket** — a Unix socket
   pair the harness makes (`std::os::unix::net::UnixStream::pair`, the end handed over as `OwnedFd`
   through `Stdio::from`); perfrun writes the child line and the record on it and reads the
   harness's two bytes (*go-ahead*, *bye*) from it. stdout and stderr are the run's capture pipes
   (step 1 runs) or `/dev/null` (timed runs), and the child inherits them. The harness drops
   perfrun's `Command` right after the spawn. On Linux perfrun first sets `prctl(PR_SET_DUMPABLE,
   0)` on itself [c44, n6].
2. Three pipes (*ready*, *go*, *exec-status*); signals: perfrun sets SIGTERM and SIGPIPE to
   `SIG_IGN` (they are watched in step 5); `fork`. The child: restores `SIG_DFL` for every signal
   perfrun changed and an empty signal mask [p5]; `setsid()` — it leads a new session and group
   whose id is its own pid, so the program cannot leave it [n3]; stdin `/dev/null`; every other
   descriptor closed but the three pipes and stdout/stderr (bounded: `close_range`/`PROC_PIDLISTFDS`);
   then `execv("/usr/bin/sandbox-exec", ["-p", PROFILE, PERFGO, ready, go, status, PROGRAM, NAME,
   ARGS…])`. On Linux it execs PERFGO directly, after `PR_SET_PDEATHSIG(SIGKILL)` and a `getppid()`
   check. Right after the fork perfrun writes `child <pid>` on the control socket.
3. **PERFGO** (inside the sandbox): marks *status* close-on-exec, writes one byte on *ready* (if
   that fails it exits 125), reads one on *go* — **end-of-file instead of a byte means perfrun is
   gone: it exits 125 and the program never runs** — closes both, and `execv(PROGRAM, [NAME,
   ARGS…])`; if the exec fails it writes `errno` on *status* and exits 127. No CPU-time limit [n2].
4. **One wait for everything before *go*** [p5, disputed 1]: perfrun adds each watch on its own
   (`EV_RECEIPT`; Linux: one `epoll_ctl` each) — the child's exit (`EVFILT_PROC`/`NOTE_EXIT`;
   `ESRCH` means it already exited: skip to step 6 with the other watches kept), SIGTERM
   (`EVFILT_SIGNAL`), the control socket's end-of-file and its readable bytes, and *ready*. On
   *ready*: macOS `proc_pid_rusage(child, RUSAGE_INFO_V6)` — the baseline: instructions, cycles,
   performance-core instructions and cycles **when the computer has two kinds of cores**
   (`hw.nperflevels` ≥ 2, read once with `sysctlbyname`; otherwise the P fields are recorded
   absent) [m10], user and system time;
   Linux: the counters opened on the child before it was released (`perf_event_open`, user-mode
   instructions and cycles, `inherit = 1`, one event per core type summed, `TOTAL_TIME_RUNNING`
   summed against `TOTAL_TIME_ENABLED`: below it → no time words; all zero → no counter) [c34–c36,
   c45, c46]. perfrun then waits, in the same wait, for the harness's **go-ahead** byte (it has
   registered the child's group; §3.3 *The harness side*) — end-of-file instead (the harness is gone
   or cancelled) → perfrun kills the child's group, reaps it and exits. A SIGTERM at any point before
   *go* → the same, with a `stopped` record. Then *go*; then *status* is read: end-of-file means the
   exec happened, bytes mean "the program never started (exec failed: <errno>)" [n1]. **The deadline
   timer is added at *go*** (DEADLINE seconds), and the monotonic clock starts.
5. **perfrun owns the program** [c37, n3, n11]: deadline, SIGTERM, or the control socket's end-of-file
   (macOS kqueue `EV_EOF` on the socket; Linux `EPOLLRDHUP`/`EPOLLHUP`) → `SIGKILL` to the child's
   group (`-pid`), record `timeout` or `stopped`. perfrun is not in that group, so it survives to
   write the record. On Linux perfrun also kills the group after the child exits normally (forked
   members may remain) [c117].
6. `waitid(P_PID, child, WEXITED | WNOWAIT)` — the child is a zombie, its pid reserved — then, on
   macOS, the end counters from `proc_pid_rusage` on the zombie (same fields, plus
   `ri_lifetime_max_phys_footprint` and `ri_child_*`). Recorded as **end minus baseline**:
   instructions, cycles, P-core instructions and cycles, CPU time (user + system, `cpu_us`) [c39].
   Recorded **as is**: the memory (the lifetime maximum footprint, which starts over at exec); wall
   from *go* to `waitid`. **No context switches on macOS**: `rusage_info_v6` has none, the zombie's
   task info is gone, and the only end count comes from reaping; they are never worded [n8]. On
   Linux the end counts need `wait4`, which reaps here: CPU time, voluntary and involuntary
   switches and `ru_maxrss` × 1024 in bytes, each including the moments before exec [n10] — the
   pid is free from this point on Linux (§6, unchecked).
7. The record: one `write()` on the control socket, `key value` lines, ASCII, at most 4 KiB, last
   line `end`: status (`ok`, `timeout`, `stopped`, `never-started <errno>`, or the launcher's failure
   in words), the program's exit code or signal, whether perfrun sent the kill, the counters, the
   facts (below), the 1-minute load average in hundredths. **Then perfrun waits for the harness's
   *bye* byte (or end-of-file), and only then reaps the child (macOS) and exits 0** — so the child's
   pid stays reserved while the harness can still signal its group [m2]. The **facts** lines — the
   CPU's name (`machdep.cpu.brand_string`), the OS product version and build
   (`kern.osproductversion`, `kern.osversion`), the arch (`hw.machine`), the number of performance
   cores (`hw.perflevel0.logicalcpu`) and whether there are two kinds of cores; Linux: the harness
   reads `/proc/cpuinfo` and `/etc/os-release` itself — are also all that `perfrun facts` writes
   (then `end`; no child line, no bye) [p29].
   **The harness reads it strictly** [c40]: at most 4 KiB + 1 beyond the child line, the `child`
   line first (`run` mode only), then each known key once, every number bounded, unknown keys and a
   second record refused, `end` last.

**Judging a run**, in this order [c41–c43, n1, n9, m17, p6]:
1. The harness's own end first: its timeout → `run-failed: timeout` (the C in step 1:
   `c-timed-out`); cancellation → no row. Output over the cap: on the C → `output-too-large`; on
   the other side alone → **behaves-differently** ("prints more than 64 MiB where the C prints
   2.1 MB") [c41].
2. perfrun did not exit 0, no record, no `end`, over 4 KiB, a malformed line → **unmeasurable**
   "the launcher stopped before measuring" (that row only).
3. The record says `timeout` → `run-failed: timeout` (the C: `c-timed-out`); `stopped` → no row.
4. **No *ready*, or the record's status is `never-started`** → "the program never started" with the
   reason (`c-could-not-start` on the C, `could-not-start` on the other side). An exit code is never
   read as "never started" once the status pipe closed on a successful exec: a program that runs and
   exits 65, 71 or 125 is judged by its own end [p6].
5. macOS only: a `SIGKILL` perfrun did not send → **stopped-by-sigkill**, worded without asserting
   the cause [n9].
6. Otherwise the program's own end (exit code or signal) and its counters. A zero baseline is
   never subtracted.

**The harness side** [c38, c27, n3, n4, m2, m17, p2, p3, p7]: `run_measured(launcher, socket,
run_dir, input, collect)`:
1. Under the `LIVE` registry's lock (as every spawn): spawn perfrun and register its group; drop the
   lock and the `Command`.
2. **Outside the lock**, read the `child <pid>` line (bounded: 64 bytes, within the step-1
   allowance). Ctrl-C is never held up by this read [p2].
3. Retake the lock, check `cancelled()`: cancelled → close the socket without the go-ahead (perfrun
   kills the child and exits; a dead perfrun ends perfgo; the program never runs); otherwise register
   the child's group and write the **go-ahead**; drop the lock.
4. Read the record (to `end`), then **unregister the child's group, then write *bye*** — the child is
   reaped only after that, so a cancel never signals a free pid [m2]. Then reap perfrun.
5. **perfrun ends without a complete record** (a signal it could not handle, a crash, a cut record)
   → the harness SIGKILLs the child's group at once (its pid is still reserved: perfrun never reaped
   it, and launchd reaps orphans only after perfrun's death — the one window, between perfrun's
   death and this kill, is named in §6), then unregisters it [p3].
6. Deadlines: perfrun's DEADLINE is the target's `[oracle] timeout_secs` for timed runs and
   `timeout_secs` + 60 s for step-1 runs (every new binary's first exec falls in step 1, after
   *go*) [c7, n4, m17]. The harness's own timer starts at the go-ahead and is DEADLINE + 15 s; before
   the go-ahead its bound is the step-1 allowance — so perfrun's deadline fires first and its record
   says `timeout`; when the harness's fires (perfrun stalled), both read as a timeout [p7]. On the
   harness's timeout, overflow or cancellation: SIGTERM to perfrun, a bounded grace (250 ms), then
   SIGKILL to perfrun's group and (while registered) the program's.
7. perf's run temp dirs are 0700 and registered with the signal handler's live-folder list (as every
   run's temp folder now is).

### 3.4 The perf profile

The scenario profile (no network, no signal but to itself, reads denied under the home folder and
the target root except the side's binary, perfgo and the run's temp dir, writes only the temp dir),
with exec allowed for exactly two literals — perfgo and the side's program — and its fork rule
`(deny process-fork (with send-signal SIGKILL))` [c42, c94]. A run cannot read the other side's
binary. A program may exec **itself**: counts carry on across the exec, the memory reads only the
last image (§6) [c34].

### 3.5 Running a row

**Per side against the C, per workload** [c48–c56, n20, n25, p13, p26]:
1. **Same end and output**: C, other, C — for every row, with its own two C runs: streams captured,
   rewritten as a scenario's. The two C runs differ → **c-unstable**. A C-side failure (crashed,
   timed out, over the output cap — "prints more than 64 MiB; give it an output-file argument — then
   perf times it but no longer compares its output" — could not start) is the C's: **it is stored
   only on the workload's `c_alone` row** (§3.6), the workload's remaining rows are not run, and
   each keeps its earlier row with `last_try: {outcome: c-…}` (§3.7) [p26]. Otherwise the other side
   differs in its end or its streams → **behaves-differently**. A non-zero C exit is named on the
   row from its stored exit code and in the progress line; when both exit non-zero, both codes [c51].
2. **Too short and short**, decided per row from step 1 (the C's numbers from its step-1 run with
   fewer instructions) with a floor of **1e9 instructions and 0.5 s of CPU** [c53, c54, m6, p13,
   p19]:
   - **both sides under both** → **too-short**, quoting both sides' step-1 CPU times and the fix:
     "too short to time: the C ran 40 ms of CPU and the Rust 360 ms on small-text — use an input at
     least about 7× bigger (13× for half a second)": the first factor is the floor over the smaller
     side's instructions, the second half a second over the C's CPU time, each rounded up to one
     decimal below 2 (1.1×) and to a whole number above; "check the workload's options first" when
     the C wrote nothing or exited non-zero [c98];
   - **either side under both** (and not too-short) → measured, marked **short run**: never "about
     as fast"; inside the margin "can't tell on a run this short — use a bigger input";
   - otherwise measured in full.
   The memory line is kept whenever either side's median footprint is over 4 MiB (an empty program's
   is under 1 MiB), short or not [p13]. Step 1's streams go through pipes (≈ 4–5 % more work); the
   floor is judged on them, the timed words on step 3.
3. **n timed runs a side, interleaved** C, other, C, other, … (n = `--runs` if given, else the
   workload's `runs`), streams to `/dev/null`. A timed run that ends differently from its side's
   step-1 runs ends the row: **run-failed** with the side, the run's index and how it ended
   (`failed_run`) [c55, c56, m28].

**The C alone, per workload** (the baseline row; §3.6): C, C (end and output; c-unstable or a C-side
failure), the floor on the C alone (too-short under both legs, with the same fix words — the day-one
warning [n27]), then n timed runs of the C.

### 3.6 The C alone [n27, p26]

Every `perf run` without `--unit` or `--as-it-stands-only` measures the C alone on each workload
first: outcome **baseline** with n runs, or a C-side outcome, or too-short. It is stored in
`program.json` under `c_alone` (§3.9), one row per workload, with `other` absent; its `compilers`
input is `cc`'s line only [m22]. **A C-side outcome found during any row's step 1 — also under
`--unit` — is written to the `c_alone` row** (beside its earlier baseline as `last_try` when there is
one), never onto a unit's or the as-it-stands row. Its words: "the C: CPU about 1.2 s here today
(varies with load) · 12.4 MB · 1.21e10 instructions". The View's "The original C" lines read these
rows, C-side outcomes shown there once. Event side `c`.

### 3.7 Outcomes (closed set) [c57, m20, p28]

- **Measured**: `baseline`, `measured` (`short: true` for a short run).
- **What the code does**: `behaves-differently`, `stopped-by-sigkill`, `too-short`, `run-failed:
  timeout | exit | signal`; on the C alone only: `c-unstable`, `c-crashed`, `c-timed-out`,
  `output-too-large`.
- **What the set-up could not do**: `not-verified: <reason>`, `replaces-mismatch`,
  `crate-does-not-build`, `does-not-link`, `mixed-panic`, `input-unusable`, `c-could-not-start`,
  `could-not-start`, `run-failed: unmeasurable`.

**One rule for what replaces a row** (cited by §3.1, §3.5 and §3.9): a set-up outcome, or a C-side
outcome on a unit's or the as-it-stands row, **never replaces an earlier row that is not itself a
set-up row**; it is kept beside it as `last_try`. A set-up outcome replaces an earlier set-up row.
With no earlier row, it is stored as the row. **A behaves-differently finding survives every
re-measure that does not end `measured` or `too-short` with the same output** (§3.11): such a
re-measure keeps the finding beside the new row as `found_before: {first_difference, kept}` [m25].

Every outcome is a result (exit 0); a set-up failure (the launcher or the C does not build, the lock
is held, the workloads file is missing, empty or has an error, an unsupported platform, a launcher
cache a build tool may write, a compiler not owned by the system) is an error (exit 1); a clap usage
error is exit 2 [c80].

### 3.8 The words (std only, in `harness-core`) [c58–c70, m26]

Per metric, the **Hodges–Lehmann shift**: the median of every pairwise log-ratio `ln(other_i /
c_j)`, its distribution-free interval `[D(c+1), D(m·n−c)]` from the sorted pairs, `c` the largest u
with `P(U ≤ u) ≤ 0.025` under the exact (m, n) Mann–Whitney null (a dynamic program in u128) for
the m and n **runs that have a value** — a run whose value is missing, not finite or not above zero
is dropped whole before any pair is formed [c60, m14]; with fewer than 5 values on either side →
**"can't tell — too few runs gave a value; measure again"** (short form `can't tell: too few`) [p20].
At m = n: c = 2, 8, 23, 64, 127, 341 at n = 5, 7, 10, 15, 20, 31; confidence 96.8, 96.2, 95.7, 95.5,
95.1, 95.0 % (computed exactly). It is at least 95 % confident at every m, n for time and
instructions; memory's clustered footprints are covered by its margin instead [n24].

**Rounding** [c69, m15, p16]: one decimal under 10 %, whole numbers above; each end of an interval
is rounded **away from the boundary its branch relies on** — the end beside 0 for "close call" and
"probably", the end beside ±M for "slower", "faster" and "about as fast" — and shows a second decimal
when one decimal would put it on that boundary (0.01 → "0.01", 2.004 → "2.01"); ends that round equal
show one number.

- **Time** (the headline). **The metric**, per row [c59, n12, m3, m10, p10, p11]:
  - **performance-core-normalised cycles** (P-core cycles per P-core instruction × all
    instructions) when the computer has two kinds of cores, at least three quarters of the row's 2n
    timed runs have at least half their cycles on the performance cores, **and** the P-core cost per
    instruction is steady across the runs — its spread (1.4826 × the median absolute deviation of
    ln(p_cycles / p_instructions)) no larger than raw cycles' spread; a run with no P-core cycles has
    no value (dropped);
  - otherwise **raw cycles**, said: "cycles" (no two kinds of cores, macOS V4, Linux); "cycles — the
    program's phases differ in speed" (the steadiness test failed);
  - **when the share rule failed** (fewer than three quarters of the runs mostly on the performance
    cores), perf computes both intervals on the same runs and **words a difference only when both
    agree** — both past +M, or both past −M (then "slower by X % (a–b %) — cycles; K of the 30 runs
    ran mostly on the slower cores", X and the interval from raw cycles); otherwise "can't tell — K of
    the 30 runs ran mostly on the slower cores", followed by the cause the record supports: "the
    computer was busy (load about 14 on 8 fast cores) — close other work and measure again" when the
    runs' median load average is at least the number of performance cores; else "the program may
    run there by design (several threads, or a low priority)" [p10];
  - no counters at all → CPU time, said. Never mixed in a row. The headline includes the kernel
    instructions preemption adds (0.2–1.5 % at load 14–52, §2). The assumption is stated: an
    efficiency-core stretch is counted at the performance-core cost.

  With margin **M = 2 %**, the interval [a, b]:
  - inside ±M → **"about as fast as the C (within 2 %)"** (never on a short run);
  - a > +M → **"slower by X % (a–b %)"**; X ≥ 100 % → **"3.7× as slow (3.4–4.0×)"**;
  - b < −M → **"faster: takes X % less time (a–b %)"**;
  - **near the line** — b − a ≤ 2M and the interval excludes 0 → **"about X % slower (a–b %) — too
    close to the 2 % line to call"** (the mirror for faster) [c61, n13];
  - otherwise, the interval excludes 0 → **"probably slower, by about X % (a–b %) — not clearly
    past the 2 % line"** (or faster);
  - otherwise (the interval holds 0): at n = 31 **"no clear difference: within ±Y %"**, Y = max(|a|,
    |b|); on a short run "can't tell on a run this short — use a bigger input"; otherwise **"can't
    tell: the estimate is ±Y %"**.
  At n < 31, every "probably", "close call" and "can't tell: the estimate …" row adds **"— measure
  again with 31 runs: <command> (about N s a row; on a busy computer it may still not tell)"**; perf
  does not predict whether 31 runs will settle it [p9, p12, p14, n15, m4]. The command is per side
  (`--unit u001`, or `--as-it-stands-only`), with the estimate of §6 for that command [n22, p31].
- **Instructions** (details; never speed words; **M = 1.5 %**): "about the same instructions (within
  1.5 %)", "X % more instructions (a–b %)", "X % fewer", near the line "about X % more instructions —
  too close to the 1.5 % line to call", otherwise "instructions: can't tell". A unit's or the program
  as it stands's row whose archives carry std, and whose interval of absolute median differences
  holds 1.04e7 and lies within 0–2e7, adds "(about Rust's fixed start-up)" [c31, m12, p17]. No
  counters → "instructions not counted" [c70].
- **Memory** (details; no counter needed): the same interval with margin **max(5 %, min(1 MiB, 20
  %))** of the C's median [n18]: inside → "about the same memory (within 8 %)"; past → "uses about X
  % more memory (a–b %)" or "less"; **every other interval** → "can't tell — memory varied from run
  to run" (no near-the-line wording for memory: clustered footprints make it unsafe) [m5, p21].
- **CPU time shown** [c67, n16, p15]: the C's median CPU time a run, labelled as varying with load
  ("CPU about 1.21 s here today"); on a row with a time answer the other side's seconds come from the
  headline — the C's × (1 + shift), "→ about 1.28 s (from the estimate)"; on a can't-tell row each
  side's own median, labelled as varying with load, with no arrow. With no counters, CPU time is the
  headline itself.
- **Several cores** [n17, m9]: when either side's median CPU time exceeds 1.2 × its median clock
  time, the row says "uses several cores: the words compare total CPU work, not waiting — clock time
  0.70 s → 0.25 s", and its short form gains "· parallel" (the interval left to the detail).
- A unit row that says "about as fast" adds "(perf cannot tell whether big-text runs this unit's
  code)" [c68].
- Not worded, with reasons: the P-core share of each run and the context switches (kept in the
  file; words about scheduling would say more than perf knows) [c69].

The CLI, per row: a headline line and detail lines, each wrapped at 80 columns with a 6-space
hanging indent:
```
perf: u001-katajainen on big-text — slower by 6.2 % (4.1–8.3 %)
      CPU about 1.21 s → about 1.28 s · 3.1 % more instructions
      · memory about the same · 15 runs each
```
The C alone: `perf: the C on big-text — CPU about 1.21 s here today · 12.4 MB · 15 runs`.

**Short forms** (the View and the unit header; the full sentence is the selected row's detail), each
at most 26 columns with the longest numbers [n30, m7, m8, m23, p18, p22, p30]:

| outcome / words | short form |
|---|---|
| about as fast | `about as fast` |
| slower / faster | `slower 6.5 % (4.1–8.9 %)`, `faster 12 % (10–14 %)`; parallel: `slower 6.5 % · parallel` |
| ≥ 2× | `3.7× as slow` |
| near the line | `close call: ≈2.3 % slower` (or faster) |
| probably | `probably slower ≈3.1 %` (or faster) |
| no clear difference | `no clear diff ±12 %` |
| can't tell | `can't tell: busy`, `can't tell: slow cores`, `can't tell: ±3.4 %`, `can't tell: too few`, `short run: can't tell` |
| too-short | `too short to time`; when step 1's CPU times differ more than 2×: `too short · Rust 9× CPU` |
| behaves-differently | by its end first: `Rust crashed` (a signal), `exits differently` (another exit code), else by stream: `prints differently`, `prints too much` (over the cap) |
| stopped-by-sigkill | `stopped by SIGKILL` |
| run-failed | `run failed: timeout`, `run failed: exit`, `run failed: signal` |
| could-not-start, set-up | `could not start`, `not measured` — the reason in the detail |
| the C alone (under The original C) | `CPU 1.21 s · 12.4 MB` (three significant figures; GB above 1 GB), `C output unstable`, `C crashed`, `C timed out`, `C output too large`, `C could not start`, `too short to time` |

### 3.9 Results: `migration/perf/program.json` and `migration/perf/units/<id>.json` (`ruharness-perf`, v1)

`program.json` holds two lists, `c_alone` and `as_it_stands`, one row per workload each; a unit's
file one list of rows. A measurement replaces only the rows it measured, by §3.7's rule. Each row
[c71–c79, n31, m20, m28, p28]:
- `workload`, `outcome`; on `baseline` and `measured` rows also `short`, `runs` (5–31) and
  `platform_metrics` (`macos-v6-pnorm`, `macos-v6-cycles`, `macos-v6-cycles-phases`,
  `macos-v6-share`, `macos-v4-cycles`, `linux-cycles`, `linux-hybrid-summed`, `cpu-time`);
- `inputs`: the workload digest, the program digest, the crate digest(s), the unit's `replaces` and
  `program_name`, `units` and `left_out` (as-it-stands rows), `recipe` (`perf-recipe-1`), `launcher`
  (`PERF_LAUNCHER`), `computer` (OS product version and build, arch, CPU, two kinds of cores and
  how many fast), `compilers` (the first line of `cc --version`, and of `rustc -V` except on
  `c_alone` rows);
- on `baseline` and `measured` rows: `c` (and `other`, not on `baseline`), arrays of exactly `runs`
  entries, each a run: `instructions`, `cycles`, `cpu_us`, `wall_us`, `memory` (bytes) — each ≥ 1
  when present; `p_instructions`, `p_cycles`, and on Linux `switches_voluntary` and
  `switches_involuntary` — each ≥ 0 when present, the P counts both present or both absent;
  `load` (hundredths, ≥ 0); `end` (`exit N`, 0–255, or `signal N`); plus the row's archive facts
  (`std`, `fat_lto`) for the start-up note and the LTO mention;
- on other outcomes: `c` and `other` absent; `step1` holds what step 1 measured, by name (`c_first`,
  `other`, `c_second`: instructions, cpu_us, `end`, stdout and stderr sizes) — absent on a set-up row
  and on `run-failed: unmeasurable`; a `run-failed` row with a timed run adds `failed_run: {side,
  index, end}` (`end` also `timeout`);
- **set-up facts**, closed values only, so the words can be rebuilt on every read [p28]: mixed-panic
  `runtimes: [{id, runtime: abort | unwind | none}]`; does-not-link `cause: no-std | lto | unknown`
  and the unit ids; replaces-mismatch the entry's index; and `log`: the name of the run's log under
  `migration/build/.perf/logs/` holding the tool's first lines (not committed, named in the words);
- `first_difference` (behaves-differently only): numbers and closed values — the stream (`stdout`,
  `stderr`, `exit`), both lengths, the byte offset, both ends; `found_before` (§3.7): the same fields
  plus the kept files' sizes and blake3;
- `last_try` (beside an earlier row): `{outcome, units, set-up facts}`.
Unit ids from the plan alphabet are allowed wherever a row names units.

**No words are stored**: they are computed by §3.8 on every read. Read strictly by outcome
(`read_regular` with a 4 MiB cap; the shapes above; counts' rules; runs 5–31; ids once; a new key is
a new `schema_version`) [c75, c79]. Written after every row with `write_atomic` into a folder the CLI
resolves with `safe_ledger_dir` and passes in [c76]; links refused by the CLI and the cockpit on read
and on write [c75]. Rows of removed workloads are dropped on the next write; a unit file of a unit no
longer in the plan reads "no longer in the plan". The results files count in the cockpit's
retained-read budget [c14].

**Kept outputs** of a behaves-differently row: `migration/build/.perf-out/program/` and
`.perf-out/units/<id>/` [n7, n33], `<workload>.{c,other}.{stdout,stderr}`, each capped at 64 MiB,
written with `write_atomic` through a helper that refuses links, in no tool profile; replaced when
that row is re-measured, removed only when the finding clears (§3.11); the row records each file's
size and blake3 [c74]. A non-zero C exit's stderr is not kept: keeping it would need the kept-
outputs folder for every row, and the person sees it by running the workload once (its size is in
`step1`; §9) [c51].

**Current** iff each input equals today's, each with its own reason (the workload, the C, the
unit's Rust, its replaced files, the program's name, the as-it-stands units per §3.2, the system,
the compilers, the kind of computer, the harness). `harness perf run` judges all. `harness perf
show` judges the computer with `perfrun facts` when the launcher cache is current, and otherwise says
"computer not checked — run harness perf run once" (it never builds the launcher); the compilers
with `cc --version` and `rustc -V` as tool runs when the target's allowlist has them, else
"compilers not checked"; `--no-check` skips both [n35, m24, p29]. The cockpit judges what it can
without starting a process and says the rest is not checked (§3.11). A results file is evidence,
not ledger truth; committing it keeps a history in git; it gates nothing [c78, c79]. Stale facts:
`perf run` refuses ("scan first").

### 3.10 The CLI [c80–c84]

- `harness perf run [--target T] [--unit ID]… [--workload ID]… [--runs N (5–31)]
  [--as-it-stands-only] [--allow-unsandboxed] [--json]` — the writer lock; without `--unit` or
  `--as-it-stands-only`, the C alone, the program as it stands and every measurable unit; `--unit`
  only those units; `--as-it-stands-only` only the program as it stands (fewer than two measurable
  units: says why) [n27, m29]. Unknown ids: exit 1 naming the known ones.
- `harness perf init [--target T]` (resolves `migration/perf` with `safe_ledger_dir`; refuses unless
  `workloads.toml` is absent by lstat) [c83]; `harness perf save [--target T] --expect … --bytes
  …`; `harness perf show [--target T] [--no-check] [--allow-unsandboxed]`.
- Progress (`message` events): "building the launcher…" (only when its cache is stale), "building
  the C program…", "u001 — building its Rust…", "the program as it stands — u001, u002 (u-tree left
  out: …)", "the C on big-text — checking it ends the same way twice…", "the C exits 1 on big-text:
  its error path is timed", "u001 on big-text — C, u001, C…", "u001 on big-text — timed run 9 of
  30…", "keep the computer quiet while it measures". A behaves-differently row is printed first and
  in full, with where both outputs are kept. Summary: "perf: measured 4 rows, 1 too short, 0 behave
  differently — wrote migration/perf/ (commit it to keep a history; perf compares what the program
  prints and how it ends)".
- Events: `perf-row {side: "c"|"program"|"unit", unit (unit rows only), workload, outcome, words}` —
  `words` a display-only courtesy (SCHEMAS says so) [c83].

### 3.11 The cockpit [c85–c92]

- A **Speed** group in the tree beside Features, labels of at most 19 columns [n30]: `Speed (no
  file)`, `Speed (none yet)`, `Speed (file error)`, `Speed (C only)`, `Speed (2 of 3)`; acts **Write
  your workloads file** / **Edit the workloads file** / **Continue my workloads draft**, **Measure
  speed** (greyed, with the state's words, in the first three states), on a unit **Measure this
  unit's speed** and **Measure again with 31 runs** (the workloads whose rows could not tell, are
  "probably" or a close call), and the same on the As-it-stands heading (`--as-it-stands-only`)
  [c16, c63, n22, m29, p14]. Each confirm dialog says what runs, how many times, the estimate (§6),
  what it writes (`migration/perf/`, scratch folders under `migration/build/`, and — when stale —
  the launcher cache in the home folder), no verdict, all under the ledger's lock, that Cancel keeps
  finished rows, and to keep the computer quiet; when it ends: "Measured 4 rows — see Speed" [c92].
- **Inputs read by the cockpit** [c14, m21, disputed 2]: to judge "your workload changed" it hashes
  each input off the UI thread in `Snapshot::load` (shared with MCP), with the same confined, bounded
  read perf uses, each digest **cached by (device, inode, size, modification time)** in a
  process-wide cache. `Snapshot::load` reads the live lock holder itself (as `unit_report` does); a
  holder whose command starts with `harness-core`'s `PERF_RUN_LOCK` (the string `perf run`, the
  command perf's lock records) is a perf run, and while one holds the lock no input is hashed — a
  cached digest is used, and an input whose (device, inode, size, mtime) changed reads "can't check
  while measuring". An input over `preflight`'s budget reads "can't check: inputs too large to hash
  here" (the read is never refused for it); the program-digest budget applies with a features **or**
  perf file. An input perf could not use reads with the CLI's own words (§3.1) [c13].
- The **Speed View** (golden at 54 columns, inside an 80-column terminal; the workload column is the
  longest workload id + 2):
  ```
  Speed — your workloads, the C against the Rust in use
  measured on Apple M3 with rustc 1.94.1 (not checked
  here) · as verify builds them · 15 runs each ·
  compares what the program prints and how it ends

  The original C
    big-text    CPU 1.21 s · 12.4 MB
    many-small  too short to time
  As it stands (2 of 3 units — 1 left out)
    big-text    slower 6.2 % (4.1–8.3 %)
    many-small  too short to time
  u001-katajainen
    big-text    slower 6.2 % (4.1–8.3 %)
    many-small  too short to time
  u002-hash
    big-text    about as fast
    many-small  too short to time
  ```
  The header is built from the rows: the computer and compilers they record (rustc cut to its
  version), "not checked here", "n runs each" or the per-row n; rows from different computers or
  compilers: "measured on 2 kinds of computer — see each row" [c78, n30]. Workloads in file order;
  units sorted worst first by their worst row, the order: behaves differently (Rust crashed, exits
  differently, prints too much, prints differently), stopped by SIGKILL, run failed, slower (by the
  shift), too short with a CPU gap over 2× (by the gap), probably slower, close call (slower), could
  not start, not measured, can't tell (busy, slow cores, too few, ±), no clear difference, short run,
  too short, about as fast, close call (faster), probably faster, faster; out-of-date rows last within
  a unit; ids cut with …; a stale row's short form dimmed with "out of date: …"; a row with a
  `last_try` adds its line [c89, m20, m23, p18]. The selected row's full sentence and details show
  below the list. The As-it-stands heading counts left-out units; their names are in its detail
  [p25].
- A unit's header: `Speed: slower 6.2 % on big-text` and, on its own line, the interval and "1 of 2
  workloads" — never past 54 columns.
- The project summary gains `Speed: 3 of 5 units measured — 1 slower, 1 about as fast, 1 parallel ·
  1 out of date — see Speed` [c90, m9].
- A **behaves-differently** row [c86–c88, n29, m25]: a fact on the unit — "With u001's Rust the
  program prints differently on big-text (stdout, byte 40 961) — verify does not run this workload"
  (or "exits differently", "the Rust crashes", "prints more than 64 MiB") — with the next step
  "Compare the outputs; then change the unit's Rust (below) and measure this unit again". No
  Re-check. **It clears only when a re-measure ends `measured` or `too-short` with the same output**;
  through anything else it stays, from the row or from `found_before`, as "found before the unit's
  Rust changed — measure this unit again to check" (or "found on Apple M3 with rustc 1.90.0 —
  measure again to check"). An as-it-stands difference goes on its heading and in the project
  summary, naming the units it holds; "no unit's Rust differs alone" only when every held unit's own
  row on that workload is current and measured the same, else the commands that would measure the
  missing ones. **Compare the outputs** shows the kept files side by side when they are regular files
  whose size and blake3 match, control characters escaped; else "the two outputs are not on this
  computer — measure again" [c73, c88].
- **A slower row's next step**, from the crate's provenance (`attempts::provenance`) and the
  cockpit's own labels [c91, m18, p24]: "perf times the Rust in use. If speed matters here — note
  these numbers first (or commit migration/perf/): measuring again replaces them —" then
  - **model-made** (pipeline, steered or chat), one attempt: "Modify a-1234 with a note about speed
    (give these numbers), then Replace u001's verified crate with the new attempt, measure this unit
    again — and if it is not faster, Replace it back with a-1234"; several attempts share the crate:
    the lowest id is named; with no provider, Modify is greyed and the words say "connect a model to
    Modify";
  - **a recorded hand edit**: "Hand edit u001, then measure again" (or, when the crate has no
    src/logic.rs and src/ffi.rs: the next branch);
  - **no recorded attempt** (written by hand outside the cockpit): "commit the unit's crate first
    (git) — replacing it deletes it; edit it in your editor, then run harness verify u001 in a
    terminal (the cockpit cannot Re-check code it did not record), then measure again".
  Help gains a Speed section, those paths, "perf stops even a fork; verify allows a fork but not
  starting another program", "perf compares what the program prints and how it ends", and the
  glossary [c90, c114, n34].
- **The MCP reads** export per unit, and for the As-it-stands heading and the C alone, a Speed fact
  of closed values and numbers [n35, m27, p27]: the row's **answer**, computed by the same
  `harness-core` words, from a closed set (about-as-fast, slower, faster, probably-slower,
  probably-faster, close-call-slower, close-call-faster, cant-tell-busy, cant-tell-slow-cores,
  cant-tell-estimate, cant-tell-too-few, no-clear-difference, short-run, too-short, and the §3.7
  outcomes), the workload id, `platform_metrics`, `short`, `runs`, the shift and interval **only when
  the answer is not a can't-tell kind**, `current` with reasons from a closed set, and
  `environment_checked: false` (computer and compilers); for the C alone: CPU time, memory and the
  outcome. Ranked by the worst order above; fenced as the features facts are.
- Acts' argv: `with_sandbox_flag(harness_argv(["perf", "run", …, target_arg]))`.

### 3.12 Security (summary) [c93, c94, n2, p3]

The program runs only under the perf profile: no fork (killed on trying), no signal out, no network,
no reads under the home folder or the target beyond its binary, perfgo and its temp dir, writes only
its temp dir. **Nothing the program starts, and not the program itself, outlives its run** on macOS
while the harness is alive or ends by its own cancel path: the program cannot fork; it leads its own
session, so it cannot leave its group; it never starts before the harness has registered that group,
and a cancel before the go-ahead ends it unstarted; perfrun kills the group on its deadline, a
SIGTERM or the harness's end; the harness kills it on timeout, on cancel, from its signal handler,
and at once when perfrun ends without a complete record; its pid stays reserved (unreaped) until the
harness has unregistered it. Named, not covered: **something outside the harness SIGKILLs both the
harness and perfrun while the program runs** — the program runs on in its sandbox until it ends (no
CPU limit holds on macOS); and the moment between perfrun's death and the harness's kill (§3.3). The
launcher is the one unsandboxed harness binary: built with a compiler found through root-owned paths
into a private cache no tool profile can write, hash-checked before each run. The counters return on
a socket the program never holds (macOS; Linux relies on `PR_SET_DUMPABLE`, unchecked). The input is
read once, confined and bounded; results carry no words and are read strictly — a committed results
file can be forged (it gates nothing). Run temp dirs under `TMPDIR` can be rewritten by a same-user
process that outlived a build. No new dependency; `unsafe` Rust stays forbidden everywhere.

## 4. Tests and checks [c95–c107, c111]

- **The words** (in `harness-core`): the critical values and confidences at m = n (n = 5, 7, 10, 15,
  20, 31) and at m ≠ n, with ±1 mutations failing; fewer than 5 values gives "too few"; a seeded
  synthetic A-vs-A at 0.5 % noise "about as fast" at every n; the interval narrows as n grows;
  **constructed samples** with known intervals for each branch — about as fast, slower, faster,
  close call, probably (e.g. [+0.5, +5.5]), no clear difference at 31, can't tell at 15 — and, over
  2 000 seeded rows of +3 % at σ 4–5 % and n = 31, "probably slower" the most common answer and never
  "faster" or "about as fast" [n14, m13]; the "31 runs" offer on every probably, close-call and
  can't-tell row at n < 31, never on a short-run row's "about as fast" [p12, p14]; on the share
  rule's rows: the recorded fp-aa7 #248 row (identical binaries) never "slower", and identical
  programs scaled ×1.5, ×2 and ×3.7 read "slower" when both metrics agree [m3]; the busy cause only
  with a median load at least the fast-core count; an 8-thread program and a background-priority
  harness read "may run there by design" [p10]; a phased program (shuffle, then chase) uses raw
  cycles and identical binaries never read slower or faster [p11]; on an Intel-style record (P fields
  absent) never the share rule [m10]; fallbacks (V4, p_instructions = 0, a self-backgrounding
  program, a stored 0) never panic; on the recorded rows the CPU seconds shown never contradict the
  headline, and a busy row shows two medians with no arrow [c96, n16, p15]; instruction intervals
  crossing ±1.5 %; the start-up note on a std unit at 1e7 difference on 4e9 and 1e9 Cs (the 1e9 case
  at 1.04e7 ± 0.3e7), never on a no-std unit and never for 0.9 % real work on a 4e9 C [m12, p17];
  rounding: an end at 0.01 beside 0 shows "0.01", at 2.004 beside M "2.01", "3–3 %" never [m15, p16];
  the CLI lines at 80 columns and every short form at 26 with the longest numbers.
- **Memory**: the same allocating binary on both sides under load never "more", "less" or any
  directional wording (every wording named in the test); a synthetic two-cluster sample; a 1 MB
  program's margin is 20 %; the recorded windows of M5 and P21 read "varied" [c97, n18, m5, p21].
- **Threads**: a 1-thread and a 4-thread side — "uses several cores" and "· parallel" [n17, m9].
- **The floor**: either side of 1e9 and 0.5 s; a memory-bound C at 2.6 s and 0.9e9 instructions is
  measured in full (not short), with its memory line [m6, p13]; both under, with both step-1 times
  and factors (1.1× below 2, never "1×") [p19]; a 9× CPU gap shown and ranked [p18].
- **The launcher and trampoline**: an exact baseline; argv[0]; a self-exec'ing program; a missing
  program and an exec the profile denies are each "never started"; **programs exiting 65, 71 and
  125 judged by their own end** [p6]; perfrun's own deadline gives `run-failed: timeout`, never a
  crash; a fresh binary with `timeout_secs = 5` is not timed out in step 1 [m17, n4]; **step 1
  captures stdout and stderr through the launcher** [p1]; a spinner that ignores SIGXCPU and SIGTERM
  is dead after timeout, cancel, overflow, **perfrun alone SIGKILLed** [p3] and the harness's signal
  handler; a cancel between perfrun's fork and the go-ahead leaves no program run, **answered within
  the CLI's 250 ms** [p2]; a SIGTERM while perfrun waits for the go-ahead gives `stopped` within the
  grace [p5]; the program sees default signal dispositions [p5]; **a child already exited when its
  exit watch is added still gets the deadline and the socket watches, and the deadline counts from
  *go*** [disputed 1]; the child's pid stays unreaped until the harness's *bye*, and a cancel after
  the record never signals a free pid [m2]; killing the harness while the program runs kills it;
  perfrun stays idle meanwhile; a self-SIGKILL worded without asserting a fork; a record cut short,
  over 4 KiB, a key twice, an unknown key, two records; `perfrun facts` read by its own rule (no
  child line) [p29]; no 2-s drain wait in any case [c101].
- **The launcher's build and cache**: a crate build's tool profile cannot write the cache with the
  target under `/var/folders`; **a default `/var/folders` TMPDIR is accepted, TMPDIR=$HOME is
  refused** [p4]; a tool-profile process that rewrites `xcrun`'s cache does not change the compiler
  [m1]; a user-owned Xcode is refused with its words [p8]; a perf run from another harness version
  never removes a folder a running perf run holds [n4]; a changed binary refused by its hash.
- **The profile**: `system`, `popen`, `posix_spawn`, `fork`, `vfork` each killed; no signal out; a
  run cannot read the other side's binary.
- **Paths**: the same program at `p000` and `p012`: argv[0], PATH, the executable path after the
  `$PROGDIR` rewrite, output and memory equal [c103].
- **Linux** (CI's ubuntu job; nothing of it runs here): counters opened before release, `inherit`,
  per-type events summed, `PR_SET_DUMPABLE` on perfrun, `PR_SET_PDEATHSIG`, a forking program killed
  on timeout with the record intact, a forked child killed after the program exits, `ru_maxrss` in
  bytes, GNU ar long names [n6, n10, c117, p23].
- **Workloads, results, evidence, units**: each rule of §3.1–§3.2 and §3.9, including every
  input-unusable reason's words; **archives**: an aborting, an unwinding, a thin-LTO (prefixed name),
  a fat-LTO and a no-std staticlib, each read right; a model-made unit beside a hand-written one
  (mixed-panic, the terminal path); a no-std unit beside a std unit (cause no-std, both orders); a
  fat-LTO unit (staticlib-only, as Cargo honours `lto` only there) beside a std unit (cause lto); two
  units exporting one name (cause unknown, the linker's lines) [p23]; currency per unit: a
  driver-stale unit left out, Re-checked, then the row reads "u-x was verified since" [p25]; a
  pending unit never named; a C-alone c-unstable keeps a unit's behaves-differently row and its kept
  outputs; a `--unit` run whose C turns c-unstable updates the C line and keeps the unit's row [p26];
  a crate-does-not-build after a behaves-differently row keeps the finding; a set-up row replaced by
  a newer set-up row; the set-up facts rebuild their words [p28]; a re-measure ending c-unstable,
  run-failed or stopped-by-sigkill keeps a behaves-differently finding as `found_before` [m25].
- **The cockpit**: the input digests cached across loads; no input hashed while a perf run holds the
  lock, in the cockpit and through MCP (the holder read inside `Snapshot::load`) [disputed 2]; an
  input over the budget reads "can't check" and the target opens; the Speed View's golden with a
  24-character workload id, each short form, a C-alone line with 12.41 s and 124.3 MB, a `last_try`
  line and a several-cores row; the tree labels at most 19 columns; the worst order over every
  outcome; C-side outcomes only under The original C; the unit header within 54 columns; the
  next-step words for each provenance (model-made with one and several attempts and no provider, a
  recorded hand edit, a zopfli-shaped lib.rs-only crate) [m18, p24]; the interrupted-Accept words
  the same in perf and in the cockpit's Cause [m30]; a behaves-differently fact kept through an
  Accept; the MCP fact's answer field on a busy row (no shift exported) [p27].
- **End to end** on a mini target with two accepted units, plus zopfli's u001: a generated input; a
  time-printing C with three sides is never behaves-differently on any side; a crashing C → shown once
  under the C; equal streams but exit 1 → "exits differently"; output over the cap on the Rust side
  only → "prints too much"; Compare; the C alone on day one (too-short warned); `perf show` with a
  current launcher cache, with none, and with `--no-check`.
- **Mutation checks** of §3.2's selection and archive facts, §3.3's judging order and the
  harness side's order (go-ahead, bye), §3.5's order, §3.7's replace rule and §3.8's rule; re-run
  §2's premise with the built launcher under load.

## 5. Order of work [c108–c110]

Each step committed green: (b) `harness-core`: the workloads and results files and their strict
readers, `PERF_LAUNCHER`, `PERF_RUN_LOCK`, `perf-recipe-1`, the words, short forms and worst order,
`promotion_marker` made public, the cockpit's interrupted-Accept words; (c) the shared build
refactors (`whole_cc_into`, objects once, the `replaces` helper, the `.perf` and `.perf-out` helpers,
slots, hashes, the archive reader); (a) the launcher, trampoline, cache (compiler through root-owned
paths, the locks) and profile, with the harness's `run_measured` (control socket, go-ahead, bye) and
the SIGTERM-then-SIGKILL change; (d) rows and measurement in `harness-oracle`; (e) the CLI; (f) the
cockpit (Speed group and View, the generalised Edit flow, the cached input hashing, the MCP fact);
(g) SCHEMAS (writer rows for `workloads.toml`, `migration/perf/**`, `migration/build/.perf/**` and
`.perf-out/**`, the crate `target/**` and `Cargo.lock`, the launcher cache outside the target; the
`perf-row` event; results are forgeable and non-canonical), the tutorial, the testing guide's Part 11
on liblzg. Then the code review, fix passes each checked, mutation checks, the handoff [c110].

## 6. Residuals and costs [c112–c117]

- **Rust's fixed start-up** (≈ 1.04e7 instructions for a std-using unit, 2.8e6 cycles, 48 KiB; a
  no-std unit adds nothing) is in every unit's and the program-as-it-stands's numbers: at the floor
  about 1 % of the instructions (under the 1.5 % margin, named on the row when it can explain it).
- **A workload that never runs a unit's code** reads "about as fast" — said on the row.
- **Behaves-differently is not a verify check**; output over 64 MiB is compared only by size, and
  output written to files is never compared [c116, n34].
- **Multi-process programs** cannot be measured. A program that re-execs itself: memory reads only
  its last image.
- **Several cores**: cycles, instructions and CPU time add up all threads; the words say so.
- **Noise**: memory rises with load; the same code can read "about as fast" one day and "can't
  tell" the next; at 15 runs on a busy Mac most rows do not answer and the person is offered 31,
  which answers most of them (§2); two matching C runs do not prove the C stable; the
  efficiency-core assumption (§3.8), checked per row only by the P-cost steadiness; kernel
  instructions under preemption (0.2–1.5 % at load 14–52) are in the headline; the executable's path
  differs per side; context switches are recorded (Linux only) but never worded.
- **One kind of computer**: results compare only on the same kind; history only through git.
- **Orphans**: if something outside the harness SIGKILLs both the harness and perfrun while a program
  runs, it runs on in its sandbox until it ends; and between perfrun's death and the harness's kill
  of the group, launchd could reap a program that ended at that very moment and the pid be reused
  (a window of the harness's reaction time) [n2, p3]. On Linux perfrun reaps before its record (the
  end counts need it), so a cancel between that reap and the harness's reading of the record could
  signal a reused pid (unchecked).
- **Security**: committed results can be forged; run temp dirs under `TMPDIR`; a crate's `build.rs`
  and the cargo configuration are outside the crate digest, as for verify — they can change the
  release profile, `rustflags` or the compiler wrapper [c33]; every `cc` call other than the
  launcher's still finds its compiler through `xcrun`'s per-user cache — a verify residual [m1].
- **Linking**: a thin-LTO unit links only first in plan order; Cargo ignores `lto` for a
  template-shaped crate (`["staticlib", "rlib"]`) [p23].
- **Linux** (`--allow-unsandboxed`): no sandbox — the program can fork (a grandchild that calls
  `setsid()` outlives the run), signal, rewrite binaries; the record's protection rests on
  `PR_SET_DUMPABLE`; counts are user-mode only; CPU time, switches and `ru_maxrss` include the
  moments before exec; hybrid CPUs' per-type counts are summed — **all unchecked here**.
- **Cost**, the estimate the dialog and the "31 runs" words show [c112, c113, n4, n21, n32, m29,
  p31]: builds — the launcher (only when its cache is stale, about 5 s), one compile of the C, per
  unit a crate build (about 5–30 s warm; when the crate's `target/` is missing or older than its
  sources: "plus building u002's Rust, which may take minutes") and one link (about 1 s), the
  as-it-stands link; first execs of each new binary (every side each run; perfrun and perfgo when
  their cache is new) at 0.25–14 s each on a loaded Mac (none on Linux) — counted as 10 s; runs
  per workload: the C alone 2 + n, each row 3 + 2n; seconds = runs × (the C's median clock time
  from its last stored rows, or 2 × its CPU time, + 0.1 s) + 10 s per new binary + the builds. For a
  1.2 s C (clock 1.3 s) at n = 15: the C alone ≈ 34 s, a row ≈ 56 s; the "31 runs" command for one
  unit (no C-alone step, so the C's first exec falls in that row too): 65 runs ≈ 91 s + 20 s of
  first execs + the C compile, the crate build and two links ≈ 111 s plus building. Before the C was
  ever measured: "about 2 + n runs of your program, then 3 + 2n for each row, per workload". A
  too-short row stops after its 3 step-1 runs.

## 7. What changed from revision 4

A control socket on perfrun's stdin carries the child line, the go-ahead, the record and the bye, so
stdout and stderr capture step 1 again; the child-line read happens outside the registry lock, and a
cancel there never starts the program; the program stays unreaped until the harness's bye, and the
harness kills its group at once when perfrun ends without a record; the deadline timer added at go,
each watch added on its own, signals restored in the child; exit codes never read as "never
started"; the cache refusal turned the right way (any tool-profile write root, never a default
TMPDIR), a shared lock per run, hashes kept in the cache; the compiler check covers ld, libLTO and
the SDK, with words; `perfrun facts` and its reader rule, `perf show` never building the launcher;
the share rule words a difference only when both metrics agree, its cause from the recorded load;
phased programs kept off normalisation; P fields only on computers with two kinds of cores; no
prediction of what 31 runs will do, the offer on every row that could use it; short runs need both
legs of the floor, the memory line kept above 4 MiB; rounding away from each branch's boundary;
"too few" words; memory has no close-call wording; the start-up note by the archive's std and the
interval; a too-short CPU gap shown and ranked; the C-alone short form and column rule; archives read
for panic runtime, std and fat LTO, three states, causes in words; set-up facts stored so their words
rebuild, the replace rule restated, `found_before` keeps a finding; C-side outcomes only on the C's
row; currency per unit with only verified units recorded; next steps by provenance; the MCP fact's
answer; tree labels within 19 columns; the cockpit's interrupted-Accept words changed to match.

## 8. Not decided here (for the person)

None.

## 9. Review record

- **The draft (0b64bba)**: 4 lenses, 49 findings, 46 confirmed by two verifiers, 2 by one, 1 refuted
  (`wf_fe7fb255-d44`).
- **Revision 1 (00f2e23)** checked (`wf_0f90338d-b1d`): 7 resolved, 41 in part, 1 made worse; 33 new.
- **Revision 2 (cd1f79b)** checked (`wf_89198fcc-3e6`): of 118 changes 36 done, 73 in part, 2 not
  done, 7 wrong; 35 new (two high).
- **Revision 3 (1b27d10)** checked (`wf_d21b13f6-ea1`): 74 done, 43 in part, 1 wrong; 30 new (one
  high), 1 by one verifier (taken).
- **Revision 4 (f7a4905)** checked (`wf_8aea486a-f12`; `perf/revision4-check.json`, digest
  `perf/rev4-digest.md`): 38 done, 35 in part, 2 wrong; 31 new confirmed by two verifiers (one high:
  the go-ahead pipe took perfrun's stdout, so step 1 could not capture), 2 confirmed by one (the
  kqueue registration and timer; how the cockpit knows a perf run holds the lock — both taken as
  clarifications), 1 refuted.
- **Choices made where the reviewers gave options**, all rounds: the trampoline (over ptrace or a
  kqueue hold); the launcher's private cache outside the target, refused only when a tool profile
  could write it (over refusing temp-folder targets, which every test is); its compiler found through
  root-owned paths; the hash as a check after the build; four-character slots, 1-based; the floor of
  1e9 instructions and 0.5 s of CPU, both legs for too-short and short (over a cycles floor or a
  measured start-up); 15 runs by default (the exact interval at a 3 % spread is about ±3.8 % at 7,
  ±2.3 % at 15, ±1.6 % at 31); the metric from three quarters of the runs and a steady P cost (over
  half: half lets more efficiency-core runs in); on the share rule's rows a difference only when both
  metrics agree (over always-busy, which hid a 3.7× slowdown, and over raw words alone, which called
  identical programs slower); no prediction of 31 runs (over the projected interval, which split rows
  at random); memory by interval with a scaled margin and no close-call wording; CPU seconds derived
  from the headline; no warm-up (step 1 is the first exec); the original C as a `baseline` row (the
  draft's C-only floor point became its too-short check); C, other, C per row; no recovery; no
  per-side start-up program; no dead-strip (the results must be verify's programs); a C's stderr not
  kept (it would need the kept-outputs folder for every row; one run by hand shows it); the
  instruction margin 1.5 %; `promotion_marker` made public; the panic runtime from archive member
  names with a third state (over the manifest, which `.cargo/config.toml` can override); the control
  socket (over passing descriptors beyond 0–2, which needs `unsafe`); the go-ahead and bye (over a
  SIGTERM handler alone, which a SIGKILL of perfrun defeats, and over dropping the registration at
  reap, which left a free pid registered); exit 1 for a missing, empty or broken workloads file; the
  Linux hybrid case summed and marked unchecked; the draft's integration caps kept. Not taken: a fresh
  TMPDIR for every crate build and `RLIMIT_NPROC`; a scheduling-band field in the record (the busy
  cause uses the load and the fast-core count instead) [c25, c118].

## 10. Build notes — the check of revision 5

The check of revision 5 (`wf_2b416222-f83`; `perf/revision5-check.json`, digest `perf/rev5-digest.md`)
found 39 open items done, 25 in part, 6 done wrongly, and 19 new findings confirmed by two verifiers
(none high: 4 medium, 10 low, 5 nits; cited [q1]), 3 by one, none refuted. As decided before the
round (FEATURES-PROGRESS, DECISIONS), revision 5 is built without a further design round; each note
below is a decision the build makes, each with a test, and **the notes govern where they differ from
§1–§9**.

**Launcher**
1. [q1, P7] The go-ahead usually arrives before *ready*: perfrun reads it whenever the socket is
   readable, remembers it, and sends *go* once both have arrived. The harness's timer counts from
   the go-ahead plus the step-1 allowance; both timers read as a timeout, and "perfrun's fires
   first" is not claimed.
2. [q2] The child keeps exactly *ready*'s write end, *go*'s read end, *status*'s write end, stdout
   and stderr; perfrun keeps the other ends and closes its copies of the child's.
3. [q3] The record's status gains `never-started no-ready` (with the child's end); the go-ahead is
   the byte `G`, the bye `B`; on an early end perfrun discards a pending `G` before it waits for `B`.
4. [q4, c24, n4] Locks: the shared version lock is taken while the exclusive `perf/.lock` is held,
   then `perf/.lock` is released; stale folders are removed only under `perf/.lock`, keeping the two
   newest versions (two worktrees in turn do not rebuild each other); after every `flock` the locked
   file's (device, inode) is compared with a fresh stat of the path, and the lock retried if they
   differ; `perf show` holds the shared lock while it runs `perfrun facts`. The cache refusal also
   covers `CARGO_HOME`.
5. [q5] Every rendered profile (tool, run, scenario) ends with `(deny file-write* (subpath <the
   canonical launcher cache root>))`; the refusal of §3.2 stays as a second line.
6. [q6] CPU times from `proc_pid_rusage` are mach ticks, converted with `mach_timebase_info`; the V4
   fallback stays (no P fields).
7. [q7] The program's group is killed before perfrun's (the registry kills in that order); on the
   pre-go SIGTERM path the child too stays unreaped until the bye.
8. [q8, c37] The child resets every catchable signal (1 to NSIG−1) to `SIG_DFL` and empties its mask;
   perfrun adds its SIGTERM watch before it ignores SIGTERM, before the fork; perfrun's kills go to
   both `-pid` and `pid` (the child may not have called `setsid()` yet). Linux's
   `PR_SET_CHILD_SUBREAPER` is not taken (Linux unchecked; §6 names the grandchild).
9. [disputed: the compiler, P8] Both layouts are named — Command Line Tools: `<dev>/usr/bin/clang`
   and `<dev>/SDKs/MacOSX.sdk`; Xcode: `<dev>/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang`,
   the `ld` and `lib/libLTO.dylib` beside it, `<dev>/Platforms/MacOSX.platform/Developer/SDKs/
   MacOSX.sdk`. `clang -###` confirms the `ld` and `libLTO` used are the checked ones (else
   `-fuse-ld=<checked ld>`); an `xcrun` shim is refused; compile and link run with a cleared
   environment. With no `xcode_select_link`: `/Applications/Xcode.app/Contents/Developer`, then the
   Command Line Tools; when the selected folder fails the ownership check and the Command Line Tools
   pass, they are used (said in the progress line).

**Words**
10. [q9, P11] The steadiness test is per side: the mean of the two sides' own spreads of
    ln(p_cycles / p_instructions) against the mean of their raw-cycle spreads; skipped when ≥ 97 %
    of the row's cycles were on the performance cores; "the program's phases differ in speed" only
    when it failed. A real cost-per-instruction slowdown (×1.1) keeps normalisation and reads slower.
11. [q10, P10] The share rule's can't-tell never asserts one cause: "K of the 30 runs ran mostly on
    the slower cores — the computer may have been busy (load about 14 on 8 fast cores), or the
    program runs there by design (several threads, a low priority)", the busy clause first only when
    the load less the program's own parallelism (its median CPU/clock, rounded up) is at least the
    fast-core count; short form `can't tell: slow cores`.
12. [q13, P12] The 31-run act's set is exactly the words' set (n < 31; probably, close call, or "can't
    tell: the estimate"); a short row whose interval lies inside ±M gives the short-run words, never
    the offer.
13. [q14, m15, P16] Rounding: each end to the nearest step; when that would put it on or past a
    boundary its branch relies on — 0 or ±M, and for a close call both — it shows two decimals,
    rounded away from that boundary; Y in "within ±Y %" rounds up.
14. [q16, p18] The too-short gap is shown only when the Rust is the slower side by more than 2×
    against both step-1 C runs, and both sides ran at least 20 ms of CPU; ranked with slower. The
    half-second factor is taken over the smaller side's CPU time.
15. [q15, p23] Archive members are matched as name segments: the runtime's members are
    `panic_abort-<hash>.panic_abort…` / `panic_unwind-<hash>.panic_unwind…`, std's `std-<hash>.std…`,
    each possibly after a thin-LTO `<crate>-<hash>.` prefix — never a crate whose own name merely
    contains them. Fat LTO is a defined `rust_eh_personality` with no std member. Causes gain "two
    units use no std" and "two units are built with lto"; the thin-LTO order sentence is dropped
    (it did not reproduce); perf always tries the link, and the archive facts only word a failure.
16. [m9] "· parallel" goes on a short form only where it fits 26 columns (about as fast, slower or
    faster without the interval, ≥ 2×); otherwise it is in the detail; the unit header's second line
    carries it.
17. [n14, m13] The seeded statement is a rate: over 12 seeds × 2 000 rows of +3 % at σ 4–5 %, n = 31,
    "probably slower" is the most common answer and "faster" or "about as fast" under 0.5 % of rows.
18. [p17, disputed: the start-up note] The note's interval (of the absolute median differences) must
    hold 1.04e7 with its upper end ≤ 2.6e7 (no lower bound); its §4 case is a rate over the recorded
    rows.
19. [q17] The MCP answer for a short row is `cant-tell-short-run`, with no shift exported.
20. [m11] §2's answer rates name their source (`design-check4/…/m11.py`); §6 says "at 15 runs under
    load fewer than half the windows answer; at 31 most do".
21. [m8] A Rust that crashes part-way with stdout already different reads "Rust crashed" (a §4
    case); the cockpit's fact sentence follows the same order.

**Integration**
22. [q11, c91, p24, m18] The recorded hand edit's step: "Hand edit u001's crate, then Replace u001's
    verified crate with the new attempt and measure this unit again — and if it is not faster,
    Replace it back with <the attempt in use now>"; the §4 test checks that each named act changes
    the crate perf will measure. §1 reads "written by hand and verified".
23. [q12, p26] One C-side rule: `c-unstable`, `c-crashed`, `c-timed-out`, `output-too-large` and
    `c-could-not-start` (moved out of the set-up list) are written only to the workload's `c_alone`
    row — replacing a C-side or too-short row there, otherwise beside its baseline as `last_try`;
    unit and as-it-stands rows are left exactly as they were (no `last_try`); the reader refuses a
    `c-` outcome outside `c_alone`.
24. [c21, m22, p25, q19] `left_out` reasons gain `replaces-changed` and `accept-interrupted`;
    currency compares with **measurable today** (every condition of §3.2's selection), not
    `fresh_green` alone; a held unit whose crate changed reads "u-x's Rust changed since"; that
    words for a left-out unit only for the build, link and replaces reasons.
25. [c33] A crate whose manifest's `[profile.release]` sets `opt-level`, `lto`, `codegen-units` or
    `panic` away from the defaults is named on its row ("the crate's manifest sets lto = thin"),
    read with the workspace's toml reader; `.cargo/config.toml` stays a §6 residual.
26. [c51] When the C exits non-zero, the progress line quotes the first line of its step-1 stderr
    (control characters shown escaped, at most 80 columns); nothing is stored.
27. [c113] The C compile's figure (about 1–5 s) joins the estimate; the example's sum includes the
    builds; a crate's build is "cold" when `target/release` is missing or its newest file is older
    than the newest source file (files compared, not folders).
28. [n30, q18] The View's header for rows that differ only in compilers: "with 2 compilers — see each
    row". Tree labels: `Speed (no workload)` for the starter, and `Speed (not yet run)`.
29. [p28] Run logs live in `migration/build/perf-logs/` (never recreated by the `.perf` helper), one
    per run, the last 20 kept.
30. [disputed: the Modify words] The greyed Modify reason is the cockpit's own, word for word.
