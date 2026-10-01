# C-vs-Rust performance baselines (design)

Status: **draft — for the adversarial design review** (2026-09-30). The §15 spike and its
premise run: DECISIONS.md "2026-09-30 — C-vs-Rust performance baselines: §15 spike" (notes in that
session's scratchpad `spike-perf/`). The person's wish (DECISIONS M4 roadmap notes): "baseline the
original C, then compare migrated code side by side as the transition proceeds".

## 1. What it is, in plain words

`harness perf` runs the C program and, for each migrated unit, the same program with that unit's
Rust swapped in, on workloads the person names — the inputs and options their program is really
used with. Each side runs several times, interleaved. For each unit and workload it says, in
words, whether the Rust is slower, faster, within noise of the C, or that more runs are needed
to tell; and it keeps the numbers (instructions, cycles, time, peak memory) in a file per unit.

It is information, never a gate: `verify`, `migrate`, `promote` and `bench check` never run it and
never read it. No model is involved (Tier 0).

Words used below: a **side** is one of the two programs (the **C program**, all C; the **mixed
program**, the C with one unit's Rust swapped in); a **workload** is a command line plus, at most,
one input file; a **run** is one execution of one side on one workload.

## 2. Premise (spike, run end to end)

On zopfli's u001-katajainen, the two programs `verify` builds, a 200 KB input, 1 warm-up and 5
interleaved runs a side, under load from other agents:

| | C program | mixed program | mixed / C |
|---|---|---|---|
| instructions | 3.94e9 (range 0.022 %) | | +3.24 % |
| cycles | 1.007e9 (range 1.7 %) | | +0.79 % |
| wall | 0.26 s (range 3.8 %) | | +3.8 % |
| peak footprint | 12.39 MB | 12.39 MB | 0 |

Two corrections the premise forced: (1) the oracle's samples (30 KB, ~1 ms runs) are useless for
timing — fixed costs dominate (+85 % instructions, cycles range 46 %); a workload must run for
≥ ~50 ms or ≥ ~1e8 instructions; (2) instructions and cycles disagree (the Rust executes 3 % more
instructions at a better IPC): instructions are the stable signal, cycles the time-like one —
both are reported, and the words never rest on instructions alone.

A harness-owned C launcher (`perfrun OUT PROGRAM ARGS…`: fork, exec, `waitid(WNOWAIT)`,
`proc_pid_rusage(RUSAGE_INFO_V4)`, `wait4`) run under `sandbox-exec` with a profile that lets it
exec exactly itself and the program read every counter (instructions 3.970e9, cycles, lifetime
maximum footprint 14.25 MB, user/system time, maximum RSS) with no root and no `unsafe` Rust.

## 3. The design

### 3.1 Workloads: `migration/perf/workloads.toml` (`ruharness-perf-workloads`, v1)

```toml
schema_version = 1

[[workload]]
id = "big-text"            # closed alphabet, as features ids: [a-z0-9-], ≤ 32
args = ["-c", "{input}"]   # ≤ 8 strings, each ≤ 256 bytes; `{input}` is the input's file name
input = "bench/big.txt"    # optional: a regular file inside the target, ≤ 64 MiB
runs = 7                   # optional: runs a side, 3 to 31 (default 5)
```

- The person writes it (`harness perf init` writes a commented starter with one workload built
  from `[oracle.whole_program] args`, input left for them to name). Read strictly: unknown keys
  are refused with the key named (it is the person's file, not a format other tools write).
- `input` is resolved against the target root, canonicalized, and must stay inside it, outside
  `migration/` and `.git/`, be a regular file and at most 64 MiB; it is copied into each run's
  working folder (the run's temp dir), as `run_scenario` places a scenario's input, with the
  same file name and the modification time set to the epoch. The workload's digest is blake3
  over its id, args, `runs` and the input's bytes.
- Why not the features scenarios: their inputs are the oracle's samples only (too small to time,
  §2), and a scenario describes *what* the program does, a workload *how much* — a scenario's
  `input` grammar would have to grow a file form that every `verify` then reads. Revisit: if the
  person wants one list, a scenario could gain `perf = true` with a file input.

### 3.2 What is measured

For each unit with a **promoted** crate (status `verified` or `merged`; never an attempt's
candidate) and each workload:
- the **C program**: every top-level `.c` of `source_dir`, linked with `extra_link_args` —
  `build_whole_c`'s inputs;
- the **mixed program**: the same less the unit's `replaces`, plus the unit's staticlib —
  `build_whole_mixed`'s inputs, the crate built as `verify` builds it (`build_crate_staticlib`:
  `cargo build --release --offline`, the crate's own profile; no LTO, no cross-language LTO).

Both are **rebuilt** by `harness perf` into `migration/build/.perf/<unit>/` with the same compile
flags as `verify` (`-O2 -ffp-contract=off -w`): the binaries `verify` leaves are not
content-addressed and `migrate` writes candidates' over them (survey §1). The C program is built
once per `harness perf` and shared by every unit.

Also, per unit, without a workload: the unit's **own driver** (`drv_c` against `drv_rs`, as
`verify` builds them) — the closest look at the unit's functions alone. A driver run is usually
short; its row is flagged "too short to time" below the floor of §3.5 and gets no words.

Not measured: the program with *all* promoted units' Rust at once — two Rust staticlibs in one
link each carry their own copy of Rust's runtime and do not link. Named in §6.

### 3.3 The launcher (`perfrun`, harness-owned C)

`crates/harness-oracle/src/perf/perfrun.c`, embedded with `include_str!` and compiled by
`harness perf` with `cc -O2 -w` and no target include folder (as the probe runtime is), into the
perf build folder. `perfrun OUT PROGRAM ARGS…`:
1. records a monotonic start time, `fork`s; the child `execv`s `PROGRAM` (absolute) with `ARGS`
   (argv[0] the program's path), its environment the launcher's (the run's `PATH` and `TMPDIR`);
2. `waitid(P_PID, child, WEXITED | WNOWAIT)` — the child is dead but not reaped;
3. **macOS**: `proc_pid_rusage(child, RUSAGE_INFO_V4)`: `ri_instructions`, `ri_cycles`,
   `ri_lifetime_max_phys_footprint` (`ri_phys_footprint` is 0 after exit);
   **Linux**: before the exec, the parent opened `perf_event_open` counters on the child for
   `instructions:u` and `cycles:u` with `enable_on_exec` (the child waits on a pipe until they
   exist); read now. No counter (`perf_event_paranoid` ≥ 3, no PMU — GitHub-hosted runners) is
   recorded as such, never as zero;
4. `wait4(child)`: user and system time, `ru_maxrss` (bytes on macOS, KiB on Linux — converted);
   the monotonic end time gives the wall time (the harness's own poll is 50 ms coarse);
5. writes `OUT` (one `key value` line each, ASCII), then exits with the child's exit code, or
   re-raises the child's signal.

The program's own children are not counted (rusage is per process): a workload whose program
forks is measured for its first process only, and says so (§6).

### 3.4 Running a workload

The runs use a new sandbox profile, the **perf run profile**: the run profile (reads denied under
the home folder and the target except the two programs and the run's temp dir; writes only to the
temp dir; no network) with `process-exec` allowed for exactly two literals — `perfrun` and the
side's program — and `process-fork` allowed (the launcher forks; the program may too). Each run
gets a fresh temp dir (`RunTmp`), the workload's input copied in, `TMPDIR` and `PATH` as a
scenario run has; the counters come back through the temp dir (`Extras::collect`, a 4 KiB cap).
The harness's timeout (`[oracle] timeout_secs`) bounds each run.

The order, per unit and workload:
1. **Same output first.** One run of each side; their exit and both streams (rewritten as a
   scenario's, `$TMPDIR`) must be the same. If not: the row says "not comparable: the mixed program
   behaves differently on this workload" and no timing runs follow — a correctness question for
   `verify`, not a speed one.
2. **Warm-up**: one run a side, discarded (the first exec of a just-linked binary can carry the
   system's first-run check — seconds on macOS).
3. **n runs a side, interleaved** C, mixed, C, mixed, … (n = `runs`, default 5): interleaving
   spreads the machine's drift over both sides. A run that fails (a non-zero exit the first run
   did not have, a timeout, no counters) ends the workload's row with that reason.

### 3.5 The words (std only)

Per metric (instructions, cycles, wall, peak footprint), per side: the median, the minimum and the
maximum of the n runs, the **spread** ((max − min) / median), and the **ratio** of the medians
(mixed / C). The words for a metric:
- **too short to time** — the C side's median is under 50 ms wall or 1e8 instructions (§2);
- **slower by X %** / **faster by X %** — the two sides' ranges do not overlap (X the medians'
  difference, to one decimal — however small: instructions' spread is often 0.02 %);
- **within noise (±Y %)** — the ranges overlap and both spreads are at most 3 %; Y is the larger
  spread;
- **inconclusive — add runs** — anything else (n < 4 included).

The **headline** for a row is the cycles' words, then the instructions' when they differ in kind
("cycles within noise (±0.6 %); instructions slower by 3.2 %"); the peak footprint's words when
not "within noise". Wall time is recorded and shown, never the headline (50–100× noisier than
instructions, §2). Why disjoint ranges and not a test: with 5 runs a side, disjoint ranges are
exactly a two-sided Mann–Whitney p of 0.008, the rule reads in one sentence, and "inconclusive"
says what to do.

### 3.6 Results: `migration/perf/<unit>.json` (`ruharness-perf`, v1)

```json
{
  "schema": "ruharness-perf", "schema_version": 1,
  "inputs": { "program": "blake3:…", "crate": "blake3:…", "facts": "blake3:…",
              "machine": "macos-aarch64 · Apple M3 · Apple clang 21.0.0 · rustc 1.90.0",
              "launcher": "blake3:…" },
  "rows": [
    { "workload": "big-text", "digest": "blake3:…", "runs": 5,
      "c":     { "instructions": [..5..], "cycles": [..], "wall_us": [..], "footprint": [..],
                 "user_us": [..], "system_us": [..] },
      "mixed": { … },
      "words": { "instructions": "slower by 3.2 %", "cycles": "within noise (±0.6 %)",
                 "wall": "within noise (±3.1 %)", "footprint": "within noise (±0.0 %)" },
      "headline": "cycles within noise (±0.6 %); instructions slower by 3.2 %" },
    { "workload": "driver", … }
  ]
}
```
- Written atomically (`write_atomic`) after every row (an interrupted `harness perf` keeps the
  rows it finished). One file per unit, replaced by each measurement of that unit.
- It records measurements, not timestamps (SCHEMAS' rule for committed files holds); it is
  machine-bound evidence, not ledger truth: a fresh clone resumes without it. The person may
  commit it to keep a history in git.
- **Current** iff its inputs equal today's: the C program's digest (`program_digest_now`), the
  unit's crate digest, the facts digest, the machine string and the launcher's digest; else "out
  of date" with the reasons in words ("the unit's Rust changed", "measured on another machine").
  A row whose workload digest differs from today's workload is out of date alone.
- Read strictly (bounded sizes, every number an integer, words from the closed set above) — the
  cockpit shows it.

### 3.7 The CLI

- `harness perf [--unit ID]… [--workload ID]… [--allow-unsandboxed]`: takes the writer lock;
  without `--unit`, every unit with a promoted crate; without `--workload`, every workload plus
  each unit's driver row. Prints, per row, `perf: u001-katajainen on big-text — cycles within
  noise (±0.6 %); instructions slower by 3.2 % (5 runs a side)`; `--json` events `perf_row`
  (unit, workload, headline, the four words — no numbers in events).
- `harness perf init`: the starter workloads file, refused if one exists.
- `harness perf` with no workloads file: says how to write one; with no promoted unit: says so.
- Exit codes: 0 when every row was measured (whatever the words), 1 on an error or refusal; a
  row "not comparable" is a measured result (exit 0) — perf never reds a run.

### 3.8 The cockpit

- A unit's header gains a line under the features line: `Speed: cycles within noise of the C
  (±0.6 %), instructions +3.2 % — big-text, 5 runs` (the first workload's headline; "(out of date:
  the unit's Rust changed)" when not current; nothing when never measured).
- The unit's menu gains **Measure speed** (`harness --json perf --unit <id>`); the project menu
  **Measure speed of every unit**. A view **Speed** lists every row of the unit (workload, the
  four words, medians a side). No Next-step rule: speed is never the next step.
- Display-filtered as everything else (workload ids are a closed alphabet; words a closed set).

### 3.9 Security

- The workload's input is target-controlled but bounded and confined (§3.1); the programs run
  under the perf run profile; the launcher is harness-owned, compiled from embedded source into
  the harness's build folder and run by path; its `OUT` file is read with the same lstat /
  same-inode / cap rules as every collected file.
- No new dependency: the launcher is C built with `cc`; the statistics are `std`. `unsafe` Rust
  stays forbidden in every crate (the spike's "second unsafe exception" is not needed).

## 4. Tests and checks

- The words (§3.5): a table of synthetic samples — ranges disjoint by a hair and touching,
  overlapping at spreads of 3 % and 3.1 %, n = 3 and 4, a too-short C median at 49 and 50 ms —
  each giving its words; mutating any bound must fail it.
- The launcher: built and run on a busy loop (counters present and non-zero; exit code and
  signal passed through; `OUT` well formed), on a program that execs nothing else.
- The perf run profile: exactly two exec literals and fork allowed; a program that tries to exec
  a third is denied; nothing written outside the temp dir.
- The workloads file: strict reading (unknown key, a path out of the target, a symlink out, a
  FIFO, a file over 64 MiB — each refused by name); the digest changes with the input's bytes.
- The results file: written and read back strictly; out-of-date reasons for each input.
- End to end on zopfli: a generated 1 MB input; u001's row has every metric measured; the driver
  row says "too short to time"; a mixed program made to print differently is "not comparable".
- The cockpit: the Speed line, its out-of-date words, the act's argv.
- Mutation checks of §3.4's order (same output first, warm-up discarded, interleaving) and §3.5's
  bounds.

## 5. Order of work

Steps, each committed green: (a) the launcher and its profile; (b) workloads file and results file
in `harness-core`; (c) the run and the words in `harness-oracle`; (d) the CLI; (e) the cockpit;
(f) SCHEMAS, the tutorial, the testing guide's perf step. Then the code review, fix passes each
checked, mutation checks, the handoff.

## 6. Residuals and costs (named)

- **One unit at a time**: the program with every promoted unit's Rust at once is not measured (two
  Rust staticlibs do not link together). Revisit when the harness builds one crate for all units.
- **A workload that does not run the unit's code** reads "within noise" — nothing tells that the
  unit was never reached. Revisit: the features probe could tell (a workload as a probed scenario).
- **Multi-process programs**: only the first process is counted.
- **Machines differ**: results compare only on the machine that made them; never across.
- **Linux without a PMU** (shared CI runners): time and memory only.
- **E-cores** (Apple silicon): a run that migrates to an efficiency core is slower in cycles; the
  spread shows it, and the words say "inconclusive" rather than guess.
- **Cost**: per unit and workload, 2 + 2 × (1 + n) runs (n = 5: 14 runs) plus the build of the
  mixed program; the C program builds once.
