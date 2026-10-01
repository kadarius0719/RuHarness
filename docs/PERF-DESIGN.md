# C-vs-Rust performance baselines (design)

Status: **revision 2 — after the check of revision 1 (00f2e23)**, to be checked (2026-09-30).
History: the draft (0b64bba) had an adversarial review (4 lenses, 49 findings, 46 confirmed by two
verifiers); revision 1 answered them; its check (workflow `wf_0f90338d-b1d`) found 41 answered only
in part, 1 made worse, and 33 new findings, condensed into 118 changes (scratchpad
`perf/rev2-changes.md`, numbered there; cited here as [c12]). The §15 spike: DECISIONS.md
"2026-09-30 — C-vs-Rust performance baselines: §15 spike". The person's wish: "baseline the
original C, then compare migrated code side by side as the transition proceeds".

## 1. What it is, in plain words

`harness perf run` measures, on workloads the person names (the inputs and options their program is
really used with):
- **the original C** — its CPU time and memory on each workload: the baseline, from the first day,
  before any unit is accepted;
- **the program as it stands** — the C with every accepted unit's Rust swapped in at once (when two
  or more units are accepted and current): the line that moves as the migration proceeds;
- **each accepted unit** — the C with only that unit's Rust: which unit to look at when the
  program got slower. Per-unit rows do not add up to the program as it stands (each carries Rust's
  fixed start-up once) [c1].

Each side's runs are interleaved with the C's. The answer is one sentence about time — "about as fast
as the C (within 2 %)", "slower by 6 % (4–8 %)", "about 2 % slower — too close to the 2 % line to
call", "can't tell: the runs varied by about 5 % — the computer was busy" — then the details:
CPU time, instructions and memory.

Before any timing, the sides must **end the same way and print the same thing** (each side's folder
written `$PROGDIR`, the run's temp dir `$TMPDIR`). When they do not, that is reported as what it is:
the program behaves differently with that Rust on a real input — a correctness finding `verify`
cannot make (it never runs this workload), not a speed result [c2].

It is information, never a gate: `verify`, `migrate`, `promote` and `bench check` never run it or
read it; it writes no verdict and no plan status. No model is involved (Tier 0). A program that
starts other programs cannot be measured: perf's sandbox refuses it, as the features map's
scenarios do (unlike verify's whole-program check) [c2, c114].

Words: a **side** is one program (the **C**, a **unit's program**, the **program as it stands**); a
**workload** is a command line plus at most one input file; a **run** is one execution of one side;
a **row** is one side measured against the C on one workload.

## 2. Premise

**The spike** (DECISIONS 2026-09-30) on zopfli's u001: instructions +3.24 %, cycles +0.79 %, wall
+3.8 %, footprint equal at 200 KB; the oracle's 30 KB samples are too small to time.

**Checked by the review and the checks** (scratchpad `perf/`, `design-review/`, `design-check/`;
load averages recorded — other agents loaded the machine throughout) [c3–c7]:
- **Counting the program alone under the sandbox** (`perf/premise3/`): a harness-owned launcher
  outside the sandbox forks; the child execs `sandbox-exec` with a profile that allows exec of
  exactly a harness-owned **trampoline** and the program, and denies fork; the trampoline signals
  "ready" on a pipe and waits; the launcher reads `proc_pid_rusage(RUSAGE_INFO_V6)` (the
  **baseline**: sandbox-exec's set-up and the trampoline's start, ≈ 7.5e7 instructions) and says
  "go"; the trampoline sets its CPU-time limit and execs the program **with argv[0] = the program's
  bare name**. Subtracted at the end: an empty program ≈ 1.1e7 instructions, a 1e8-iteration loop
  6.13–6.16e8 (identical binaries, loaded machine: kernel work under preemption moves instruction
  counts by 0.2 % at load 14, 1–1.5 % at load 37–52; up to 6.5 % on a low-IPC loop). The baseline is
  read while the trampoline waits — no race. (`sandbox-exec` alone cannot set argv[0]: it passes
  its operand.)
- **Time under load**: cycles mix performance- and efficiency-core time; normalising to the
  performance-core cost per instruction cuts the per-run spread from 6–8 % to 2.3–3.3 %, but the
  cost per instruction itself rises on runs mostly off the performance cores (runs with under
  half their cycles there read 2–16 % high). In 48 live trials the interval rule (§3.6) never said
  slower or faster for identical programs; at load 13–53 seven runs a side answered in 3 of 20 rows,
  thirty-one in 3 of 4; on a quiet machine (spread ≤ 0.75 %) seven runs answer about 98 % of rows.
- **Memory**: the lifetime maximum footprint clusters (≈ 12.7 / 14.7 MB on zopfli) and its
  minimum is not stable under load (identical binaries read different in 12 of 20 rows); an
  interval with a margin of max(5 %, 1 MiB) gave no false difference in 70+ rows.
- **Rust's fixed start-up** (a unit linked, never called): +1.0e7 instructions, +2.8e6 cycles, +3
  pages (48 KiB).
- **Linking**: two Rust staticlibs from one rustc link into one C program on macOS (ld64 rescans
  archives) and in plan order with GNU ld inside `--start-group … --end-group`; with different
  panic strategies the first archive's runtime silently wins [c6].
- **First execs**: 1.4–8 s for each newly written binary on macOS (also through sandbox-exec).
- **The launcher's build** through `cc` puts its object in `/var/folders`; a same-user process
  swapped it after a passed hash in 23 of 30 tries — the launcher's build is a trust boundary.

## 3. The design

### 3.1 Workloads: `migration/perf/workloads.toml` (`ruharness-perf-workloads`, v1)

```toml
schema_version = 1

[[workload]]
id = "big-text"            # [a-z0-9-], ≤ 24, unique
args = ["-c", "{input}"]   # ≤ 8 strings, each ≤ 256 bytes; `{input}` exactly once when input is set
input = "bench/big.txt"    # optional: a regular file in the target, ≤ 64 MiB
runs = 15                  # optional: runs a side, 5 to 31 (default 15)
```

- At most 16 workloads; the file at most 64 KiB, read with `read_regular`; read as features.toml is
  (unknown keys refused with line and column — the person's file) [c8, c9].
- `{input}` must appear exactly once in `args` when `input` is set, and is refused without it.
- `input` as written: relative, UTF-8, no control character, no component starting with `.` or `-`.
  Resolved: its canonical path lies in the canonical target root, outside the canonical ledger
  folder and any `.git`; a regular file, at most 64 MiB, read **once** per `harness perf run`; every
  run's copy and the digest come from those bytes [c11, c12].
- The workload's digest: blake3 over id, args, the input's name and bytes — **not `runs`** (a row
  records the n it used) [c10].
- An input that is missing, refused or too large affects only its workload: outcome
  **input-unusable**, with the fix in words ("bench/big.txt is not here — put the file back or
  remove the workload"; "bench/big.txt is a link to outside the project — copy the file instead"),
  never stored over an earlier measured row [c13].
- **Written** with `harness perf init` (a starter, refused if a file exists) or the cockpit's
  **Write / Edit your workloads file**: the features Edit flow generalised — its own draft slot
  (`harness-tui-workloads`), "Continue my workloads draft", `harness perf save --expect … --bytes …`
  with the 64 KiB cap. Saving checks the text only (syntax, keys, limits, the name rules); the
  input's existence and size are checked when perf reads it [c12, c16].
- The starter teaches [c15]:
  ```toml
  schema_version = 1
  # Workloads: runs of the whole program long enough to time — half a second or more of the C.
  # Use the options and an input your program is really used with. Put the input file in the
  # project (a real file, not a link; at most 64 MiB; not under migration/ or .git) and commit it,
  # or a fresh clone cannot measure. A program that starts other programs cannot be measured.
  #
  # [[workload]]
  # id = "big-text"
  # args = ["{input}"]          # your program's own options; {input} is the input file's name
  # input = "bench/big.txt"
  # runs = 15                   # 5 to 31
  ```

### 3.2 What is measured, and how it is built

**Units** [c17, c18]: status `verified` or `merged` **and** `status::unit_report(…).fresh_green()`.
A unit with a half-done Accept (a `.promote-<attempt>` marker) is not measured: "an Accept was
interrupted — Re-check it first" (perf does no recovery: it writes no verdict). After perf builds a
unit's crate, its `unit_crate_file_set_hash` must equal the verdict's `inputs.rust_crate`, else "its
Rust changed since verify — Re-check it". Every reason is worded ("its test driver changed since
verify").

**Sides** [c19–c22]:
- the **C**: every top-level `.c` of `source_dir` (a non-regular one refused by name), linked with
  `extra_link_args`;
- **a unit's program**: less the unit's `replaces`, plus its staticlib. A shared helper checks the
  `replaces` entries (each matches a collected top-level `.c`; an empty list refused): a mismatch,
  a crate that does not build, or a program that does not link is that unit's row (outcomes
  `replaces-mismatch`, `crate-does-not-build`, `does-not-link`), and the next unit is measured;
- **the program as it stands**: with two or more measurable units — every one's staticlib, in plan
  order (`--start-group … --end-group` on GNU ld), the linker's own first lines when it fails. One
  unit: "as it stands = u001's row (one unit so far)". None: the C's own row, "no accepted unit to
  compare yet". Its file records the units it holds (`units: [{id, crate}]`) and names those left
  out ("with 3 of 4 accepted units — u-tree left out: verify it first").

**Build** — "as verify builds them: C -O2 -ffp-contract=off, Rust release, no LTO", said in every
result; Rust's fixed start-up is part of what is measured (§6) [c31, c33]:
1. **The launcher and trampoline first** (§3.3), before any target code is compiled or built:
   `cc -c` then link, `TMPDIR` set to a fresh 0700 folder inside `migration/build/.perf/launcher/`,
   which no tool profile and no run lists as writable; their blake3 recorded and checked before each
   run [c23–c25];
2. `whole_cc_into(base, link_args, runner, out, inputs)`, shared with `verify`; the C objects
   compiled once into `.perf/obj/`, each side one link into its slot; every object, staticlib and
   binary hashed after its build and checked before the first run, and every link finished before
   the first run [c26];
3. each step's write set is its own folder (compiles `.perf/obj`, each link its slot, cargo the
   crate's `target/` and `Cargo.lock`), and `.perf/` is made by a `scratch_dir`-style helper that
   refuses links [c23].

**Slots** [c28]: `.perf/bin/<slot>/<name>`, `<name>` the target's program name
(`features::program_name`), `<slot>` four characters: `p000` the C, `pNNN` the unit at plan position
NNN, `pall` the program as it stands (a plan of more than 999 units is refused by name). Every side
runs by its full path from a slot of the same width, with **argv[0] = `<name>`** (the trampoline sets
it). What still differs per side: the executable's own path (`_NSGetExecutablePath`) [c29, c30].

Not measured: the unit's differential driver (mostly `printf` and start-up) — a revisit item.

### 3.3 The launcher (`perfrun`) and the trampoline (`perfgo`) — harness-owned C

`crates/harness-oracle/src/perf/{perfrun,perfgo}.c`, embedded with `include_str!`; a version
constant `PERF_LAUNCHER` in `harness-core`, pinned to the sources by a test. The launcher is the one
harness-built binary that runs unsandboxed; the program runs only inside the sandbox.

`perfrun MODE PROFILE PERFGO DEADLINE CPU PROGRAM NAME ARGS…`:
1. Its **stdin is the write end of a pipe the harness made** (`std::io::pipe`, `Stdio::from`; the
   harness gives the writer only to perfrun's `Command` and reads after the run, the `Command`
   dropped — never on a drain thread); the record goes there, never to a file the program can
   reach [c40].
2. Two more pipes (ready, go); `fork`. The child: stdin `/dev/null`; stdout and stderr the run's
   capture pipes (step 1 runs) or `/dev/null` (timed runs); every other descriptor closed (bounded:
   `close_range`/`PROC_PIDLISTFDS`); then `execv("/usr/bin/sandbox-exec", ["-p", PROFILE, PERFGO,
   ready, go, CPU, PROGRAM, NAME, ARGS…])`. On Linux (no sandbox) the child execs PERFGO directly
   and `PR_SET_PDEATHSIG`, `PR_SET_DUMPABLE 0` are set [c44].
3. **PERFGO** (inside the sandbox): `setrlimit(RLIMIT_CPU, CPU)` (a backstop: the program can
   `setsid()` out of the group), writes one byte on *ready*, reads one on *go*, closes both, and
   `execv(PROGRAM, [NAME, ARGS…])`.
4. On *ready*: macOS `proc_pid_rusage(child, RUSAGE_INFO_V6)` (else V4) — the baseline: instructions,
   cycles, performance-core instructions and cycles, user and system time (mach ticks →
   `mach_timebase_info`), switches; Linux: the counters were opened on the child before it was
   released (`perf_event_open`, `instructions:u`, `cycles:u`, `inherit = 1` — a child's counts arrive
   only when it exits; one event per core PMU on hybrids, read separately; `TOTAL_TIME_ENABLED`/
   `RUNNING` read: running < enabled → no words, never scaled; running = 0 → no counter). Then
   *go*, and the monotonic clock starts [c34–c36, c45].
5. **perfrun owns the program by pid** [c37]: a deadline (DEADLINE, a little under the harness's
   timeout), SIGTERM, or EOF on the record pipe → `SIGKILL` to the child's pid and its group, reap,
   record `timeout`/`stopped`. The pid cannot be reused while unreaped.
6. `waitid(P_PID, child, WEXITED | WNOWAIT)`, then the end counters (same fields, plus
   `ri_lifetime_max_phys_footprint`, `ri_child_*`); `wait4` for `ru_maxrss`. Recorded as **end
   minus baseline**: instructions, cycles, P-core instructions and cycles, CPU time (user + system,
   `cpu_us`), voluntary and involuntary switches; wall from *go* to `waitid` [c39].
7. The record: built in memory, one `write()`, `key value` lines, ASCII, at most 4 KiB, last line
   `end`: status (`ok` or the launcher's failure in words), the program's exit code or signal,
   the counters, the CPU's name (`sysctlbyname("machdep.cpu.brand_string")`; Linux: the harness
   reads `/proc/cpuinfo`), the 1-minute load average; perfrun exits 0 [c40, c47].

**Judging a run** [c41–c43]: (1) the harness's own end — timeout, overflow, cancellation — first;
(2) perfrun did not exit 0, wrote nothing, no `end`, over 4 KiB → **unmeasurable** "the launcher
stopped before measuring" (that row only); (3) no *ready* (the profile was refused, the trampoline
failed) or sandbox-exec's own exit (65, 71) → "the program never started"; `ri_child_*` non-zero or
a `SIGKILL` from the profile's fork rule (below) → "the program tries to start another program;
perf's sandbox does not allow it". A zero baseline is never subtracted.

**The harness side** [c38, c27]: a new runner entry `run_measured(launcher, record_pipe, run_dir,
input, collect)`; perf's run temp dirs 0700 and registered with the signal handler's live-folder
list (as the map's random folder); on timeout, overflow and cancellation the harness sends SIGTERM to
perfrun, waits a bounded grace (250 ms), then SIGKILL to its pid and group —
`kill_live_process_groups` included.

### 3.4 The perf profile

The scenario profile (no network, no signal but to itself, reads denied under the home folder and
the target root except the side's binary, perfgo and the run's temp dir, writes only the temp dir),
with exec allowed for exactly two literals — perfgo and the side's program — and its fork rule
`(deny process-fork (with send-signal SIGKILL))`, so a program that tries to start another is
stopped at once rather than timed failing [c42, c94]. A run cannot read the other side's binary.

### 3.5 Running a row

Per side against the C, per workload [c48–c56]:
1. **Same end and output**: C, other, C (streams captured, rewritten as a scenario's: `$` → `$$`,
   the temp dir → `$TMPDIR`, **each side's own slot folder** → `$PROGDIR`). The two C runs differ →
   **c-unstable** ("the C prints something different each time on big-text: stdout, byte 41 — two
   matching runs do not prove it is stable"). A C-side failure first: crashed, timed out, over the
   output cap ("prints more than 64 MiB; give it an output-file argument"), could not start → its
   own outcome, on every unit. Otherwise the other side differs in its end or its streams →
   **behaves-differently** ("exits 1 where the C exits 0"; "stdout differs at byte 40 961"). A
   non-zero C exit is not refused — it is named: "the C exits 1 on big-text: its error path is
   timed".
2. **Too short**: when **both** sides' step-1 instruction counts are under the floor of **1e9**
   instructions (≈ 0.1–0.3 s; Rust's start-up ≈ 1 % of it) → **too-short**, with the numbers and the
   fix: "too short to time: the C ran 40 ms of CPU on small-text — use an input at least about 25×
   bigger (half a second or more is best)"; the C's exit and output sizes are said, and "check the
   workload's options first" when it wrote nothing. The C under, the other over → measured, marked
   **short run**: never "about as fast"; inside the margin "can't tell on a run this short — use a
   bigger input". No memory line on a short row [c52–c54].
3. **n timed runs a side, interleaved** C, other, C, other, … (n = `--runs` if given, else the
   workload's `runs`), streams to `/dev/null`. No separate warm-up: every binary's first exec was in
   step 1. A timed run that ends differently from its side's step-1 runs ends the row: **run-failed**
   with the side named [c55, c56].

The C pair and the too-short check run once per workload and are shared by every side.

### 3.6 Outcomes (closed set) [c57]

`measured`, `short-run`, `behaves-differently`, `c-unstable`, `c-crashed`, `c-timed-out`,
`output-too-large`, `c-could-not-start`, `too-short`, `run-failed: timeout | exit | signal |
unmeasurable`, `tries-to-start-programs`, `not-verified: <reason>`, `replaces-mismatch`,
`crate-does-not-build`, `does-not-link`, `input-unusable`. Every outcome is a result (exit 0); only
a set-up failure (the launcher or the C does not build, the lock is held, no workloads file) is an
error (exit 1). `not-verified`, `does-not-link`, `input-unusable` are printed but never stored over
a measured row.

### 3.7 The words (std only) [c58–c70]

Per metric, the **Hodges–Lehmann shift**: the median of every pairwise log-ratio
`ln(other_i / c_j)` (n² of them), its distribution-free interval `[D(c+1), D(n²−c)]` from the sorted
pairs, `c` the largest u with `P(U ≤ u) ≤ 0.025` under the exact (n, n) Mann–Whitney null (a
dynamic program in u128; c = 2, 8, 23, 127, 341 at n = 5, 7, 10, 20, 31; confidence 95.0–96.8 %).
It misses the true difference at most 5 % of the time and never gets weaker as runs are added.

- **Time** (the headline). The metric, per row [c59, c60]: **performance-core-normalised cycles**
  (P-core cycles per P-core instruction × all instructions) when every run has V6 counters, a
  P/E split, and at least half its cycles on the performance cores; otherwise raw cycles, said
  ("cycles — runs were split across core types"); never mixed in a row; a missing count is absent,
  never zero; no counters at all → CPU time, said. The assumption is stated: an efficiency-core
  stretch is counted at the performance-core cost. With margin **M = 2 %**:
  - the interval inside ±M → **"about as fast as the C (within 2 %)"**;
  - above +M → **"slower by X % (a–b %)"**; X ≥ 100 % → **"3.7× as slow (3.4–4.0×)"**;
  - below −M → **"faster: takes X % less time (a–b %)"**;
  - a narrow interval straddling M → **"about X % slower (a–b %) — too close to the 2 % line to
    call"** (and the mirror for faster);
  - otherwise → **"can't tell: the runs varied by about X %"** + the cause: "— the computer was
    busy (up to N % of the runs on efficiency cores; close other work and measure again)" when most
    runs were mostly off the performance cores, else "— measure again with more runs: harness perf
    run --unit u001 --workload big-text --runs 31" while n < 31; at n = 31: **"no clear difference:
    the runs' noise allows up to X %"** [c61–c64].
- **Instructions** (details; never speed words; M = 1 %): "about the same instructions", "X % more
  instructions (a–b %)", "X % fewer", or **"about X % more instructions (not certain: a–b %)"** when
  the interval crosses ±1 % [c65].
- **Memory** (details; macOS lifetime maximum footprint, Linux maximum RSS — named): the same
  interval with margin **max(5 %, 1 MiB)**: "about the same memory", "uses about X % more memory
  (a–b %)", "less", or "can't tell — memory varied from run to run" [c66].
- **CPU time shown**: the median `cpu_us` a side (user + system, the sandbox's set-up subtracted),
  labelled "CPU time"; clock time only when there are no counters [c67].
- A unit row that says "about as fast" adds "(perf cannot tell whether big-text runs this unit's
  code)" [c68].

The CLI, per row (two lines, under 80 columns):
```
perf: u001-katajainen on big-text — slower by 6 % (4–8 %)
      CPU 1.21 s → 1.28 s · 3.2 % more instructions · memory about the same · 15 runs each
```

### 3.8 Results: `migration/perf/program.json` and `migration/perf/units/<id>.json` (`ruharness-perf`, v1)

One row per workload; a measurement replaces only the rows it measured. Each row [c71–c79]:
- `workload`, `outcome`, `runs`, `platform_metrics` (`macos-v6-pnorm`, `macos-cycles-mixed`,
  `linux-cycles`, `linux-hybrid`, `cpu-time`), `units` (program.json only);
- `inputs`: the workload digest, the program digest, the crate digest(s), the unit's `replaces` and
  `program_name`, `recipe` (`perf-recipe-1`), `launcher` (`PERF_LAUNCHER`), `computer` (OS name and
  build, arch, CPU), `compilers` (the first line of `cc --version` and of `rustc -V`); the facts digest
  is not an input (the program digest covers the C);
- `c` and `other`: arrays of exactly `runs` integers each — instructions, cycles, p_instructions,
  p_cycles, cpu_us, wall_us, memory, max_rss, switches_voluntary, switches_involuntary, load — a
  missing counter absent, never 0; early-stop rows keep what step 1 measured;
- `first_difference` (behaves-differently only): numbers only — the stream (`stdout`, `stderr`,
  `exit`), both lengths, the byte offset, both exit codes; the strict reader refuses text there.
**No words are stored**: they are computed from the numbers by §3.7 on every read. Read strictly
(`read_regular` with a cap; arrays of the stated length; p_instructions ≤ instructions, p_cycles ≤
cycles, both zero or neither; ids from the alphabet, once); written through `safe_ledger_dir` and
`write_atomic` after every row; a linked `migration/perf` refused. Rows of removed workloads are
dropped on the next write; a unit file of a unit no longer in the plan reads "no longer in the plan".

**Kept outputs** of a behaves-differently row: `migration/build/perf-out/<p|unit>/<workload>.
{c,rust}.{stdout,stderr}` (capped, links refused, in no tool profile), replaced when that row is
re-measured, removed when it no longer differs; the row records each file's size and blake3 [c74].

**Current** iff each input equals today's, each with its own reason: "your workload changed", "the
program's C changed", "the unit's Rust changed", "the system was updated (26.5.2 → 26.6)", "measured
with other compilers (rustc 1.90.0 then, 1.91.0 now)", "measured on another kind of computer (Apple
M1)", "made by another version of the harness". `harness perf run` judges all; the cockpit judges
what it can without starting a process and shows "measured on <computer> with <compilers>". A
results file is evidence, not ledger truth; committing it keeps a history in git; a committed file
whose digests match reads current — it gates nothing, and the next measurement replaces it [c78,
c79]. Stale facts: `perf run` refuses ("scan first"), as `features map` does.

### 3.9 The CLI [c80–c84]

- `harness perf run [--target T] [--unit ID]… [--workload ID]… [--runs N (5–31)]
  [--as-it-stands-only] [--allow-unsandboxed] [--json]` — the writer lock; without `--unit`, the C,
  the program as it stands and every measurable unit; `--unit` measures only those units (no
  as-it-stands row; `--as-it-stands-only` conflicts with it). Unknown ids: exit 1 naming the known
  ones. No workloads file or no workload: exit 1, "write your workloads file first (harness perf init
  gives a starter)".
- `harness perf init`, `harness perf save --expect … --bytes …`, and a read-only `harness perf show`
  (the words of the stored rows, current or out of date).
- Progress (`message` events): "building the launcher…", "building the C program…", "u001 —
  building its Rust…", "big-text — checking the C ends the same way twice (CPU 1.21 s)…", "u001 on
  big-text — timed run 9 of 30…", "keep the computer quiet while it measures". A
  behaves-differently row is printed first and in full, with where both outputs are kept. Summary:
  "perf: measured 4 rows, 1 too short, 0 behave differently — wrote migration/perf/ (commit it to
  keep a history)".
- Events: `perf-row {side: "program"|"unit", unit, workload, outcome, words}` — `words` a
  display-only courtesy, recomputed by every reader.

### 3.10 The cockpit [c85–c92]

- A **Speed** group in the tree beside Features: `Speed (no workloads yet)`, `Speed (none
  measured)`, `Speed (2 of 3 units measured)`; acts **Write your workloads file** / **Edit the
  workloads file** / **Continue my workloads draft**, **Measure speed**, and on a unit **Measure
  this unit's speed** and **Measure again with more runs**. Each confirm dialog says what runs, how
  many times, an estimate (§6, an upper bound), that it writes `migration/perf/` and no verdict, that
  Cancel keeps finished rows, and to keep the computer quiet.
- The **Speed View** (golden at 54 columns, inside an 80-column terminal):
  ```
  Speed — your workloads, the C against the Rust
  on this Mac (Apple M3, rustc 1.90.0), 15 runs each,
  as verify builds them

  The original C
    big-text      CPU 1.21 s · 12.4 MB
    many-small    CPU 0.31 s · 3.1 MB
  As it stands (2 of 3 units)
    big-text      slower by 6 % (4–8 %)
    many-small    about as fast (within 2 %)
  u001-katajainen
    big-text      slower by 6 % (4–8 %)
    many-small    about as fast (within 2 %)
  u002-hash
    big-text      too short to time
  ```
  Units sorted worst first; ids cut with …; "out of date: …" under a stale row, its words dimmed.
- A unit's header: `Speed: slower by 6 % on big-text (4–8 %)` (its worst row of today's workloads).
- A **behaves-differently** row: a fact on the unit — "With u001's Rust the program prints
  differently on big-text (stdout, byte 40 961) — verify does not run this workload: see Speed →
  Compare the outputs" — no Re-check (it would pass and loop); it clears when the row is
  re-measured the same or goes out of date. **Compare the outputs** shows the kept files side by
  side when they exist and match the row, else "the two outputs are not on this computer — measure
  this unit's speed again". An as-it-stands difference goes in the project summary [c86–c88].
- A slower row's next step, as words: "if it matters, Modify with a note about speed, or Retry —
  then measure again" [c91]. Help gains a Speed section and the glossary (CPU time,
  instructions, memory, performance cores) [c90].
- Acts' argv: `with_sandbox_flag(harness_argv(["perf", "run", …, target_arg]))`.

### 3.11 Security (summary) [c93, c94]

The program runs only under the perf profile: no fork (killed on trying), no signal out, no network,
no reads under the home folder or the target beyond its binary, perfgo and its temp dir, writes only
its temp dir. The launcher is the one unsandboxed harness binary: built first, through its own
private folders, hash-checked before each run. perfrun owns the program by pid; the trampoline's CPU
limit bounds a program that `setsid()`s out of the group. The counters return on a pipe the program
never holds (macOS). The input is read once, confined and bounded; results carry no words and are
read strictly — but a committed results file can be forged (it gates nothing). Run temp dirs under
`TMPDIR` can be rewritten by a same-user process that outlived a build (as every run's today); the
program's counts are whatever its behaviour makes them. No new dependency; `unsafe` Rust stays
forbidden everywhere.

## 4. Tests and checks [c95–c107, c111]

- **The words**: the critical values (c = 2, 8, 23, 127, 341; positions D(3)/D(23), D(9)/D(41),
  D(342)/D(620)) with ±1 mutations failing; a synthetic A-vs-A at 0.5 % noise "about as fast" at
  every n; the interval narrows as n grows; near-the-line, busy-cause and n = 31 words; live A-vs-A
  never slower or faster; fallbacks (V4, p_instructions = 0, a small P-share, a self-backgrounding
  program) never panic or divide by zero; the detail line never contradicts the headline.
- **Memory**: the same allocating binary on both sides under load never "more/less".
- **The floor**: either side of 1e9; the C under and the other over (short run); both under.
- **The launcher and trampoline**: an exact baseline (a test-only delay before *go* changes no
  count); argv[0] = the bare name; a self-exec'ing program; a refused profile, a child ending early
  → unmeasurable; an empty program never reads 0; a `setsid()`/`setpgid()` spinner dead after timeout,
  cancel, overflow and the harness being killed; a record cut short, a timeout, exit 0 with no record
  — each judged as §3.3 says; the launcher's folder and build TMPDIR in no other profile.
- **The profile**: `system`, `popen`, `posix_spawn`, `fork`, `vfork` each killed; no signal out; a
  run cannot read the other side's binary.
- **Paths**: the same binary at `p000` and `p012`: argv[0], output and memory equal.
- **Workloads, results, evidence, units**: each rule of §3.1–§3.2 and §3.8 (the caps, `{input}`, a
  missing input skips only its workload, the digest ignores `runs`, links refused, never-ran rows
  never stored over measured ones, removed workloads dropped, a half-done Accept and a stale unit not
  measured, an empty `replaces`, a non-linking unit's row while the next is measured, zero and one
  unit never C against C).
- **End to end on zopfli**: a generated 1 MB input; a time-printing C stays c-unstable-free under C,
  other, C only when it is stable; a crashing C → a C-side outcome on every unit; equal streams but
  exit 1 → behaves-differently; Compare after another unit's measurement; the cockpit's golden View.
- **Mutation checks** of §3.5's order, §3.6's outcomes and §3.7's rule; re-run §2's premise with the
  built launcher under load.

## 5. Order of work [c108–c110]

Each step committed green: (a) the launcher, trampoline and profile, with the harness's
`run_measured` and the SIGTERM-then-SIGKILL change; (b) the workloads and results files in
`harness-core` (`PERF_LAUNCHER`, the strict readers); (c) the shared build refactors
(`whole_cc_into`, objects once, the `replaces` helper, slots, hashes); (d) rows and words in
`harness-oracle`; (e) the CLI; (f) the cockpit (Speed group and View, the generalised Edit flow);
(g) SCHEMAS (writer rows for `workloads.toml`, `migration/perf/**`, `migration/build/.perf/**` and
`perf-out/**`, the crate `target/**` and `Cargo.lock`; the `perf-row` event; results are forgeable
and non-canonical), the tutorial, the testing guide's Part 11 on liblzg (make an input, time the C by
hand, write the workloads file, measure, read the words — a word table, u-version as the unit no
workload reaches). Then the code review, fix passes each checked, mutation checks, the handoff.

## 6. Residuals and costs [c112–c117]

- **Rust's fixed start-up** (≈ 1e7 instructions, 2.8e6 cycles, 48 KiB) is in every unit's and the
  program-as-it-stands's numbers: near the floor it reads as about 1 % more instructions or a little
  more memory. A unit no workload reaches therefore does not read exactly "the same".
- **A workload that never runs a unit's code** reads "about as fast" — said on the row; revisit with
  the features probe telling which units a workload reaches.
- **Behaves-differently is not a verify check**: scenarios take only the oracle's samples; revisit a
  file-input scenario so the finding becomes one.
- **Multi-process programs** cannot be measured (the profile kills a fork).
- **Noise**: memory rises with load; the same code can read "about as fast" one day and "can't
  tell" the next; two matching C runs do not prove the C stable; the efficiency-core assumption
  (§3.7); kernel instructions under preemption (0.1–2 %); the executable's path differs per side.
- **One kind of computer**: results compare only on the same kind (OS build, arch, CPU); history
  only through git.
- **Security**: committed results can be forged; run temp dirs under `TMPDIR`; a crate's `build.rs`
  and `.cargo/config.toml` are outside the crate digest (as for verify).
- **Linux** (`--allow-unsandboxed`): no sandbox — the program can fork (its children counted by
  `inherit` only when they exit), signal, rewrite binaries, forge or suppress the record through
  `/proc` unless `PR_SET_DUMPABLE` holds; counts are user-mode only (a syscall-heavy change can read
  "about as fast"); hybrid CPUs' per-PMU counts are read apart, but runs that cross core types mix as
  macOS's did before normalisation — **unchecked here** (no Linux machine).
- **Cost** (an upper bound the dialog shows): per `perf run` — the launcher build, one compile of
  the C, per unit one crate build (usually warm) and one link, the as-it-stands link; each new binary's
  first exec (1.4–8 s on macOS, none on Linux); per workload 2 C runs; per row 1 + 2n runs (n = 15:
  31), each about the program's time plus 0.05–0.1 s under load. For a 1.2 s C and n = 15: ≈ 75 s a
  row.

## 7. What changed from revision 1

The launcher's baseline through a trampoline (exact, no kqueue race) that also sets argv[0] and a CPU
limit; the launcher built first through private folders; perfrun owning the program by pid; the
harness's SIGTERM-then-SIGKILL; the profile killing a fork; same end and output with C, other, C and
C-side outcomes; a floor of 1e9 on both sides and short runs; no warm-up; 15 runs by default; the
time metric chosen per row with the cause words, near-the-line words, n = 31 words; instructions'
uncertain branch; memory by interval; CPU time shown; outcomes widened; results files and kept
outputs reshaped; the original C as its own row and the program as it stands only with two units;
slots of one width; perf does no recovery; the cockpit's 54-column View, alert without Re-check,
the generalised Edit flow; the CLI's `run`/`show`; §6's costs restated.

## 8. Not decided here (for the person)

None.

## 9. Review record

- **The draft (0b64bba)**: 4 lenses (measurement, security, integration, the person), 49 findings,
  46 confirmed by two verifiers, 2 by one, 1 refuted (`wf_fe7fb255-d44`; `perf/design-review-result.json`).
- **Revision 1 (00f2e23)** checked (`wf_0f90338d-b1d`; `perf/revision1-check.json`): of the draft's
  findings 7 resolved, 41 in part, 1 made worse (the launcher's build); 33 new, all confirmed by two
  verifiers (the launcher's build path, setsid escape, NOTE_EXEC races, argv[0] through
  sandbox-exec, memory's minimum, P-core normalisation, the floor, outcomes, merge rules, the View's
  width, …), 1 disputed (recovery as a write), 3 refuted. The 118 changes they ask for:
  `perf/rev2-changes.md`; this revision takes each, choosing where the reviewers gave options: the
  trampoline (over ptrace or a kqueue hold), the launcher's private build folder inside `.perf/`
  (over a folder outside the target: no tool profile lists it), 15 runs (over a growing n), memory
  by interval (over ranges only), CPU time shown, no warm-up, the original C as a row (over no row),
  no recovery (over recovering and saying so).
