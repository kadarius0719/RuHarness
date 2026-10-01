# C-vs-Rust performance baselines (design)

Status: **revision 3 — after the check of revision 2 (cd1f79b)**, to be checked (2026-10-01).
History: the draft (0b64bba) had an adversarial review (4 lenses, 49 findings, 46 confirmed by two
verifiers); revision 1 (00f2e23) answered them; its check (`wf_0f90338d-b1d`) gave 118 changes
(scratchpad `perf/rev2-changes.md`, cited [c12]); revision 2 (cd1f79b) took them; its check
(`wf_89198fcc-3e6`, `perf/revision2-check.json`, digest `perf/rev2-digest.md`) found 36 of the 118
done, 73 in part, 2 not done, 7 done wrongly, and 35 new findings confirmed by two verifiers (cited
[n12]), 1 disputed, none refuted. This revision answers each. The §15 spike: DECISIONS.md
"2026-09-30 — C-vs-Rust performance baselines: §15 spike". The person's wish: "baseline the
original C, then compare migrated code side by side as the transition proceeds".

## 1. What it is, in plain words

`harness perf run` measures, on workloads the person names (the inputs and options their program is
really used with):
- **the original C alone** — its CPU time, instructions and memory on each workload: the baseline,
  from the first day, before any unit is accepted (§3.6) [c20, n27];
- **the program as it stands** — the C with every accepted unit's Rust swapped in at once (when two
  or more units are measurable): the line that moves as the migration proceeds;
- **each accepted unit** — the C with only that unit's Rust: which unit to look at when the
  program got slower. Per-unit rows do not add up to the program as it stands (each carries Rust's
  fixed start-up once) [c1].

perf measures **accepted** Rust only — the crate `verify` passed and Accept promoted. A new attempt
is measured once it is accepted (§3.12) [c91, n28].

Each comparison runs the C and the other side alternately. The answer is one sentence about time —
"about as fast as the C (within 2 %)", "slower by 6 % (4–8 %)", "about 2 % slower (0.6–3.4 %) — too
close to the 2 % line to call", "can't tell — 9 of the 30 runs ran mostly on the slower cores: the
computer was busy" — then the details: CPU time, instructions and memory.

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
cycles +0.79 %, wall +3.8 %, footprint equal at 200 KB; the oracle's 30 KB samples are too small to
time.

**Checked by the review and the checks** (scratchpad `perf/`, `design-review/`, `design-check/`,
`design-check2/`; other agents loaded the machine throughout; the load average is given with each
figure where it was recorded) [c3–c7]:
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
- **Time under load** (load 13–53): cycles mix performance- and efficiency-core time; normalising to
  the performance-core cost per instruction cuts the per-run spread from 6–8 % to 2.3–3.3 %, but the
  cost per instruction itself rises on runs mostly off the performance cores (runs with under half
  their cycles there read 2–16 % high). In 48 live trials the interval rule never said slower or
  faster for identical programs. With every row normalised, seven runs a side answered 3 of 20
  rows and thirty-one 3 of 4; on a quiet machine (spread ≤ 0.75 %) seven runs answer about 98 % of
  rows. The revision-2 rule (normalise only when **every** run is mostly on the performance cores)
  answered 6 of 48 recorded rows and 2 of 8 live rows at n = 15 (load 10–15) [n12]; §3.8's rule
  replaces it, and its answer rates are re-derived from the recorded rows when it is built (§4).
- **Memory** (load 13–53): the lifetime maximum footprint clusters (≈ 12.7 / 14.7 MB on zopfli) and
  its minimum is not stable under load (identical binaries read different in 12 of 20 rows); an
  interval with a margin of max(5 %, 1 MiB) gave no false difference in 70+ rows. The footprint
  starts over at exec (the end maximum read 950 584 bytes, below the baseline read's 966 968): it
  is recorded as is, never minus the baseline [c5].
- **Rust's fixed start-up** (a unit linked, never called; zopfli's u001): +1.0e7 instructions,
  +2.8e6 cycles, +3 pages (48 KiB).
- **Linking** (macOS, checked): two Rust staticlibs from one rustc link into one C program (ld64
  rescans archives; one rustc gives them one std); with different panic strategies the first
  archive's runtime silently wins. **Unchecked here** (no Linux machine; from documentation): GNU ld
  scans each archive once, so plan order alone can fail and `--start-group … --end-group` is
  needed; two rustc versions or fat LTO make the std copies collide [c6, c111].
- **First execs** (macOS): each newly written binary's first exec waits on the system's check —
  7–11.5 s at load 9–11 (9.6 s and 10.7 s for two fresh binaries), 23–27 s for a run that starts
  three new ones. The wait is off-CPU (instructions and CPU time unchanged) [c7, n4].
- **The launcher's build** through `cc` put its object in `/var/folders`; a same-user process
  swapped it after a passed hash in 23 of 30 tries — the launcher's build is a trust boundary.
- **Process control** (load 8–11): perfrun's forked child stays in perfrun's process group unless
  moved, so killing "its group" killed perfrun before its record [n3]; `RLIMIT_CPU` on macOS only
  sends SIGXCPU, so a program that ignores it runs on [n2]; a failed exec after "go" exits 127 with
  no message [n1]; kqueue `EVFILT_WRITE` with `EV_CLEAR` reports the reader's close as `EV_EOF`
  [n11]; `rusage_info_v6` has no switch counts, and wait4's `ru_nvcsw` reads 0 [n8].

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
  every refusal of the file names the line and column (its own span for a rule beyond the syntax)
  and the workload [c8, c9, c104].
- `{input}` is a whole argument (`"{input}"`, never `"--file={input}"`), exactly once in `args`
  when `input` is set, and refused without `input` [c9].
- `input` as written: relative, UTF-8, no control character, no component starting with `.` or `-`.
  Resolved: its canonical path lies in the canonical target root, outside the canonical ledger
  folder and any `.git`; a regular file, at most 64 MiB, read **once** per `harness perf run`; every
  run's copy and the digest come from those bytes; a change to the file after that read changes
  nothing in the run [c11, c12].
- The workload's digest: blake3 over id, args, the input's name and bytes — **not `runs`** (a row
  records the n it used) [c10].
- An input that is missing, refused or too large affects only its workload: outcome
  **input-unusable**, with the fix in words — "bench/big.txt is not here — put the file back or
  remove the workload"; "bench/big.txt is a link — copy the file in instead"; "bench/big.txt is
  over 64 MiB — use a smaller input" — never stored over an earlier measured row (§3.7) [c13].
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

**Units** [c17, c18, n26]: status `verified` or `merged` **and** `status::unit_report(…).fresh_green()`,
read under perf's own writer lock, **and** no half-done Accept: `status::promotion_marker` (made
public) is called directly after the lock is taken — `unit_report` skips the marker while a live
holder holds the lock, and under perf's lock that holder is perf — and a legacy `.<crate>.prev` is
treated the same ("an Accept was interrupted — Re-check it first"; perf does no recovery: it writes
no verdict). The plan's `replaces` must equal the verdict's `inputs.replaces` ("its replaced files
changed since verify — Re-check it"). After perf builds a unit's crate, its
`unit_crate_file_set_hash` must equal the verdict's `inputs.rust_crate` ("its Rust changed since
verify — Re-check it"). Every stale reason of `unit_report` has its words ("its test driver changed
since verify", "its C changed since verify", "verify it first"). A pending or blocked unit is never
measured, fresh or not.

**Sides** [c19–c22, n27]:
- the **C**: every top-level `.c` of `source_dir` (a non-regular one refused by name), linked with
  `extra_link_args`;
- **a unit's program**: less the unit's `replaces`, plus its staticlib. A shared helper checks the
  `replaces` entries (each matches a collected top-level `.c`; an empty list refused): a mismatch,
  a crate that does not build, or a program that does not link is that unit's row (outcomes
  `replaces-mismatch`, `crate-does-not-build`, `does-not-link`, with the tool's first lines), and
  the next unit is measured;
- **the program as it stands**: with two or more measurable units — every one's staticlib, in plan
  order (`--start-group … --end-group` on GNU ld) — and only when every unit's crate has the same
  panic setting (`[profile.release] panic`; else outcome `mixed-panic`: "u001 aborts on a panic,
  u002 unwinds — the program as it stands would silently use one; give them the same setting")
  [c22]. A unit that does not build or link is left out of it and named. Its row records the units
  it holds (`units: [{id, crate}]`, plan order) as an input of the row, so a change reads "u-hash
  was accepted since" or "u-tree is left out now" [c21]. Fewer than two measurable units: no
  as-it-stands row; the CLI says "one unit measured (u001) — u-tree left out: verify it first" or
  "no accepted unit to compare yet", and a stored as-it-stands row from before reads out of date
  ("the program as it stands now holds one unit") [n27].

**Build** — "as verify builds them: C -O2 -ffp-contract=off, Rust release, no LTO", said in every
result; Rust's fixed start-up is part of what is measured (§6) [c31, c33]:
1. **The launcher and trampoline** (§3.3) are built from their embedded sources into a private
   cache outside the target and every temporary folder: `~/Library/Caches/ruharness/perf/` on
   macOS, `$XDG_CACHE_HOME/ruharness/perf/` (else `~/.cache/…`) on Linux; a 0700 folder named
   `<PERF_LAUNCHER>-<blake3 of the sources and cc --version>`, made by a helper that refuses
   links; `cc -c` then a link, `TMPDIR` set to a fresh 0700 folder inside it. No tool profile and
   no run lists it as writable — whatever the target's place, also a target under `/tmp` or
   `/var/folders`, where every tool profile may write [c24, n5]. Kept between runs (no new first
   exec each run [n4]); a stale version folder is removed when a new one is built; both binaries'
   blake3 recorded at the build and checked before each run. The hash guards against a change after
   the build; the build itself is trusted (it runs before any target code is built in this
   `perf run`) [c25].
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
`<name>`** (the trampoline sets it). What still differs per side: the executable's own path
(`_NSGetExecutablePath`) [c29, c30].

Not measured: the unit's differential driver (mostly `printf` and start-up) — a revisit item. A
per-side do-nothing program measuring Rust's start-up per unit [c32] is not built: it costs a link
and 2n runs a side to measure a share the instruction margin (§3.8) already covers at the floor;
the start-up is named on the rows it can explain (§3.8).

### 3.3 The launcher (`perfrun`) and the trampoline (`perfgo`) — harness-owned C

`crates/harness-oracle/src/perf/{perfrun,perfgo}.c`, embedded with `include_str!`; a version
constant `PERF_LAUNCHER` in `harness-core`, pinned to the sources by a test. The launcher is the one
harness-built binary that runs unsandboxed; the program runs only inside the sandbox. On a platform
other than macOS and Linux, `perf run` refuses by name [c44].

`perfrun MODE PROFILE PERFGO DEADLINE PROGRAM NAME ARGS…`:
1. Its **stdin is the write end of a pipe the harness made** (`std::io::pipe`, `Stdio::from`; the
   harness keeps the reader and gives the writer only to perfrun's `Command`); the record goes
   there, never to a file the program can reach [c40]. On Linux perfrun first sets
   `prctl(PR_SET_DUMPABLE, 0)` on itself, so a same-user program cannot open
   `/proc/<perfrun>/fd/0` (unchecked here) [c44, n6].
2. Three more pipes (*ready*, *go*, *exec-status*); `fork`. The child: `setsid()` — it leads a new
   session and group whose id is its own pid, and as a session leader the program cannot leave it
   (`setsid` and `setpgid` fail) [n3]; stdin `/dev/null`; stdout and stderr the run's capture pipes
   (step 1 runs) or `/dev/null` (timed runs); every other descriptor closed but the three pipes
   (bounded: `close_range`/`PROC_PIDLISTFDS`); then `execv("/usr/bin/sandbox-exec", ["-p", PROFILE,
   PERFGO, ready, go, status, PROGRAM, NAME, ARGS…])`. On Linux (no sandbox) it execs PERFGO
   directly, after `PR_SET_PDEATHSIG(SIGKILL)` and a `getppid()` check against perfrun's pid
   (unchecked here). Right after the fork perfrun writes `child <pid>` on the record pipe (one
   write, under `PIPE_BUF`) [n3].
3. **PERFGO** (inside the sandbox): marks *status* close-on-exec, writes one byte on *ready*, reads
   one on *go*, closes both, and `execv(PROGRAM, [NAME, ARGS…])`; if the exec fails it writes
   `errno` on *status* and exits 127. No CPU-time limit: on macOS it only sends SIGXCPU, which a
   program may ignore, and it would kill a legitimate threaded program as a crash [n2].
4. On *ready*: macOS `proc_pid_rusage(child, RUSAGE_INFO_V6)` (else V4) — the baseline:
   instructions, cycles, performance-core instructions and cycles, user and system time (mach ticks
   → `mach_timebase_info`) — and `proc_pidinfo(PROC_PIDTASKINFO)`'s context switches (`pti_csw`);
   Linux: the counters were opened on the child before it was released (`perf_event_open`,
   `instructions:u`, `cycles:u`, `inherit = 1` — a child's counts arrive only when it exits; on a
   hybrid CPU one event per core type, summed, with `TOTAL_TIME_RUNNING` summed over them against
   `TOTAL_TIME_ENABLED`: below it → no time words, never scaled; all zero → no counter) —
   **unchecked here** [c34–c36, c45, c46]. Then *go*; then *status* is read: end-of-file means the
   exec happened, bytes mean "the program never started (exec failed: <errno>)" [n1]. The
   monotonic clock starts at *go*.
5. **perfrun owns the program** [c37, n3, n11]: its deadline (DEADLINE seconds after *go*), SIGTERM,
   or the harness closing the record pipe (macOS: kqueue `EVFILT_WRITE` with `EV_CLEAR`, acting on
   `EV_EOF`; Linux: epoll with no requested events, acting on `EPOLLERR`) → `SIGKILL` to the
   child's group (`-pid`: the program and, on Linux, what it forked), reap, record
   `timeout`/`stopped`. perfrun is not in that group, so it survives to write the record. The pid
   cannot be reused while unreaped.
6. `waitid(P_PID, child, WEXITED | WNOWAIT)`, then the end counters (same fields, plus
   `ri_lifetime_max_phys_footprint`, `ri_child_*`); `wait4` for `ru_maxrss` and the switch counts.
   Recorded as **end minus baseline**: instructions, cycles, P-core instructions and cycles, CPU
   time (user + system, `cpu_us`; Linux from `wait4`'s `ru_utime + ru_stime`); context switches —
   macOS one total (`ru_nivcsw` at the end minus `pti_csw` at *ready*; no voluntary/involuntary
   split exists there), Linux voluntary and involuntary from `wait4` (including the moments before
   exec, named) [c39, n8]. Recorded **as is**: the memory (macOS lifetime maximum footprint, which
   starts over at exec; Linux `ru_maxrss` × 1024 in bytes, which includes the time before exec —
   about perfrun's size, a floor named in §6 [n10]); wall from *go* to `waitid`.
7. The record: built in memory, one `write()`, `key value` lines, ASCII, at most 4 KiB, last line
   `end`: status (`ok`, `never-started <errno>`, or the launcher's failure in words), the program's
   exit code or signal, whether perfrun sent the kill, the counters, the CPU's name
   (`sysctlbyname("machdep.cpu.brand_string")`; Linux: the harness reads `/proc/cpuinfo`), the OS
   product version and build, the 1-minute load average in hundredths; perfrun exits 0 [c40, c47].
   **The harness reads it strictly** [c40]: at most 4 KiB + 1 read (more is "over 4 KiB"), the
   `child` line first, then each known key once, every number bounded (counts ≤ 2^63, load ≤
   100 000), unknown keys and a second record refused, `end` last.

**Judging a run**, in this order [c41–c43, n1, n9]:
1. The harness's own end first: its timeout → `run-failed: timeout` (step 1 on the C:
   `c-timed-out`); output over the cap → `output-too-large`; cancellation → no row.
2. perfrun did not exit 0, wrote no record, no `end`, over 4 KiB, a malformed line →
   **unmeasurable** "the launcher stopped before measuring" (that row only).
3. No *ready* (the profile was refused, perfgo failed), sandbox-exec's own exit (65, 71), or
   `never-started` → "the program never started" with the reason: on the C `c-could-not-start`,
   on the other side `could-not-start` — never behaves-differently or too-short.
4. macOS only: a `SIGKILL` perfrun did not send → **tries-to-start-programs**, worded without
   asserting the cause: "the program was stopped by SIGKILL — perf's sandbox does this when a
   program tries to start another; if yours does not, it may have stopped itself or run out of
   memory". `ri_child_*` non-zero → the same (unreachable while fork is denied).
5. Otherwise the program's own end (exit code or signal) and its counters. A zero baseline is
   never subtracted.

**The harness side** [c38, c27, n3, n4]: a new runner entry `run_measured(launcher, record_pipe,
run_dir, input, collect)`. It reads the record's `child <pid>` line right after the spawn (bounded:
64 bytes, 10 s) and registers that group beside perfrun's in the live-process registry, so the
signal handler's `kill_live_process_groups` and every timeout or cancel reach the program even when
perfrun is gone. Its timeout for one perfrun is DEADLINE + a start-up allowance + 10 s: the
allowance is 60 s for a step-1 run (where every new binary's first exec falls) and 5 s for a timed
run, so a first exec never reads as the program timing out. DEADLINE is the target's
`[oracle] timeout_secs` and counts from *go*. On the harness's timeout, overflow or cancellation it
sends SIGTERM to perfrun, waits a bounded grace (250 ms), then SIGKILL to perfrun's group and the
program's. perf's run temp dirs are 0700 and registered with the signal handler's live-folder list
(as the map's random folder; verify's run temp dirs get the same).

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
   the same outcome without running it again. Otherwise the other side differs in its end or its
   streams → **behaves-differently** ("exits 1 where the C exits 0"; "stdout differs at byte
   40 961"). A non-zero C exit is not refused — it is named on the row from its stored exit code:
   "the C exits 1 on big-text: its error path is timed"; when both exit non-zero, both codes [c51].
2. **Too short**, decided per row from the two sides' step-1 instructions with a floor of **1e9**
   (≈ 0.1–0.5 s of CPU depending on the code; Rust's start-up is about 1 % of it) [c53, c54]:
   - **both under** → **too-short**, with both sides' CPU times and the fix: "too short to time:
     the C ran 40 ms of CPU and the Rust 360 ms on small-text — use an input at least about 7×
     bigger (13× for half a second)": the first factor is the floor over the C's instructions, the
     second half a second over the C's CPU time, both rounded up; "check the workload's options
     first" when the C wrote nothing or exited non-zero;
   - **either under** → measured, marked **short run**: never "about as fast"; inside the margin
     "can't tell on a run this short — use a bigger input"; no memory line (its footprint is
     mostly start-up);
   - otherwise measured in full.
   Step 1's streams go through pipes (≈ 4–5 % more work than `/dev/null`); the floor is judged on
   them, the times on step 3 [c54, c98].
3. **n timed runs a side, interleaved** C, other, C, other, … (n = `--runs` if given, else the
   workload's `runs`), streams to `/dev/null`. No separate warm-up: every binary's first exec was in
   step 1. A timed run that ends differently from its side's step-1 runs ends the row:
   **run-failed** with the side named [c55, c56].

**The C alone, per workload** (the baseline row; §3.6): C, C (the same check of end and output,
c-unstable or a C-side failure as above), the floor on the C alone (too-short with the same fix
words — the day-one warning [n27]), then n timed runs of the C.

### 3.6 The C alone [n27]

Every `perf run` that measures the program (no `--unit`) measures the C alone on each workload
first: outcome **baseline** with n runs, or a C-side outcome, or too-short. It is stored in
`program.json` under `c_alone` (§3.9), one row per workload, with `other` absent. Its words: "the C:
CPU about 1.2 s here today (varies with load) · 12.4 MB · 1.21e10 instructions". The View's "The
original C" lines read these rows. Event side `c`. With `--unit`, the C alone is not re-measured
(each unit row has its own C runs).

### 3.7 Outcomes (closed set) [c57]

- **Measured**: `baseline` (the C alone), `measured` (a side against the C; a short run is
  `measured` with `short: true`).
- **What the code does**: `behaves-differently`, `tries-to-start-programs`, `c-unstable`,
  `c-crashed`, `c-timed-out`, `output-too-large`, `too-short`, `run-failed: timeout | exit |
  signal`.
- **What the set-up could not do** — printed, never stored over a measured row [n31]:
  `not-verified: <reason>`, `replaces-mismatch`, `crate-does-not-build`, `does-not-link`,
  `mixed-panic`, `input-unusable`, `c-could-not-start`, `could-not-start`, `run-failed:
  unmeasurable`.

Every outcome is a result (exit 0); a set-up failure (the launcher or the C does not build, the lock
is held, the workloads file is missing, empty or has an error, an unsupported platform) is an error
(exit 1); a clap usage error (`--unit` with `--as-it-stands-only`) is exit 2 [c80].

### 3.8 The words (std only) [c58–c70]

Per metric, the **Hodges–Lehmann shift**: the median of every pairwise log-ratio
`ln(other_i / c_j)` (n² of them; values not finite or ≤ 0 never reach the sort, which uses
`total_cmp`), its distribution-free interval `[D(c+1), D(n²−c)]` from the sorted pairs, `c` the
largest u with `P(U ≤ u) ≤ 0.025` under the exact (n, n) Mann–Whitney null (a dynamic program in
u128; c = 2, 8, 23, 64, 127, 341 at n = 5, 7, 10, 15, 20, 31; confidence 96.8, 96.2, 95.7, 95.5,
95.1, 95.0 %, computed exactly). It is at least 95 % confident at every n for time and instructions; memory's
clustered footprints are covered by its margin instead [c60, n24]. Percentages are rounded to whole
numbers (to one decimal under 10 %); an interval whose ends round equal shows one number [c69].

- **Time** (the headline). The metric, per row [c59, n12]: **performance-core-normalised cycles**
  (P-core cycles per P-core instruction × all instructions) when at least **three quarters** of the
  row's 2n timed runs have V6 counters, a P/E split, and at least half their cycles on the
  performance cores — over all the runs; otherwise **raw cycles**, said ("cycles — K of the 30 runs
  ran mostly on the slower cores"); never mixed in a row; a V4 run has no split; a missing count is
  absent, never zero; no counters at all → CPU time, said. The headline includes the kernel
  instructions preemption adds (0.1–2 %, §6). The assumption is stated: an efficiency-core stretch
  is counted at the performance-core cost. With margin **M = 2 %**, the interval [a, b]:
  - inside ±M → **"about as fast as the C (within 2 %)"**;
  - a > +M → **"slower by X % (a–b %)"**; X ≥ 100 % → **"3.7× as slow (3.4–4.0×)"**;
  - b < −M → **"faster: takes X % less time (a–b %)"**;
  - **near the line** — b − a ≤ 2M and the interval excludes 0 (a > 0 or b < 0) →
    **"about X % slower (a–b %) — too close to the 2 % line to call"** (the mirror for faster),
    X the shift, on the line's side [c61, n13];
  - otherwise **can't tell**, with the cause [c62, n14, n15, n23]:
    - the row fell to raw cycles → "can't tell — K of the 30 runs ran mostly on the slower cores:
      the computer was busy; close other work and measure again";
    - else, while n < 31 and the interval's width × √(n/31) ≤ 2M → "can't tell: the runs varied by
      about S % — measure again with more runs: <command>" (cost said, §6);
    - else, while n < 31 → "can't tell: the runs varied by about S % — more runs would not settle
      it; measure on a quieter computer";
    - at n = 31, the interval excludes 0 → **"probably slower, by about X % (a–b %) — not clearly
      past the 2 % line"** (or faster);
    - at n = 31, the interval holds 0 → **"no clear difference: within ±Y %"**, Y = max(|a|, |b|).
    S is the runs' spread: 1.4826 × the median absolute deviation of ln(value) over a side's timed
    runs, as a %, the larger of the two sides. The command is per side: `harness perf run --unit
    u001 --workload big-text --runs 31`, `harness perf run --as-it-stands-only --workload big-text
    --runs 31` [n22].
- **Instructions** (details; never speed words; **M = 1.5 %**, above Rust's start-up at the floor
  [n19, c53]): "about the same instructions (within 1.5 %)", "X % more instructions (a–b %)", "X %
  fewer", near the line "about X % more instructions — too close to the 1.5 % line to call",
  otherwise "instructions: can't tell". A unit row within 1 % above the C adds "(about Rust's fixed
  start-up)" [c31].
- **Memory** (details; macOS lifetime maximum footprint, Linux maximum RSS — named): the same
  interval with margin **max(5 %, min(1 MiB, 20 %))** of the C's median — about 8 % on zopfli, 20 %
  for a 1 MB program [n18]: "about the same memory (within 8 %)", "uses about X % more memory (a–b
  %)", "less", near the line "about X % more memory — too close to the 8 % line to call", and "can't
  tell — memory varied from run to run" only when the interval is wider than twice the margin
  [c66, n19]. No memory line on a short run.
- **CPU time shown** [c67, n16]: the C's median CPU time a run (user + system, the sandbox's set-up
  subtracted), labelled as varying with load ("CPU about 1.21 s here today"); the other side's
  seconds come from the headline — the C's × (1 + shift), "→ about 1.28 s (from the estimate)" —
  so the two never contradict. With no counters, CPU time is the headline itself and both medians
  are shown.
- **Several cores** [n17]: when either side's median CPU time exceeds 1.2 × its median clock time,
  the row says "uses several cores: the words compare total CPU work, not waiting — clock time
  0.70 s → 0.25 s".
- A unit row that says "about as fast" adds "(perf cannot tell whether big-text runs this unit's
  code)" [c68].

The CLI, per row: a headline line and a detail line, each wrapped at 80 columns with a 6-space
hanging indent [c69]:
```
perf: u001-katajainen on big-text — slower by 6 % (4–8 %)
      CPU about 1.21 s → about 1.28 s · 3 % more instructions · memory about the same
      · 15 runs each
```
The C alone: `perf: the C on big-text — CPU about 1.21 s here today · 12.4 MB · 15 runs`.

**Short forms** (the View and the unit header; the full sentence is the row's detail) [n30]:
`about as fast` · `slower 6 % (4–8 %)` · `3.7× as slow` · `faster 12 % (10–14 %)` · `too close to
call (≈2 % slower)` · `probably slower ≈3 %` · `no clear difference (±4 %)` · `can't tell — busy
computer` · `can't tell — more runs` · `short run: can't tell` · `too short to time` · `prints
differently` · `C prints differently each time` · `C crashed` · `tries to start programs` · `not
measured: <reason in ≤ 30 columns>`. Each fits 26 columns after a 24-character workload id.

### 3.9 Results: `migration/perf/program.json` and `migration/perf/units/<id>.json` (`ruharness-perf`, v1)

`program.json` holds two lists, `c_alone` and `as_it_stands`, one row per workload each; a unit's
file one list of rows. A measurement replaces only the rows it measured. Each row [c71–c79, n31]:
- `workload`, `outcome`, `short`, `runs` (5–31), `platform_metrics` (`macos-v6-pnorm`,
  `macos-cycles-mixed`, `macos-v4-cycles`, `linux-cycles`, `linux-hybrid-summed`, `cpu-time`) [c72];
- `inputs`: the workload digest, the program digest, the crate digest(s), the unit's `replaces` and
  `program_name`, `units` (as-it-stands rows), `recipe` (`perf-recipe-1`, a constant in
  `harness-core` beside `PERF_LAUNCHER`), `launcher` (`PERF_LAUNCHER`), `computer` (OS product
  version and build, arch, CPU), `compilers` (the first line of `cc --version` and of `rustc -V`);
  the facts digest is not an input (the program digest covers the C) [c33, c78];
- for `baseline` and `measured` rows: `c` (and `other`, not on `baseline`), arrays of exactly
  `runs` entries, each entry the run's counts — instructions, cycles, p_instructions, p_cycles,
  cpu_us, wall_us, memory (bytes), switches (macOS) or switches_voluntary and switches_involuntary
  (Linux), load (hundredths), exit code — every count ≥ 1 where present, a missing counter absent,
  never 0 [c60, c75];
- for the other outcomes: `c` and `other` absent; `step1` holds what step 1 measured, by name
  (`c_first`, `other`, `c_second`: instructions, cpu_us, exit code, stdout and stderr sizes) [c72];
- `first_difference` (behaves-differently only): numbers only — the stream (`stdout`, `stderr`,
  `exit`), both lengths, the byte offset, both exit codes; the strict reader refuses text there.

**No words are stored**: they are computed from the numbers by §3.8 on every read. Read strictly by
outcome (`read_regular` with a 4 MiB cap; arrays of the stated length; p_instructions ≤
instructions, p_cycles ≤ cycles, both zero or neither; counts ≥ 1; runs 5–31; ids from the
alphabet, once; a new key is a new `schema_version`) [c75, c79]. Written after every row with
`write_atomic` into a folder the CLI resolves with `safe_ledger_dir` and passes in (the check stays
in `harness-cli`, the rows in `harness-oracle`, the readers in `harness-core`) [c76]; a linked
`migration/perf` (or a linked parent, or a linked workloads or results file) refused by the CLI and
the cockpit on read and on write [c75]. Rows of removed workloads are dropped on the next write; a
unit file of a unit no longer in the plan reads "no longer in the plan". A never-ran outcome with no
earlier row is stored, so the person sees it [c77].

**Kept outputs** of a behaves-differently row: `migration/build/.perf-out/program/` and
`migration/build/.perf-out/units/<id>/` — a dot folder no unit id can name (ids start with a letter
or digit) [n7, n33] — `<workload>.{c,other}.{stdout,stderr}`, each capped at 64 MiB, written with
`write_atomic` through a helper that refuses links, in no tool profile; replaced when that row is
re-measured, removed when it no longer differs; the row records each file's size and blake3 [c74].

**Current** iff each input equals today's, each with its own reason: "your workload changed", "the
program's C changed", "the unit's Rust changed", "its replaced files changed", "the program's name
changed", "u-hash was accepted since" / "u-tree is left out now", "the system was updated (26.5.2 →
26.6)", "measured with other compilers (rustc 1.90.0 then, 1.91.0 now)", "measured on another kind
of computer (Apple M1)", "made by another version of the harness". `harness perf run` judges all;
`harness perf show` runs `cc --version`, `rustc -V` and `/usr/sbin/sysctl -n
machdep.cpu.brand_string` as sandboxed tool runs to judge all, or with `--no-check` says "computer
and compilers not checked" [n35]; the cockpit judges what it can without starting a process and
says the rest is not checked (§3.11). A results file is evidence, not ledger truth; committing it
keeps a history in git; a committed file whose digests match reads current — it gates nothing, and
the next measurement replaces it [c78, c79]. Stale facts: `perf run` refuses ("scan first"), as
`features map` does.

### 3.10 The CLI [c80–c84]

- `harness perf run [--target T] [--unit ID]… [--workload ID]… [--runs N (5–31)]
  [--as-it-stands-only] [--allow-unsandboxed] [--json]` — the writer lock; without `--unit`, the C
  alone, the program as it stands and every measurable unit; `--unit` measures only those units (no
  C-alone or as-it-stands row); `--as-it-stands-only` measures the C alone and the program as it
  stands, and with fewer than two measurable units says why and measures the C alone [n27]. Unknown
  ids: exit 1 naming the known ones.
- `harness perf init [--target T]` (resolves `migration/perf` with `safe_ledger_dir`; refuses unless
  `workloads.toml` is absent by lstat, as `features init` does) [c83]; `harness perf save [--target
  T] --expect … --bytes …`; `harness perf show [--target T] [--no-check]` (the words of the stored
  rows, current or out of date).
- Progress (`message` events): "building the launcher…" (only when its cache is stale), "building
  the C program…", "u001 — building its Rust…", "the C on big-text — checking it ends the same way
  twice…", "u001 on big-text — C, u001, C…", "u001 on big-text — timed run 9 of 30…", "keep the
  computer quiet while it measures". A behaves-differently row is printed first and in full, with
  where both outputs are kept. Summary: "perf: measured 4 rows, 1 too short, 0 behave differently —
  wrote migration/perf/ (commit it to keep a history)".
- Events: `perf-row {side: "c"|"program"|"unit", unit (unit rows only), workload, outcome, words}` —
  `words` a display-only courtesy, recomputed by every reader (SCHEMAS says so beside the event)
  [c83].

### 3.11 The cockpit [c85–c92]

- A **Speed** group in the tree beside Features: `Speed (no workloads file)`, `Speed (no workloads
  yet)`, `Speed (workloads file has an error)`, `Speed (the C measured, no unit yet)`, `Speed (2 of
  3 units measured)`; acts **Write your workloads file** / **Edit the workloads file** / **Continue
  my workloads draft**, **Measure speed** (greyed, with the state's words, in the first three
  states), on a unit **Measure this unit's speed** and **Measure again with more runs**, and on the
  As-it-stands heading **Measure the program as it stands again with more runs** (`--runs 31`, the
  workloads that could not tell) [c16, c63, n22]. Each confirm dialog says what runs, how many times,
  the estimate (§6), that it writes `migration/perf/` (and scratch folders under
  `migration/build/`) and no verdict, all under the ledger's lock, that Cancel keeps finished rows,
  and to keep the computer quiet; when it ends: "Measured 4 rows — see Speed" [c92].
- **Inputs read by the cockpit** [c14]: to judge "your workload changed" it hashes each input once
  per load off the UI thread (in `Snapshot::load`, shared with the MCP reads), with the same
  confined, bounded read perf uses (`read_regular`, under the target root, 64 MiB), counted in
  `preflight`'s hash budget; the program-digest budget applies with a features **or** perf file. A
  missing or refused input reads "can't check: bench/big.txt is not here", never "your workload
  changed" [c13]. While a perf run holds the lock the cockpit reads the rows already written.
- The **Speed View** (golden at 54 columns, inside an 80-column terminal):
  ```
  Speed — your workloads, the C against accepted Rust
  measured on Apple M3 with rustc 1.94.1 (not checked
  here) · as verify builds them · 15 runs each

  The original C
    big-text      CPU about 1.21 s · 12.4 MB
    many-small    too short to time
  As it stands (2 of 3 units — u-tree left out)
    big-text      slower 6 % (4–8 %)
    many-small    too short to time
  u001-katajainen
    big-text      slower 6 % (4–8 %)
    many-small    too short to time
  u002-hash
    big-text      about as fast
    many-small    too short to time
  ```
  The header is built from the rows: the computer and compilers they record, "not checked here"
  (the cockpit cannot read the CPU or OS build without a process), "n runs each" or the per-row n
  when they differ [c78, n30]. Workloads in file order; units sorted worst first by their worst row,
  the order: prints differently, C-side failure, tries to start programs, slower (by the shift),
  probably slower, too close to call (slower), can't tell, short run, too short, about as fast,
  too close to call (faster), probably faster, faster; out of date rows last within a unit; ids
  cut with …; a stale row's short form dimmed with "out of date: …" under it [c89]. The selected
  row's full sentence and details show below the list, wrapped.
- A unit's header: `Speed: slower 6 % on big-text (4–8 %)` (its worst row of today's workloads).
- The project summary gains `Speed: 3 of 5 units measured — 1 slower, 2 about as fast · 1 out of
  date — see Speed` [c90].
- A **behaves-differently** row [c86–c88, n29]: a fact on the unit — "With u001's Rust the program
  prints differently on big-text (stdout, byte 40 961) — verify does not run this workload" — with
  the next step "Compare the outputs; then Modify with a note naming big-text and the first
  difference, Accept the new attempt, and measure this unit again". No Re-check (it would pass and
  loop). It clears when the row is re-measured the same, or when a code input changed (the
  workload, the C, the unit's Rust, its `replaces`); after an environment change (system,
  compilers, computer, harness) it stays as "found on Apple M3 with rustc 1.90.0 — measure again to
  check". An as-it-stands difference goes on the As-it-stands heading and in the project summary,
  naming the units it holds: "no unit's Rust differs alone — measure each unit (or the program
  without one of them) to find it"; on a unit only when its own row differs [c87]. **Compare the
  outputs** shows the kept files side by side when they are regular files (`read_regular`, bounded)
  whose size and blake3 match the row, control characters shown escaped as in reasons
  (`harness_core::text`); else "the two outputs are not on this computer — measure again" (this
  unit, or the program as it stands) [c73, c88].
- A slower row's next step, as words [c91, n28]: "perf times only accepted Rust. If speed matters
  here: Modify with a note about speed (give these numbers), Accept the new attempt, measure this
  unit again — and Accept the earlier attempt back if it is not faster". Help gains a Speed section,
  that sentence, "perf stops even a fork; verify allows a fork but not starting another program",
  and the glossary (CPU time, instructions, memory, performance cores) [c90, c114].
- The MCP reads export each unit's Speed fact (worst outcome, workload, short form; fenced as the
  features facts are) [n35].
- Acts' argv: `with_sandbox_flag(harness_argv(["perf", "run", …, target_arg]))`.

### 3.12 Security (summary) [c93, c94]

The program runs only under the perf profile: no fork (killed on trying), no signal out, no network,
no reads under the home folder or the target beyond its binary, perfgo and its temp dir, writes only
its temp dir. **Nothing the program starts, and not the program itself, outlives its run** on macOS:
it cannot fork, it leads its own session so it cannot leave its group, perfrun kills that group, and
the harness knows the group from the record's first line and kills it when perfrun is gone [c93].
The launcher is the one unsandboxed harness binary: built into a private cache no tool profile can
write, wherever the target lies, hash-checked before each run. The counters return on a pipe the
program never holds (macOS; Linux relies on `PR_SET_DUMPABLE`, unchecked). The input is read once,
confined and bounded; results carry no words and are read strictly — but a committed results file
can be forged (it gates nothing; the next measurement replaces it). Run temp dirs under `TMPDIR`
can be rewritten by a same-user process that outlived a build (as every run's today); the program's
counts are whatever its behaviour makes them. No new dependency; `unsafe` Rust stays forbidden
everywhere.

## 4. Tests and checks [c95–c107, c111]

- **The words**: the critical values and confidences (n = 5, 7, 10, 15, 20, 31: c = 2, 8, 23, 64,
  127, 341; positions D(3)/D(23), D(9)/D(41), D(65)/D(161), D(342)/D(620); 96.8–95.0 %) with ±1
  mutations failing; a seeded synthetic A-vs-A at 0.5 % noise "about as fast" at every n; the
  interval narrows as n grows; near the line: seeded A-vs-A at loaded noise gives it in at most
  about 5 % of rows, a +2 % shift in most rows at n = 31; [−0.5, +3.5] is can't tell; a +3 % shift
  at n = 31 "probably slower"; the busy cause exactly when the row fell to raw cycles; "more runs"
  only when the projected width fits; the recorded rows (`design-check*/`) never switch to raw
  cycles because n grew [n12–n15]; fallbacks (V4, p_instructions = 0 read back from a file, a
  small P-share, a self-backgrounding program, a stored 0) never panic or divide by zero; on the
  recorded rows, the CPU seconds shown never contradict the headline (§3.8 derives them) [c96, n16];
  instruction intervals crossing +1.5 % and −1.5 %; rounding never shows "3–3 %"; the CLI lines and
  every short form at 80 and 26 columns.
- **Memory**: the same allocating binary on both sides under load never "more/less"; a synthetic
  two-cluster sample; a 1 MB program's margin is 20 % [c97, n18].
- **Threads**: a 1-thread and a 4-thread side — "uses several cores" with the clock times [n17].
- **The floor**: either side of 1e9; the C under and the other over, and the other under (short
  run); both under, with both times and the factors; step 1 through pipes, the times from step 3;
  the first row's words carry no first-exec or sandbox time [c98, n20].
- **The launcher and trampoline**: an exact baseline (a test-only delay before *go* changes no
  count); argv[0] = the bare name; a self-exec'ing program (counts carry on, memory the last image);
  a missing program and an exec the profile denies are each "never started", a real exit-127
  program is not [n1]; a refused profile, a child ending early → unmeasurable or never started as
  §3.3 says; an empty program never reads 0; perfrun's own deadline gives a `timeout` record, not
  unmeasurable [n3]; a `setsid()`/`setpgid()` spinner that ignores SIGXCPU is dead after timeout,
  cancel, overflow, perfrun being killed and the harness being killed [n2, c93]; killing the
  harness while the program runs kills the program, and perfrun stays idle meanwhile (no spin) [n11];
  a 4-thread program is not judged crashed; a self-SIGKILL is worded without asserting a fork [n9];
  a record cut short, over 4 KiB, with a key twice, an unknown key or two records; exit 0 with no
  record; macOS switches one total, never a 0 voluntary [n8]; a first exec slower than the timed
  allowance does not time out step 1 [n4]; no 2-s drain wait in any case (each case timed) [c101].
- **The launcher's cache**: a process under a crate build's tool profile cannot write it, with the
  target under `/var/folders` (the tests' own place) and outside the temp roots [n5]; a stale
  version folder replaced; a changed binary refused by its hash.
- **The profile**: `system`, `popen`, `posix_spawn`, `fork`, `vfork` each killed; no signal out; a
  run cannot read the other side's binary.
- **Paths**: the same program, allocating nothing, at `p000` and `p012`: argv[0], PATH on both
  streams, the executable path after the `$PROGDIR` rewrite, output and memory equal; a same-named
  tool earlier on PATH never reads "could not start"; inodes unchanged [c103].
- **Linux** (CI's ubuntu job; nothing of it runs here): counters opened before release, `inherit`,
  per-type events summed, `PR_SET_DUMPABLE` on perfrun (the program cannot open
  `/proc/<ppid>/fd/0`), `PR_SET_PDEATHSIG`, a forking program killed on timeout with the record
  intact, `ru_maxrss` in bytes [n6, n10].
- **Workloads, results, evidence, units**: each rule of §3.1–§3.2 and §3.9 (the caps, `{input}` as
  a whole argument, a refusal naming its workload with line and column, a missing input skips only
  its workload, the input changed after the read changes nothing, the digest ignores `runs`, a 7-run
  row current after a `runs` edit, links refused at every entry point — `perf init`, `perf run`,
  the cockpit — for the folder, a parent, the workloads file and a results file, set-up outcomes
  never stored over measured rows, removed workloads dropped, the early-stop rows' `step1`, units
  named `p`, `P`, `program`, `perf-out`; a half-done Accept taken under perf's own lock, a legacy
  `.prev`, a stale unit, a pending or blocked fresh unit, `replaces` edited after verify, an empty
  `replaces`, a `sub/unit.c` layout, a non-linking unit's row while the next is measured, mixed
  panic settings, zero and one unit never C against C, one unit never measured twice) [c104–c106].
- **End to end** on a mini target with two accepted units (a test fixture: two small `.c` files
  replaced by two tiny crates), plus zopfli's u001: a generated input; a time-printing C with three
  sides is never behaves-differently on any side, and c-unstable only when the second rolls over
  between a row's two C runs [n25, c107]; a crashing C → a C-side outcome on every row; equal
  streams but exit 1 → behaves-differently; output over the cap; Compare after another unit's
  measurement, and after the kept files are removed; no Re-check offered after a green Re-check;
  an as-it-stands difference with no unit's own; the C alone on day one (too-short warned); the
  cockpit's golden View with a 24-character workload id, two workloads and each long form.
- **Mutation checks** of §3.2's selection (freshness, the marker, `replaces`), §3.5's order,
  §3.7's outcomes and §3.8's rule; re-run §2's premise with the built launcher under load, and
  re-derive §2's answer rates with §3.8's metric rule on the recorded rows.

## 5. Order of work [c108–c110]

Each step committed green: (b) the workloads and results files in `harness-core` (`PERF_LAUNCHER`,
`perf-recipe-1`, the strict readers, `promotion_marker` made public); (c) the shared build refactors
(`whole_cc_into`, objects once, the `replaces` helper, the `.perf` and `.perf-out` helpers, slots,
hashes); (a) the launcher, trampoline, cache and profile, with the harness's `run_measured`, the
second registered group and the SIGTERM-then-SIGKILL change; (d) rows and words in `harness-oracle`;
(e) the CLI; (f) the cockpit (Speed group and View, the generalised Edit flow, the input hashing,
the MCP fact); (g) SCHEMAS (writer rows for `workloads.toml`, `migration/perf/**`,
`migration/build/.perf/**` and `.perf-out/**`, the crate `target/**` and `Cargo.lock`, the launcher
cache outside the target; the `perf-row` event; results are forgeable and non-canonical), the
tutorial, the testing guide's Part 11 on liblzg (make an input and commit it, time the C by hand,
write the workloads file, measure, read the words — a word table, u-version (Step 6.3, Part 8) as
the unit no workload reaches; whether `lzg -9` on a 2.5 MB dictionary file clears the floor is
unchecked: no liblzg here). Then the code review, fix passes each checked, mutation checks, the
handoff [c110, disputed: the order].

## 6. Residuals and costs [c112–c117]

- **Rust's fixed start-up** (≈ 1e7 instructions, 2.8e6 cycles, 48 KiB on zopfli's u001; each unit's
  differs) is in every unit's and the program-as-it-stands's numbers: at the floor about 1 % of the
  instructions (under the 1.5 % margin, named on the row when it can explain it); its 48 KiB is far
  under the memory margin. A unit no workload reaches does not read exactly "the same".
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
  tell" the next; two matching C runs do not prove the C stable; the efficiency-core assumption
  (§3.8); kernel instructions under preemption (0.1–2 %) are in the headline; the executable's path
  differs per side; context switches are recorded but never worded.
- **One kind of computer**: results compare only on the same kind (OS build, arch, CPU); history
  only through git.
- **Security**: committed results can be forged; run temp dirs under `TMPDIR`; a crate's `build.rs`
  and the cargo configuration (the target root's and its parents' `.cargo/config.toml`,
  `$CARGO_HOME/config.toml`) are outside the crate digest, as for verify — they can change the
  release profile, `rustflags` or the compiler wrapper, making "as verify builds them" untrue for
  every result [c33].
- **Linux** (`--allow-unsandboxed`): no sandbox — the program can fork (its children counted by
  `inherit` only when they exit; a grandchild that calls `setsid()` outlives the run, since
  `PR_SET_PDEATHSIG` covers only the direct child and the group kill misses a new session),
  signal, rewrite binaries; the record's protection rests on `PR_SET_DUMPABLE`; counts are
  user-mode only (a syscall-heavy change can read "about as fast"); `ru_maxrss` includes the time
  before exec (about perfrun's size); hybrid CPUs' per-type counts are summed and a run whose
  events were not running all the time has no time words — **all unchecked here** (no Linux
  machine) [c45, c46, c117].
- **Cost**, the estimate the dialog shows [c112, c113, n4, n21, n32]: per `perf run` — the launcher
  build only when its cache is stale; one compile of the C; per unit one crate build (usually warm)
  and one link; the as-it-stands link (two or more units); first execs of each new binary — every
  side each run (`.perf/` is rebuilt), and perfrun and perfgo when their cache is new — at 7–11 s
  each on a loaded Mac, none on Linux. Runs per workload: the C alone 2 + n; each row 3 + 2n; rows =
  units + 1 when there are two or more units. Seconds: runs × the C's median clock time from its
  last stored rows (or 2 × its CPU time when only that is known) + 0.1 s a run + 10 s per new binary
  — all under the ledger's lock. For a 1.2 s C (clock 1.3 s) at n = 15: the C alone 17 runs ≈ 24 s,
  a row 33 runs ≈ 46 s; at n = 31 a row 65 runs ≈ 91 s ("measure again with more runs" says so).
  Before the C was ever measured: "the C's time is not known yet: about 2 + n runs of your program,
  then 3 + 2n for each row, per workload". A too-short row stops after its 3 step-1 runs.

## 7. What changed from revision 2

The C, other, C order for every row (no C run shared); the C alone as its own `baseline` row; the
time metric from three quarters of the runs (not every run), near-the-line and "probably" words
defined, the busy cause tied to the metric, "more runs" only when it can help, the spread defined,
CPU seconds derived from the headline, several cores named; the instruction margin 1.5 %, memory's
margin scaled to small programs; the trampoline's exec-status pipe (never started ≠ exit 127); the
child leads its own session and group, perfrun kills that group and survives, the harness learns
the group from the record's first line; no CPU-time limit; the launcher in a private cache outside
the target and the temp roots, kept between runs; first execs outside the timed budget; macOS
switches as one total, Linux memory in bytes, `PR_SET_DUMPABLE` on perfrun; the record read
strictly; outcomes regrouped (set-up outcomes never stored over measured rows; `could-not-start`,
`mixed-panic`, `baseline`); early-stop rows' `step1`; kept outputs under `.perf-out`, slots 1-based;
the selection under perf's own lock with the marker and `replaces`; the cockpit's input hashing,
states, short forms, header from the rows, worst order, project summary line, the honest next step
(Accept), behaves-differently kept through environment changes, Compare escaped and bounded, the
MCP fact; `perf show` judging or saying it did not; the cost formula and its example corrected;
the order of work starting with the files and helpers; the end-to-end test on a two-unit fixture.

## 8. Not decided here (for the person)

None.

## 9. Review record

- **The draft (0b64bba)**: 4 lenses (measurement, security, integration, the person), 49 findings,
  46 confirmed by two verifiers, 2 by one, 1 refuted (`wf_fe7fb255-d44`; `perf/design-review-result.json`).
- **Revision 1 (00f2e23)** checked (`wf_0f90338d-b1d`; `perf/revision1-check.json`): of the draft's
  findings 7 resolved, 41 in part, 1 made worse; 33 new, all confirmed by two verifiers, 1
  disputed, 3 refuted. The 118 changes: `perf/rev2-changes.md`.
- **Revision 2 (cd1f79b)** checked (`wf_89198fcc-3e6`; `perf/revision2-check.json`, digest
  `perf/rev2-digest.md`): of the 118 changes 36 done, 73 in part, 2 not done, 7 done wrongly (the
  launcher's folder under a temp root, the group kill, the shared C pair, CPU seconds against the
  headline, the CPU limit's claim, the cost); 35 new findings confirmed by two verifiers (two high:
  the every-run metric rule answering almost no loaded row; the shared C pair undoing C, other, C),
  1 disputed (the order of work and a two-unit end-to-end — taken), none refuted.
- **Choices made where the reviewers gave options**, all rounds: the trampoline (over ptrace or a
  kqueue hold); the launcher's private cache outside the target and the temp roots (over a folder
  in `.perf/`, which a temp-root target exposes, and over refusing such targets, which every test
  is); the hash as a check after the build (option a); four-character slots, 1-based; the 1e9
  instruction floor (over a cycles floor or a measured start-up: instructions are the steadiest
  count, and the start-up is named instead); short runs marked and never "about as fast"; 15 runs
  by default (twice 7's cost on a quiet machine, where 7 already answers; under load, at the
  3 % run-to-run spread of normalised cycles, a normal approximation gives an interval of about
  ±3.1 % at n = 7, ±2.1 % at 15 and ±1.5 % at 31 — 15 is where it first nears the 2 % margin); memory by interval with a
  scaled margin (over ranges only); CPU seconds derived from the headline (over medians, which
  contradicted it); no warm-up (step 1 is the first exec); the original C as a `baseline` row (over
  no row); C, other, C per row (over a shared pair with a fresh C run per side); no recovery (over
  recovering and saying so); no per-side start-up program (over a stub per side); exit 1 for a
  missing, empty or broken workloads file; the Linux hybrid case summed and marked unchecked (over
  per-type words). Not taken: a fresh TMPDIR for every crate build and `RLIMIT_NPROC` (the
  launcher no longer builds where a crate build can write; nothing else in perf is trusted after
  its hash) [c25, c118].
