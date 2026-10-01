# C-vs-Rust performance baselines (design)

Status: **revision 1 — after the adversarial design review of the draft (0b64bba)**, to be
checked (2026-09-30). The §15 spike and its premise run: DECISIONS.md "2026-09-30 — C-vs-Rust
performance baselines: §15 spike". The person's wish (DECISIONS M4 roadmap notes): "baseline the
original C, then compare migrated code side by side as the transition proceeds". The review (4
lenses — measurement, security, integration, the person — 55 findings, each checked by two
verifiers) and what this revision did with each: §9.

## 1. What it is, in plain words

`harness perf` answers two questions on workloads the person names (the inputs and options their
program is really used with):
- **How fast is the program as it stands?** — the original C against the program with *every*
  accepted unit's Rust swapped in at once: the line that moves as the migration proceeds;
- **What did each unit's Rust do?** — the original C against the program with only that unit's
  Rust: which unit to look at when the program got slower.

Each side runs several times, interleaved, from the same path under the same name. The answer is
one sentence about time ("about as fast as the C", "slower by 6 % (4–8 %)", "can't tell yet — add
runs"), then the details: time, instructions and peak memory, each side. Before any timing, both
sides must print the same thing; when they do not, that is reported as what it is — a behaviour
difference `verify` missed, the most important thing perf can find.

It is information, never a gate: `verify`, `migrate`, `promote` and `bench check` never run it and
never read it. No model is involved (Tier 0). It runs only with the sandbox (macOS), or with
`--allow-unsandboxed` (Linux, §3.9).

Words: a **side** is one program (the **C program**, all C; a **unit's program**, the C with that
unit's Rust; the **program as it stands**, the C with every accepted unit's Rust); a **workload**
is a command line plus at most one input file; a **run** is one execution of one side on one
workload; a **row** is one side pair on one workload.

## 2. Premise

The spike (DECISIONS 2026-09-30): zopfli's u001-katajainen, the two programs `verify` builds, a
200 KB input, 5 interleaved runs a side: instructions +3.24 % (range 0.022 %), cycles +0.79 %
(range 1.7 %), wall +3.8 % (3.8 %), footprint equal. The oracle's samples are too small to time;
instructions and cycles can disagree (IPC), so instructions alone never decide.

Checked by the review and this revision (scratchpad `perf/`, `design-review/`):
- **Two Rust staticlibs link into one C program** on macOS (ld64), with different panic
  strategies: the program as it stands is buildable. On Linux (GNU ld) two copies of Rust's
  runtime can collide; the link is tried, and a failure is said in words (§3.2).
- **Under this machine's normal load, cycles mix efficiency- and performance-core time**: the
  cycles spread exceeds 3 % on most runs. macOS's `RUSAGE_INFO_V6` (in the SDK header; the
  launcher is C) gives performance-core instructions and cycles separately (§3.3).
- **Peak footprint is not stable on a program that allocates** (18.5–21.9 MB over 12 zopfli runs)
  and moves by a 16 KiB page with the program's path length; it gets its own rule (§3.6).
- **The first exec of a freshly written binary takes 4–8 s here** (the system's first-run check):
  builds and warm-up dominate a small row's cost (§6).
- **The measured program must not fork**, and the counters must not pass through a file it can
  write (§3.3, §3.4).
- **Counting from the program's own exec under the sandbox works** (scratchpad
  `perf/premise2/launch.c`): a launcher outside the sandbox forks, its child execs `sandbox-exec`
  with a profile that denies fork; `kqueue` `NOTE_EXEC` fires twice (sandbox-exec, then the
  program); `proc_pid_rusage(RUSAGE_INFO_V6)` at the second is the sandbox's set-up (≈ 6.4e7
  instructions), subtracted at the end. A near-empty program then reads ≈ 1.0e7 instructions (its
  own start-up), a 1e8-iteration loop 6.098e8, repeated to 0.01 %; the performance-core fields are
  filled.

## 3. The design

### 3.1 Workloads: `migration/perf/workloads.toml` (`ruharness-perf-workloads`, v1)

```toml
schema_version = 1

[[workload]]
id = "big-text"            # [a-z0-9-], ≤ 32, unique
args = ["-c", "{input}"]   # ≤ 8 strings, each ≤ 256 bytes; `{input}` is the input's file name
input = "bench/big.txt"    # optional: a regular file inside the target, ≤ 64 MiB
runs = 7                   # optional: runs a side, 5 to 31 (default 7)
```

- Read as features.toml is (the person's file; unknown keys refused with the key named — the
  global "ignore unknown fields" rule is for files tools write).
- `input`: resolved against the target root; its canonical path must lie inside the canonical
  root, outside the canonical ledger folder and outside any folder named `.git` at any depth; the
  file name is UTF-8, has no control character, does not start with `.` or `-`. It is read
  **once** per `harness perf` with `read_regular` (no FIFO, no device, at most 64 MiB); every run's
  copy and the workload's digest come from those bytes. The workload's digest is blake3 over its
  id, args, runs, the input's file name and bytes.
- Written by the person: `harness perf init` (a starter that validates and teaches, below), or the
  cockpit's **Write / Edit your workloads file** — the features file's Edit flow reused (nano,
  in-process validation with line and column, `harness perf save --expect --bytes`).

The starter (no workload: it validates, and `harness perf` says what to add):
```toml
schema_version = 1
# Your workloads: runs of the whole program long enough to time — half a second or more of the
# C program is good. Use the options and an input your program is really used with; put the input
# file in the target (for example bench/big.txt) and name it here.
#
# [[workload]]
# id = "big-text"
# args = ["-c", "{input}"]     # {input} is the input file's name in the run's folder
# input = "bench/big.txt"
```

### 3.2 What is measured

**Which units.** After the writer lock, `promote::recover_promotion` runs for each unit perf
touches (a half-done promotion is finished or undone first). A unit is measured only when its
evidence is green and current — `status::unit_report(…).fresh_green()` — and, after perf builds
its crate, the crate's `unit_crate_file_set_hash` equals the verdict's `inputs.rust_crate`; else its
rows say "not measured: verify this unit first" with the reason. Never an attempt's candidate
(measuring a green attempt before Accept is a revisit item, §6).

**The sides.**
- the **C program**: every top-level `.c` of `source_dir`, linked with `extra_link_args`;
- **a unit's program**: the same less the unit's `replaces`, plus the unit's staticlib. A unit
  whose `replaces` entries do not all match collected top-level `.c` files is refused with
  `verify`'s own words — the check moves from `whole_program()` into a shared helper so no caller
  skips it;
- **the program as it stands**: less every measured unit's `replaces`, plus every measured unit's
  staticlib (in plan order). A link that fails is a row "the units' Rust does not link together
  here (two copies of Rust's runtime)" — the per-unit rows still stand.

**How they are built** — as `verify` builds them, said in every result ("as verify builds them:
C -O2, Rust release, no LTO"): `cc -O2 -ffp-contract=off -w`, the crate by
`build_crate_staticlib` (`cargo build --release --offline`, the crate's own profile). Shared code,
new refactors named:
1. `whole_cc_into(base, link_args, runner, out, inputs)` — one compile-and-link both `verify` and
   perf call (today `build_whole_c`/`_mixed` write `prep.build/whole_*` and need a unit);
2. the C objects compiled **once** per `harness perf` (`-c` per file, the same flags), then every
   side is one link — one compile of the C, one crate build per unit, one link per side;
3. `migration/build/.perf/` made by a `scratch_dir`-style helper that refuses links, with its own
   tool profile (write dirs: `.perf/obj`, `.perf/bin`, the crate's `target/`);
4. the launcher (§3.3) and the C objects are built **before** any crate build, into folders no
   crate build's profile lists; every binary's blake3 is recorded after its build and checked
   before each run.

Each side's binary lands at `.perf/bin/<side>/<name>`, `<name>` the target's program name
(`features::program_name`) and `<side>` a fixed two-letter folder (`c0`, `u1`, `u2`, … , `as`) —
**the same path length and file name on every side**, and `argv[0]` is `<name>` on every side, so a
program that prints its own name, or whose footprint follows its path, compares like for like.

Not measured: the unit's differential driver (its time is mostly `printf` and process start-up;
a revisit item, §6).

### 3.3 The launcher (`perfrun`, harness-owned C)

`crates/harness-oracle/src/perf/perfrun.c`, embedded (`include_str!`), compiled by `harness perf`
with `cc -O2 -w`, no target include folder, into `.perf/launcher/`. A version constant
`PERF_LAUNCHER` in `harness-core` (like `MAP_PROBE`), pinned to the source by a test.

**It runs outside the sandbox; the program runs inside it.** The launcher is harness code; the
program is not. `perfrun MODE PROFILE PROGRAM ARGV0 ARGS…`:
1. opens nothing in the run's folder. Its **stdin is the write end of a pipe the harness made**
   (`std::io::pipe`, `Stdio::from` — no `unsafe`); the record goes there, never through a file the
   program could write, block (a FIFO) or forge;
2. `fork`s. The child points its stdin at `/dev/null`, its stdout and stderr at the run's capture
   pipes (same-output runs) or at `/dev/null` (timed runs, mode `timed`), closes every other
   descriptor, and `execv`s `/usr/bin/sandbox-exec -p PROFILE PROGRAM` with `argv[0] = ARGV0`.
   PROFILE is the **scenario profile** with PROGRAM as its one exec literal — no fork, no signal
   but to itself, no network, reads only the program and its folder, writes only the run's temp
   dir. On Linux (no sandbox, `--allow-unsandboxed`) the child execs PROGRAM directly;
3. **macOS**: `kqueue` `EVFILT_PROC NOTE_EXEC` on the child: at `sandbox-exec`'s exec into the
   program, `proc_pid_rusage` gives the **baseline** (the sandbox's own set-up: ~6e7
   instructions), subtracted from the end counts; **Linux**: before the exec, the parent opens
   `perf_event_open` counters on the child (`instructions:u`, `cycles:u`, `enable_on_exec`,
   `inherit = 1` — every thread and every process it starts; one event per core PMU on a hybrid
   CPU, summed; `TOTAL_TIME_ENABLED`/`RUNNING` read and a multiplexed count marked as such);
4. `waitid(P_PID, child, WEXITED | WNOWAIT)`, then **macOS** `proc_pid_rusage(RUSAGE_INFO_V6)`
   (falling back to `V4`): instructions, cycles, performance-core instructions and cycles,
   `ri_lifetime_max_phys_footprint`, `ri_child_*` (work of processes it started — non-zero means
   "starts processes", §3.5); **Linux**: read the counters;
5. `wait4`: user and system time, `ru_maxrss` (bytes on macOS, KiB on Linux — recorded as
   `max_rss` bytes), voluntary and involuntary context switches; the monotonic clock from fork to
   `waitid` gives the wall time;
6. writes one record to the pipe — `key value` lines, ASCII, at most 4 KiB, including the CPU's
   name (`sysctlbyname("machdep.cpu.brand_string")`; on Linux the harness reads `/proc/cpuinfo`),
   the child's exit code or signal, and a `status` (`ok`, or the launcher's own failure in words) —
   and exits 0. Every outcome of the program, and every failure of the launcher to measure it,
   is in the record: no exit code or stream of the program's can pass for the launcher's.

The harness runs perfrun by path through the runner (allowlist-exempt, as built binaries are),
under no sandbox, its timeout the run's (`timeout_secs`), its process group killed on timeout and
on a signal; perf's run temp dirs are registered with the signal handler's live-folder list (as
the map's random folder is) and made 0700.

### 3.4 Running a row

Per side pair and workload:
1. **Same output first** (streams captured as a scenario's, `$TMPDIR` rewritten; both sides from
   their fixed paths with `argv[0] = <name>`): the C program **twice** — if its two runs differ, the
   row's outcome is **c-unstable** ("the C program prints something different each time on
   big-text: stderr, line 2 — remove the option that prints times or random values"); then the
   other side once — if it differs from the C, the outcome is **behaves-differently**: both
   outputs are kept (`.perf/out/<side>/<workload>.{c,rust}.{stdout,stderr}`), the first difference
   is quoted, and the row is a **correctness alert**, not a speed result (§3.8, §3.9).
2. **Too short?** If the C's two runs' median instructions are under **5e8** (≈ 50–100 ms; one
   floor, on a quiet metric, both platforms' meaning of "instructions" noted in §3.6), the row stops
   here with outcome **too-short** and the numbers: "too short to time: the C program ran 4 ms on
   small-text; it needs about 50 ms — use an input about 15× bigger". (No counters at all — Linux
   without a PMU — uses 50 ms of wall time instead, said so.)
3. **Warm-up**: one more run of the other side, discarded (its first exec; the C's first exec was
   step 1).
4. **n timed runs a side, interleaved** C, other, C, other, … (n = the workload's `runs`), streams
   to `/dev/null`. A run whose record says the program now fails, times out, was killed, started
   processes, or could not be measured ends the row with that outcome and its words.

### 3.5 Outcomes (closed set)

`measured`, `behaves-differently`, `c-unstable`, `too-short`, `starts-processes` (the record's
child-work fields are non-zero: their work is not counted, so no words), `run-failed: <timeout |
exit | signal | unmeasurable>`, `not-verified` (§3.2), `does-not-link` (the program as it stands).
Every outcome is a result (exit 0); only a set-up failure — the launcher does not build, the C does
not build, the lock is held — is an error (exit 1).

### 3.6 The words (std only)

Per metric, the **Hodges–Lehmann shift**: the median of every pairwise log-ratio
`ln(other_i / c_j)` (n² pairs), with its **distribution-free 95 % interval** from the order
statistics of those pairs at the exact Mann–Whitney critical value for (n, n) — a table for
n = 5…31 (about 40 lines). The interval narrows as runs are added, so "add runs" is honest advice;
its false-alarm level is 5 % at every n. Shown as percentages (`e^x − 1`).

- **Time** (the headline; on macOS **performance-core-normalised cycles** — performance-core
  cycles per performance-core instruction × all instructions — so a run that strayed to an
  efficiency core does not read as the program's; on Linux `cycles:u`; with no counters, wall
  time, said so). With margin **M = 2 %** (Perfherder's minimum; LLVM's tracker uses about 2.5 %):
  - the interval inside ±M → **"about as fast as the C (within 2 %)"**;
  - the interval above +M → **"slower by X % (a–b %)"**; above +25 % → **"much slower by …"**;
  - the interval below −M → **"faster: takes X % less time (a–b %)"** (X = 1 − other/C);
  - otherwise → **"can't tell yet — add runs (now 7 a side)"**.
- **Instructions** (detail line, never speed words): "X % more instructions" / "X % fewer" /
  "about the same instructions", the same interval rule with M = 1 %. On macOS the count includes
  the kernel's work for the process (exec, page faults, system calls) less the sandbox's set-up;
  on Linux it is user-mode only — the two never compare, and the results file names which.
- **Memory** (detail line): the **minimum** of the n runs a side (the least disturbed run), "uses N
  KiB more (X %)" / "less" when the difference is at least 2 pages and 1 %, else "about the same
  memory"; macOS's lifetime maximum footprint, Linux's maximum RSS — named per platform.
- **A short run** (median under the floor but above 0.2 × it, reached only when §3.4 step 2's run was
  near the floor) never says "about as fast"; a clear difference is still said, marked "(short run)".

The CLI's two lines per row:
```
perf: the program as it stands on big-text — slower by 6 % (4–8 %)
      time 0.26 s → 0.28 s · 3.2 % more instructions · memory 12.4 MB → 12.4 MB · 7 runs each
```

### 3.7 Results: `migration/perf/<side>.json` (`ruharness-perf`, v1)

One file per side (`program.json` for the program as it stands, `<unit>.json` per unit), holding one
row per workload; a measurement **replaces only the rows it measured** (merged by workload id):
re-measuring one workload, or stopping part-way, keeps the others, each with its own inputs.
```json
{ "schema": "ruharness-perf", "schema_version": 1,
  "rows": [ { "workload": "big-text", "outcome": "measured", "runs": 7,
              "inputs": { "workload": "blake3:…", "program": "blake3:…", "facts": "blake3:…",
                          "crates": ["blake3:…"], "recipe": "perf-recipe-1",
                          "launcher": "perfrun-1",
                          "computer": "macos 26.5.2 (25F…) · arm64 · Apple M3",
                          "compilers": "Apple clang 21.0.0 · rustc 1.90.0" },
              "platform_metrics": "macos-v6",
              "c":     { "instructions": […], "cycles": […], "p_instructions": […],
                         "p_cycles": […], "wall_us": […], "user_us": […], "system_us": […],
                         "memory": […], "switches": […] },
              "other": { … },
              "first_difference": null } ] }
```
- **No words are stored**: they are computed from the numbers by §3.6's rule on every read, so a
  results file cannot say what its numbers do not (a hostile committed file can only lie in its
  numbers; see the next point). Read strictly: bounded arrays of integers, outcomes from the
  closed set, inputs of bounded size.
- **Current** iff each input equals today's, each with its own reason in words: "your workload
  changed", "the program's C changed", "the scan changed", "the unit's Rust changed",
  "measured with other compilers (rustc 1.90.0 then, 1.91.0 now)", "measured on another computer
  (Apple M1)", "made by another version of the harness". `harness perf` judges every input; the
  cockpit judges what it can compute without starting a process (workload, program, facts,
  crates, recipe, launcher) and shows the rest as "measured on <computer> with <compilers>". Stale
  facts (`program_digest_now`'s sentinel): `harness perf` refuses ("scan first"), as `features map`
  does; comparisons use `features::same_program`.
- Measurements, not timestamps (SCHEMAS' rule for committed files holds); machine-bound evidence,
  not ledger truth — a fresh clone resumes without it. Commit it to keep a history in git, or not;
  a result made on another computer always says so.
- Written through `safe_ledger_dir(root, ["migration", "perf"])` and `write_atomic`, after every
  row.

### 3.8 The CLI

`harness perf [--target T] [--unit ID]… [--workload ID]… [--runs N] [--as-it-stands-only]
[--allow-unsandboxed] [--json]`: the writer lock; without `--unit`, the program as it stands and
every measurable unit; without `--workload`, every workload. Progress lines (and `message`
events): `perf: building the C program…`, `perf: u001-katajainen — building its Rust…`, `perf:
u001-katajainen on big-text — checking both give the same output (the C took 0.26 s)…`, `perf:
… — timed run 6 of 14…`; per row the two lines of §3.6; a summary: `perf: measured 3 rows, 1 too
short, 0 behave differently — wrote migration/perf/`. A **behaves-differently** row is printed
first and in full, with its first difference and where both outputs are kept. Events: `perf-row`
(side, workload, outcome, the headline words — no numbers, no output). `harness perf init` (the
starter, the lock taken, refused if one exists); `harness perf save --expect … --bytes …` (the
cockpit's Edit flow).

### 3.9 The cockpit

- A **Speed** group in the tree beside Features: `Speed (none yet)` / `Speed (2 of 3 units
  measured)`; its acts: **Write your workloads file** / **Edit the workloads file**, **Measure
  speed** (everything), and on a unit **Measure this unit's speed**. Each opens a confirm dialog:
  what runs, how many times, an estimate (§6), that it changes no verdict, and to keep the
  computer quiet meanwhile.
- The Speed View, at 80 columns:
  ```
  Speed — the original C against your accepted units' Rust
  Measured on this Mac (Apple M3 · rustc 1.90.0), 7 runs each, as verify builds them.
  The original C:    big-text 0.26 s · 12.4 MB    many-small 0.08 s · 3.1 MB
  As it stands:      big-text slower by 6 % (4–8 %)    many-small about as fast
  u001-katajainen    big-text slower by 6 % (4–8 %)    many-small about as fast
  u002-hash          big-text too short to time        many-small can't tell yet — add runs
  ```
  (the worst row a unit has, first; "out of date: …" under a stale row, the old numbers dimmed).
- A unit's header gains `Speed: slower by 6 % on big-text (4–8 %)` — its worst row.
- A **behaves-differently** row is a correctness alert on the unit: the unit's header says
  `Its program prints something different from the C on big-text (stdout, line 3) — see Speed`,
  and the Speed View offers **Compare the outputs** (the two kept files, side by side).
- No Next-step rule (speed is never the next step), except that a behaves-differently row puts
  the unit's own Re-check first.

### 3.10 Security (summary)

The program runs under the scenario profile (no fork, no signals out, no network, no home reads,
writes only its temp dir); the launcher is harness code built before any target code and
hash-checked before each run; the counters return on a pipe the program never holds; the input is
read once, confined and bounded; results files carry no words and are read strictly; nothing new
leaves the sandbox. No new dependency (the launcher is C built with `cc`, the statistics `std`);
`unsafe` Rust stays forbidden in every crate. **Linux** has no sandbox: perf there runs only with
`--allow-unsandboxed`, and every guarantee above that the sandbox gives is absent (§6).

## 4. Tests and checks

- **The words** (§3.6): Hodges–Lehmann and its interval against a hand-worked table (n = 5, 7, 31;
  the critical values), an A-vs-A pair (the same binary as both sides: "about as fast" at every n),
  shifts of 1 %, 3 %, 30 %; the margins and the short-run rule; each bound mutated must fail one.
- **The launcher**: built and run on a busy loop — every field present, the record on the pipe, the
  baseline subtracted (a near-empty program reads near zero), exit codes and signals in the record;
  a program that writes a fake record to its temp dir, kills its parent, forks, or plants a FIFO —
  each denied or ignored, the true record still read.
- **The profile**: the program cannot fork, signal another process, exec a third binary, or write
  outside its temp dir (each paired with the same act succeeding unsandboxed).
- **Paths**: a program that prints `argv[0]` is comparable; the same binary at both sides' paths
  has equal footprints.
- **Workloads file**: strict reading; each refused input by name (outside the target, under the
  ledger through a link, a nested `.git`, a FIFO, over 64 MiB, a name starting with `-`); the
  digest changes with the input's bytes and name.
- **Results file**: rows merged by workload; strict reading; a file whose numbers say slower reads
  "slower" whatever words it might hold; each out-of-date reason; stale facts refused.
- **Evidence**: a unit with a stale verdict, a half-done promotion, a crate edited after verify —
  each "not measured: verify this unit first".
- **End to end on zopfli**: a generated 1 MB input; the program as it stands and u001 measured; a
  workload on the oracle's 30 KB sample reads too-short with the advice; a C program that prints
  the time reads c-unstable; a unit made to print differently reads behaves-differently with both
  outputs kept; an interrupted perf leaves no temp folder and keeps earlier rows.
- **The cockpit**: the Speed View and header lines, out-of-date words, the act argv, the
  correctness alert.
- **Mutation checks** of §3.4's order, §3.5's outcomes and §3.6's rule.

## 5. Order of work

Each step committed green: (a) the launcher and its profile handling, (b) the workloads and results
files in `harness-core`, (c) the shared build refactors (`whole_cc_into`, objects once, the
`replaces` helper), (d) the rows and the words in `harness-oracle`, (e) the CLI, (f) the cockpit,
(g) SCHEMAS (the two files, the writer table, the `perf-row` event), the tutorial and the testing
guide's Part 11 (liblzg: make an input, time the C by hand, write the workloads file, measure, read
the words; a "you should see" that shows the *shape* of the words, since the numbers vary). Then the
code review, fix passes each checked, mutation checks, the handoff.

## 6. Residuals and costs (named)

- **Linux runs unsandboxed** (`--allow-unsandboxed`): a hostile program can fork, signal, rewrite
  the binaries; counts are user-mode only and include every process it starts.
- **Multi-process programs** (macOS): rows end as starts-processes — their children's work is not
  counted.
- **A workload that never runs the unit's code** reads "about as fast" — nothing tells that the
  unit was not reached. Revisit: the features probe could say which units a workload reaches.
- **Before Accept**: a green attempt is not measured; revisit with results keyed by crate digest.
- **The driver row** is not measured; revisit if the person asks for a per-unit microbenchmark.
- **The baseline is verify's build** (-O2, `-ffp-contract=off`, no LTO, Rust's start-up linked
  in); revisit with a `[perf] cflags` if the person's own build differs much.
- **One computer**: results compare only where they were made.
- **Cost**, per `harness perf`: one compile of the C, one crate build per unit, one link per side;
  each new binary's first exec costs 4–8 s here (the launcher, every side); then per row 2 + 1 + 1
  + 2n runs (n = 7: 18 runs) — about 9 s for a 0.26 s zopfli row, 15–25 s per unit before any
  timing. The confirm dialog shows the estimate from the last measured C times.

## 7. What changed from the draft

The words (a disjoint-ranges rule that weakened as runs were added → Hodges–Lehmann with an exact
interval); time from performance-core cycles; memory and instructions with their own words and
floors; one floor on a quiet metric; the program as it stands (it links); units chosen by evidence;
the same path, name and `argv[0]` on both sides; the C compared with itself before blaming the
Rust; behaves-differently as a correctness alert with both outputs kept; the launcher outside the
sandbox, the program inside the scenario profile, the record on a pipe; results without words,
merged by workload, inputs per row, computer and compilers apart; the shared build refactors; the
cockpit's Speed group, View and Edit flow; progress, summary and the confirm dialog; the cost with
builds and first execs; the driver row dropped.

## 8. Not decided here (for the person)

None: every choice above follows the review's confirmed findings or the project's rules.

## 9. Review record

The draft (0b64bba) — 4 lenses, 55 findings, each checked by two verifiers (workflow
`wf_fe7fb255-d44`; scratchpad `perf/design-review-lenses.json`). Dispositions, by lens:
- **Measurement (12)**: the interval rule (§3.6); a margin (§3.6); P-core-normalised cycles (§3.3,
  §3.6); the same path and name (§3.2); kernel instructions named per platform, context switches
  recorded (§3.3, §3.7); one floor on instructions, short runs (§3.4, §3.6); "as verify builds
  them" said, start-up in the residuals (§3.2, §6); the program as it stands (§1, §3.2); memory's
  own words, minimum of n, page floor (§3.6); Linux `inherit = 1` and hybrid PMUs (§3.3); the
  driver row dropped (§3.2); "faster" defined, the observed interval shown (§3.6).
- **Security (15)**: no signals out and no fork for the program — the scenario profile, the
  launcher outside (§3.3); the record on a pipe (§3.3); binaries built before target code and
  hash-checked (§3.2); the input read once (§3.1); no stored words (§3.7); `safe_ledger_dir`
  (§3.7); launcher failures in the record, not exit codes (§3.3); Linux named (§3.10, §6).
- **Integration (13)**: units by evidence (§3.2); the `replaces` check shared (§3.2); the build
  refactors named (§3.2); C compared with itself first (§3.4); one path and `argv[0]` (§3.2);
  outcomes and merged rows (§3.5, §3.7); currency inputs named, stale facts refused (§3.7); what
  the cockpit can judge (§3.7); temp folders registered and 0700 (§3.3); the cost restated (§6);
  timed runs' streams to /dev/null (§3.3, §3.4); SCHEMAS rows and the `perf-row` event (§3.8, §5).
- **The person (13)**: honest "add runs" (§3.6); one sentence about time, details apart (§3.6);
  behaves-differently as an alert with both outputs (§3.4, §3.9); the Speed group and View, the
  baseline line, history through committed files (§3.9); the Edit flow and the starter (§3.1);
  too-short with numbers and advice (§3.4); merged rows (§3.7); progress and the dialog (§3.8,
  §3.9); computer and compilers apart (§3.7); speed before Accept and next steps for slower rows —
  a revisit item (§6); the guide's Part 11 (§5); the small gaps (§3.6, §3.8).
