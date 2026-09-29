# Features: what a user does with the program, mapped to the code and checked — design

Status: DESIGN, revised four times (2026-09-29): the adversarial review (four lenses, 84
findings, §R), the check of that revision (three checkers, 58, §R2), a scoped check of the
second revision (two checkers, 37, §R3), and a scoped check of the third (one checker, 11 —
no high, five medium, each resolved locally as proposed, §R4). Step 1 (core) is built; §R3
and §R4 say what they changed there.

This implements the user's direction recorded 2026-09-23 ("feature workflows": user-facing
behaviours mapped to the code that implements them, so a migration preserves what a user
perceives, not only per-function correctness), in the scope the user chose on 2026-09-29
(DECISIONS.md "Feature-workflow view: §15 spike"): **scenarios + verify**.

Sources:
- the agent briefing (§2, §12, §13.2, §16);
- the §15 spike (DECISIONS.md, 2026-09-29): the premise run end to end, the landscape, and
  the probe prototype;
- docs/ORACLE-HARDENING.md §B.3–§B.5 (a confined run that reads back one file: `Extras`);
- docs/M4-DESIGN.md §R1/§R8 (run confinement; the whole-program check);
- docs/COCKPIT-WRAPPER-DESIGN.md §2–§6 (the tree, the View, the menu, confirming) — every
  rule there holds unless this document says otherwise;
- docs/CHAT-PANE-DESIGN.md §1 (what the chat can and cannot see);
- docs/SCHEMAS.md (the ledger's contracts, the events stream, the trust boundaries).

## 0. What it is, and is not

**A feature** is something a person does with the program and can see the result of —
"compress a file to zlib", "print the help", "say that a file does not exist". The person
names it. It is made of one or more **scenarios**: one run of the whole program with fixed
arguments and at most one of the harness's sample files as input. What a person perceives
of a run is its **exit status, stdout and stderr** — that is what is compared.

The harness does three things with features, all deterministic (Tier 0, no model):
1. **Check** — in `verify` (and so in every judged turn of `migrate`, every promotion and
   every recorded hand edit): every scenario runs on the all-C program and on the mixed
   program (all C minus the unit's `replaces`, plus the unit's Rust); one check per scenario,
   `feature:<feature>/<scenario>`, passes only when both exited normally with the same code
   and byte-identical streams.
2. **Map** — `harness features map`: every scenario runs on a scratch copy of the original C
   in which each function notes, the first time it runs, that it ran (the **probe**); the
   result — which of the scanner's functions each scenario ran — is written to
   `migration/features/map.json`. Functions map to files and to the plan's units at read
   time.
3. **Show** — the cockpit: a **Features** group in the tree; each feature's View shows its
   scenarios and what they did, the units its functions live in with their states, and its
   checks on each unit's verdict; each unit's View says which features run its functions —
   or that none does, when the map can say so.

**Why both.** The spike found that zopfli's whole-program check runs `zopfli -c <sample>`
only — the gzip path — so a unit on the zlib path passes it without the check ever running
its code. Scenario checks make the person's behaviours part of every verdict; the map says
which of those checks actually exercise a unit, and which units no behaviour reaches.

**Nothing about features ever blocks work — and nothing the Rust does can make a feature
check disappear.** A features file with an error, a scenario the C program cannot run the same
way twice, a unit whose files are not part of the program, a program that does not build or
has no single `main()`: in each case `verify` still judges the unit on every other check,
records in the verdict which feature checks did not run and why, and says so while it runs;
the cockpit shows it. Every one of those reasons is decided by the C side, the file or the
plan — never by the candidate: whatever the Rust does on the mixed side (crash, hang, flood
output, fail to link) is a failed check.

**What stays true:**
- The oracle is the definition of done; the map never gates anything. A green verdict still
  means every check it records passed; features add checks, never waive one.
- The ledger is the truth; the cockpit never writes it; every write is a spawned
  `harness --json …` command, confirmed first. Editing the features file happens in a
  private copy; saving it is a confirmed `harness features save` (§7.2).
- Target-owned files are hostile input (SCHEMAS.md "Trust boundaries"). `features.toml`,
  `map.json` and the verdicts' feature fields are target-owned: nothing in them reaches a
  model prompt, an event or harness-mcp except ids from a closed alphabet and reasons from a
  closed set (§2.3); everything shown passes the display filter; every run they define is
  confined like the whole-program check's, in an empty directory of its own (§4).
- A command that builds and runs target code refuses under `sandbox: none` unless
  `--allow-unsandboxed` is given (the rule every such command follows).
- No new crate. No new tool on the allowlist: the probe needs `cc` only.
- **Additive**: a target without `features.toml` behaves byte-for-byte as today — same
  checks, same verdict bytes, same attempt records, same events. The TRACTOR bench is
  unaffected (§10).

**Not in this design** (each has a "revisit when", §13):
- Features of a library without a `main` (all TRACTOR cases): a scenario runs a program.
- Input files of the person's own, stdin, environment variables, and a file the program
  writes as the thing compared: v1 compares exit status and the two streams, with the three
  samples as inputs.
- A cumulative mixed build (every verified unit in Rust at once): each verdict swaps in its
  own unit only, as today — and the cockpit says so wherever a feature is reported as
  holding on migrated units (§8.2).
- Line-level maps; features proposed from usage text or by a model; a "Re-check every unit"
  act; a time budget for scenario runs (each run has the oracle's timeout; §13).

## 1. Where it lives

```
migration/features/
  features.toml        the person's features and scenarios (§2)
  map.json             which functions each scenario ran (§5)
migration/build/.features/   scratch of `features map` (gitignored; a leading dot is never a unit id)
```

Both files are committed. They are under `migration/`, so a bench case may hold them without
breaking its corpus lock. `map.json` is derived and deterministic for one platform and one
set of inputs (§5.2); it is committed so the map shows on a cold start, like
`observer/findings.jsonl`.

Writer table additions (SCHEMAS.md):

| File | Writer |
|---|---|
| `migration/features/features.toml` | a person; `harness features init` (a starter, never over an existing file); `harness features save` (the cockpit's Edit) |
| `migration/features/map.json` | `harness features map` |
| `migration/build/.features/**` (gitignored) | `features map` (scratch) |

(`verify`'s feature runs use the unit's own build dir, `migration/build/<unit>/`, as the
whole-program check does.)

## 2. `features.toml` (`ruharness-features`, v1)

### 2.1 Shape

Flat tables — a scenario names its feature:

```toml
schema_version = 1

[[feature]]
id = "gzip"
name = "Compress to gzip"

[[feature]]
id = "zlib"
name = "Compress to zlib"

[[feature]]
id = "help"
name = "Show the help"

[[scenario]]
feature = "gzip"
id = "text"
args = ["-c", "{input}"]
input = "sample:text"

[[scenario]]
feature = "zlib"
id = "text"
args = ["--zlib", "-c", "{input}"]
input = "sample:text"

[[scenario]]
feature = "help"
id = "flag"
args = ["-h"]
```

- `schema_version`: required, `1`.
- `id` (feature and scenario): `^[a-z0-9][a-z0-9-]{0,23}$`; unique among features, and
  unique among one feature's scenarios. (`feature:` + 24 + `/` + 24 = 57 bytes: within the
  64 at which a repair prompt cuts check names.)
- `name`: the person's words, 1–60 characters, no control characters. Shown only in the
  cockpit (display-filtered). Help advises short names (about 14 characters show in full in
  the tree at 80 columns).
- `feature` (a scenario's): the id of a `[[feature]]` in the file.
- `args`: 0–8 entries (absent = none), each one of:
  - `{input}` — the input's file name (§4.1); exactly once when `input` is set, never
    otherwise;
  - a flag: `^-{1,2}[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (`-c`, `--i5`, `--level=3`);
  - a word: `^[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (a sub-command or a value: `compress`, `9`,
    `nosuchfile`);
  - 1–64 bytes, never containing `/` or `..`.
  An argument cannot be an absolute path or reach a parent directory. A word or an `=` value
  can still name a file **relative to the run's directory** — the run's own fresh temp dir
  (§4.1), holding only the input: a name there either is the input or does not exist, and a
  file the program writes there is discarded. (That makes "a file that does not exist" an
  easy scenario to write.) The safety of the grammar rests on that directory and on the
  sandbox, not on the syntax.
- `input`: optional; `sample:text` (about 30 000 bytes of English text), `sample:rand` (16 KiB
  of pseudo-random bytes) or `sample:empty` — the whole-program check's deterministic
  samples, the same bytes. The cockpit describes them in words, never by the token alone.
- Limits: ≤ 16 features; ≤ 8 scenarios per feature and ≤ 16 in all; the file ≤ 64 KiB; every
  feature has at least one scenario. A file with no features at all is valid (a starter).
- **Strict**: an unknown key, a wrong type, a duplicate or unknown id, a limit passed is
  refused with the key's path and the rule, e.g. `features.toml: scenario "text" of feature
  "zlib": args[2] "a/b" is not allowed — an argument cannot contain "/"`; a TOML syntax error
  names its line and column (the message flattened to one line). The file is typed by hand:
  a typo must be caught, not passed over (the briefing's §13.2). Consequence, stated in
  SCHEMAS.md: **every new key bumps `schema_version`** (an exception to the ledger's
  pass-over-unknown-fields rule).

### 2.2 The snapshot: `harness_core::features`

One loader, used by the oracle, the CLI and the cockpit, returning a value, never an error
for a bad file:

`FeatureSnapshot::load(ctx: &TargetContext) -> FeatureSnapshot` —
- `None`: `migration/features/features.toml` does not exist;
- `Invalid(message)`: the file (or `migration/features/`) is a symlink, not a regular file,
  larger than 64 KiB, not UTF-8, does not validate, or has a newer `schema_version` (the
  message then says a newer harness wrote it);
- `Valid { features, digest }`: the validated features and the `features` digest (§2.4).

It reads through `ledger::read_regular` (no symlink, the opened handle checked, bounded). An
I/O error other than "not found" is `Invalid` too: a read path never fails because of the
features file.

**Loaded once per judged run** and passed to whatever judges. The oracle's required entry
point takes it — `OracleStrategy::verify_with(&ctx, &unit, &FeatureSnapshot)` — and the
provided `verify(&ctx, &unit)` loads one and calls it (so a caller cannot silently skip
features by forgetting to pass one, and nothing can substitute today's file for a replay's
`None`). The call sites: `harness verify` (main.rs), `promote_attempt` (promote.rs — it takes
the snapshot of the command that promotes: `harness promote`, or migrate's automatic
promotion with the attempt's own snapshot), the migrate judge (migrate.rs — every turn of one
run under the snapshot of that run's `RunCtx`; `override` reaches it through
`record_human_attempt`), and the bench (bench.rs — its cases have no file).

### 2.3 Why ids have a closed alphabet

Check names reach model prompts: a red verdict's failed checks are quoted to the repair turn
(`oracle_evidence` quotes `check.name`), and harness-mcp shows verdicts to the chat. The
features file is target-owned; a hostile target could write an instruction into a `name` or
into its arguments. So a check name is built from ids only (`feature:zlib/text`), an alphabet
that cannot spell a sentence or a prompt section header, and a check's detail is the
harness's own words with numbers (§6.2) — never the arguments, never the program's output.
The skip list (§3) holds ids and reasons from a closed set, and every reader parses it
strictly (an entry that does not parse is dropped and counted). The loader's error messages
quote the file's text; they appear in the cockpit (display-filtered) and in `features save`'s
refusal, never in a verdict or a prompt.

### 2.4 The digests

Recorded in every verdict and attempt that ran the feature step while a features file exists,
and in the map (§5.2):
- **`features`** — `blake3:` over a canonical rendering of what the scenarios run: for each
  feature by id, each scenario by id: its args and its input's **bytes**; plus the program's
  file name as it runs (§4.1) and `[oracle] timeout_secs` — both part of what is compared.
  Names are not in it: rewording a feature changes nothing. Features and scenarios are sorted
  by id in the digest (their order in the file is presentation) and run in file order. The
  samples' bytes come from the in-memory generator (`Sample::bytes`), so computing the digest
  reads no file but `features.toml`.
- **`program`** — `blake3:` over what the whole program is built from: every top-level `.c` of
  `source_dir` (which the whole-program build compiles, scanned or not — through its
  canonical path when it is a symlink inside the target, as the build resolves it), the
  include closure the facts record for those files, and every `.h` under `source_dir` and the
  `include_dirs` (the build's include path: a header reached with `<…>` is in no closure),
  each as `(path, current hash or "missing")` (a file over 64 MiB reads as missing), plus `[target] source_dir`, `include_dirs` and `[oracle] extra_link_args`. One
  core function computes it from such pairs, so the oracle and every reader agree on a
  missing file. Known gap, named in SCHEMAS.md: a non-C file a header includes (`.inc`,
  `.def`) is not in it. **When the facts do not describe the program** — a file they record
  whose bytes differ or which is gone, or a file under `source_dir` (followed as the scan
  follows links) they have no record of: exactly what a scan would change, so a scan always
  clears it — the digest is the sentinel `facts-stale`, which no comparison reads as the
  same program, not even itself: coverage stays `program` ("the C changed since the scan: Scan
  the project, then Re-check") until a scan (review O2). The oracle takes it **before** it
  builds anything, the map before its builds (review O6, M10).

Both are computed **once per read** — once per `verify`, once per `Snapshot::load`, once per
`state status` — never per unit; nothing is hashed without a features file. A file several
paths reach is read once, and the read preflight counts the program's files against its hash
budget (§8.7; review T1).

## 3. Which verdicts cover the features: a marker, not staleness

A verdict made before a scenario was added (or before another unit's C changed, or while the
file had an error) did not run the scenarios as they are now. That is **not** the verdict
being stale: every check it recorded is still true of the unit's own inputs. Treating it as
stale would flip every verified unit to "needs attention", count none as migrated, and report
a CONTRADICTION for each (status.rs's rule) on every edit of the file.

So:
- `VerdictInputs` gains three fields, all `#[serde(default, skip_serializing_if = …)]`
  (empty = absent, so a verdict without features keeps today's bytes), filled by `run_verify`
  **after the feature step** (never by `compute_inputs`, which stays pure) — so a verdict that
  stopped at an earlier gate records none:
  - `features: String` — the digest the verdict ran under, or `invalid` when the file could
    not be used (no feature check ran);
  - `program: String` — the program digest;
  - `features_skipped: Vec<String>` — scenario checks that did not run, each
    `<feature>/<scenario>: <reason>`, the reason from the closed set of §6.1:
    `c-side-unstable`, `c-side-crashed`, `c-side-timed-out`, `c-side-overflow`,
    `c-side-exec-failed`, `c-side-build-failed`, `not-in-program`.
- The `stale` list, `fresh_green`, the contradiction rule, `verified_in_place`, the bench's
  `fresh_green` and fence's `STALE_INPUTS` are **unchanged**.
- `UnitReport` gains `features: Coverage` (`#[serde(skip_serializing_if)]` when `None`):
  - `None` — no features file now (whatever the verdict recorded), or no verdict;
  - `Current` — the verdict's `features` and `program` equal today's and nothing was skipped;
  - `Behind(reasons)` — reasons among `not-yet` (the verdict records no features), `changed`
    (its `features` digest differs), `invalid` (it ran while the file could not be used, or
    the file cannot be used now), `program` (its `program` differs), `skipped` (it lists
    skipped scenarios; the list is in the verdict). `not-yet`, `changed`, `program` and
    `invalid` (while the file is valid now) are **re-checkable** — a Re-check changes them;
    `skipped` is not, unless the file or the program changes. The first three are exclusive
    and come first, then `program`, then `skipped`.
  The `unit` event and harness-mcp's unit report carry it (a closed set of strings, §9).
- The cockpit shows it as a **marker on the verdict line**, not a state: the unit keeps its
  state (✓ migrated stays ✓). Its words, one per reason, in this order when several apply:
  | reason | words on the verdict line |
  |---|---|
  | `not-yet` | your features: not checked on this unit yet — Re-check it (for a red verdict: "…the verdict stopped before them") |
  | `changed` | your features: not checked since you changed them — Re-check it |
  | `invalid` | your features: not checked — features.toml had an error then (Re-check it) or has one now (fix it first) |
  | `program` | your features: checked before other C changed — Re-check it |
  | `skipped` | your features: n scenarios could not run — see Features (the reasons in §6.1's words) |
- `AttemptRecord` gains `features` and `program` (both skipped when empty). Nothing refuses a
  resume: a resume already re-judges every turn under the current oracle, with the snapshot
  of its run. A **replay** of a finished attempt judges with the features the attempt was
  recorded under when it can: no digest, or `invalid`, recorded → verify with
  `FeatureSnapshot::None` (which reproduces it exactly); the recorded digest equals today's
  → with today's snapshot; otherwise with today's, and a divergence is an `Error::Diverged`
  whose first line explains it ("the features changed since it was recorded" — or "other C
  changed since", from `program`), never superseded inputs — so `--retry` still records a new
  sample and a promoted attempt of a target that gains a features file stays verifiable. The
  replay chooses its snapshot itself; the live run that may follow it in the same command
  (`trace_backed_sample`) uses the command's.

## 4. Running a scenario

### 4.1 The confined scenario run

`Confinement::run_scenario`, used by `verify` and by `features map`:
1. **One path for every run of a scenario**: before each run the side's binary is copied to
   `<build>/f/<name>` (replacing the previous one; the runs are sequential), where `<name>` is
   the target's `[target] name` when it matches `^[A-Za-z0-9][A-Za-z0-9_.-]{0,31}$`, else
   `program`. So argv[0], the program's directory and its basename are the same bytes on the
   C side, the mixed side, and in the map's plain and probed runs.
2. **A fresh temp dir** (as every confined run): the only place the run may write, removed
   after; its name has a fixed width, so every run's has the same length (review O9). The
   program's folder is `run/` inside it (review M8: `$TMPDIR`, where the map's notes go, is
   not the folder a program lists). The sample, when the scenario has one, is written into
   `run/` under its fixed file name (`sample_text.txt`, `sample_rand.bin`, `sample_empty`)
   with a fixed modification time.
3. The child runs with **cwd = `<temp dir>/run`** (it prints as `$TMPDIR/run`), `TMPDIR` = the
   temp dir, argv[0] = `<build>/f/<name>` (absolute), and
   `{input}` = the bare file name; spawned by that absolute path (sandboxed, `sandbox-exec` is
   given it). Both raw streams (the confinement's `run_raw_with`, not `run_with`, whose own
   `$TMPDIR` replacement would be escaped twice) are rewritten one-to-one: every `$` becomes
   `$$`, then the temp
   dir's path becomes `$TMPDIR` and `<build>/f` becomes `$PROGDIR` (neither contains the
   other: the temp dir is a fresh leaf under the system temp dir, `<build>` is under the
   target) — so the details' lengths are the same on every machine, and a candidate that
   prints the literal token cannot match a C side that printed its real path.
4. The run profile is the whole-program check's — exec only the binary, read only the binary
   and the temp dir under the home dir and the target root (reads elsewhere allowed, as
   today), write only the temp dir — plus `(deny signal)` `(allow signal (target self))` and
   `(deny process-fork)`, both applied to both sides alike: a scenario run cannot signal
   another process (`(target others)` alone is not enough: it means "outside the run's process
   group", which the run can change) or leave a child behind; `abort`, `raise` and signals to
   itself still work. A program that forks for its own work (workers, daemonizing) takes its
   error path on both sides: its scenario then compares that error path — the map flags a
   scenario that "compares little", and Help says that programs which fork are checked only up
   to the fork. (Without the sandbox, the candidate's inability to spawn or signal rests on the
   deny scan and the capabilities check alone, as for every candidate run today; SCHEMAS.md
   says so. Nor, without the sandbox, is the build dir out of a candidate's reach: a candidate
   run can write the C binary and the samples there. The C program's bytes are noted when it is
   built and checked before every C run, in the whole-program check and the feature step alike
   — changed, the check and every later one fail with "the C program changed while the check
   ran", never a skip (review O3; the fix check found the first pass noting them only after the
   whole-program check's candidate runs). A sample a candidate rewrites is read by both sides
   alike; C reads elsewhere are not covered; SCHEMAS.md names the residual.)
5. When the leader exits, its **process group is killed** (the `/bin/kill -KILL -- -<pgid>`
   the timeout path uses, called right after `try_wait` reports the exit and before the
   output drain's grace wait) — belt and braces behind the fork denial, and the only guard
   without a sandbox. Scenario runs opt in; every existing run is unchanged. Residual,
   stated: a grandchild that calls `setsid` leaves the group; after the reap a pgid could in
   principle be reused.
6. The outcome is reported as data — `Exited(code)`, `Signaled(n)`, `TimedOut`, `Overflow`
   (more output than the cap), `ExecFailed` — with both streams for the first two. Under the
   sandbox, a run whose stderr starts with `sandbox-exec: ` and whose exit code is 65 (a
   profile error) or 71 (exec failed) is `ExecFailed`, never `Exited`: a candidate imitating it
   fails itself, a C program imitating it only skips itself. A new method beside
   `built_status`, which the bench keeps as it is.

### 4.2 The order of runs and what each side must do

Per scenario and unit: **C, mixed, C** — the two C runs around the mixed one, so a program
whose output drifts with time or state shows it on the C side. (As built: a first C run that
did not exit normally decides the skip at once; the mixed side and the second C run would add
nothing but time.)
- **The C side is usable** when both C runs exited normally (any code), within the timeout,
  without overflowing, with identical results. Otherwise the scenario is **skipped** for this
  verdict with its `c-side-*` reason — a mistake in the scenario (or the C), not evidence
  about any unit; no check is recorded for it, and nothing is fed to a model. Under the sandbox
  the candidate cannot cause it: a scenario run cannot signal or leave a process behind
  (§4.1), cannot write the build dir, and the candidate's code cannot spawn a process (the deny
  scan, the capabilities check). Without the sandbox, see §4.1 step 4.
- **The mixed side must match** the C runs: the same exit code and byte-identical stdout and
  stderr. A signal, a timeout, an overflow or an exec failure on the mixed side never passes —
  it is the candidate's crash, as everywhere else; a slow candidate meets the per-run
  timeout and fails, it never makes a later scenario disappear.

## 5. `harness features map`

### 5.1 The command

```
harness features map [--target DIR] [--allow-unsandboxed] [--json]
```
- Takes the writer lock (holder `features map`). It touches no unit, so there is no
  promotion recovery.
- Refuses, before building anything, with a message that says what to do, when: there is no
  features file, it has no scenarios, or it does not validate (the loader's message); the
  facts are missing, or stale for any file ("scan the project first" — the probe's ids come
  from the facts); a top-level `.c` of `source_dir` has no facts record; `sandbox: none`
  without `--allow-unsandboxed`. Whether the program has one `main()` is the link's to say:
  a failed `plain` build is refused with the compiler's first lines and, when the facts do not
  record exactly one `main` (distinct file and id) among the top-level `.c`, "features need a
  program with one main()".
- Builds, in `migration/build/.features/` (recreated), with the allowlisted `cc` under the
  tool profile, with progress messages ("Building the C program…", "Building a scratch copy
  that notes each function it runs…"):
  - **`plain`** — the top-level `*.c` of `source_dir`, compiled exactly as the whole-program
    check compiles `whole_c`;
  - **`probed`** — the same, from the mirror (§5.3), with the probe runtime (§5.4).
  A build that fails is refused with the compiler's first lines (in the terminal and the
  cockpit's details — never in a verdict).
- For each scenario, in file order: `plain`, `probed`, `plain` — each a scenario run (§4.1).
- Writes `map.json` atomically once every scenario has run. An interrupted run (cancel,
  signal) writes nothing and says so; the old file stays.
- Exit 0 when `map.json` was written — even when some scenario needs a look (a finding
  recorded in the file); 1 on a refusal or an error.

### 5.2 `map.json` (`ruharness-features-map`, v1)

```json
{
  "schema": "ruharness-features-map",
  "schema_version": 1,
  "inputs": { "facts": "blake3:…", "features": "blake3:…", "program": "blake3:…",
              "platform": "macos-aarch64" },
  "unwatched": [["src/zopfli/x.c", "src/zopfli/x.c::odd"]],
  "scenarios": [
    {
      "feature": "zlib", "scenario": "text",
      "end": "exit 0",
      "stdout_bytes": 18234, "stderr_bytes": 0,
      "stderr_head": "",
      "stable": true,
      "probe_agrees": true,
      "noted": "complete",
      "functions": [["src/zopfli/zlib_container.c", "ZopfliZlibCompress"], ["…", "…"]]
    }
  ]
}
```

- `inputs`: `facts` — the hash of the facts file's canonical bytes (`Facts::to_canonical_bytes`;
  a re-scan by a changed scanner changes it); `features`, `program` — the digests (§2.4);
  `platform` — OS and architecture: a map made on another platform may reflect other `#if`
  branches, so it is out of date there (shown, never discarded — §8.4). No timings, no
  toolchain strings: the file is deterministic for its inputs.
- `unwatched`: definitions the probe could not instrument (§5.3).
- Function references are `[file, canonical id]` pairs — the facts' own `file` and `name`
  (for a static function the id already carries its file: `src/x.c::helper`). The pair is
  what makes two functions of one name in two files distinct.
- Per scenario:
  - `end`: how the first plain run ended — `exit N`, `signal N`, `timed out`, `too much
    output`, `could not start`;
  - `stdout_bytes`, `stderr_bytes`: numbers; `stderr_head`: the first line of stderr, at most
    100 bytes, every byte outside printable ASCII replaced by `?` at write time (shown only in
    the cockpit, display-filtered again, never in a verdict, an event or a prompt);
  - `stable`: the two plain runs ended the same way with identical streams;
  - `probe_agrees`: the probed run ended as the first plain run did, with identical streams —
    when false, the map is of a program that behaved differently with the notes in it;
  - `noted`: `complete` (the note file was read) or `unavailable` with `reason` — `none
    written` (the run hit no watched function) or `unreadable` (not whole 4-byte records, an
    id out of range, not a regular file);
  - `functions`: sorted, each once; empty unless `complete`.
- Units are **not** stored: they are derived from today's plan at read time.
- **Current** iff all four inputs equal today's; otherwise **out of date**, with which input
  changed.
- **Hostile input**: the file is committed, so the loader (`features::load_map`) is strict —
  shape, the §2.1 id alphabet, `end` from its closed grammar, sizes (≤ 16 MiB,
  `read_regular`), every pair (in `functions` and `unwatched`) present in today's facts (an
  unknown pair is counted, never shown). A file that does not load is a value the View shows
  ("the map could not be read: …"), never a failed read. A forged file with correct digests
  still reads as current: the View says the map is what the last `features map` recorded; a
  person can map again; it never gates anything.

### 5.3 The probed copy

- **The mirror**: every regular file under the canonical `source_dir` — except `migration/`
  and `.git/` when `source_dir` is the target root — copied to
  `migration/build/.features/mirror/<repo-relative path>` by a new walk mode (all files, not
  only `.c`/`.h`; contained: a symlink inside `source_dir` is copied as the file it names, as
  the build resolves it, one leaving it is left out; `migration/` and `.git/` never entered,
  so they count toward no limit (review M6); at most 20 000 files and 256 MiB — past
  either, the map refuses and says so), and each file the facts record that has a watched
  function replaced by its probed version. The compile uses `-I` the mirror's `source_dir`
  and `include_dirs`, `-fmacro-prefix-map=<mirror>=<canonical root>` (so `__FILE__` reads as
  in the plain build, which compiles canonical paths), `-include <build>/fnprobe.h`,
  `-ffp-contract=off` and `extra_link_args`; every tool child has `SOURCE_DATE_EPOCH=0`, so the
  plain and probed builds (and verify's all-C and mixed ones) print the same `__DATE__`,
  `__TIME__` and `__TIMESTAMP__` (review M5; not recorded in a verdict's evidence — a C unit
  that returns those macros is compared against 1970's). An include that resolves outside
  `source_dir` — read from the program's own `#include` lines and found as the compiler finds
  it (the facts never record one: the scan stays in `source_dir`) — is refused before any
  build, naming the file and the include: the mirror holds `source_dir` only, and the compiler
  would fall through to a system header of the same name, a different program mapped silently
  (review M7, fixed after the fix check). A probed copy that does not build says so in neutral
  words.
- **The insertion** — `harness_scan::probe_source(rel_path, source, index_of)`, pure, built on
  the scanner's own `collect_functions`/`canonical_id` (which, unlike mutate.rs, walk into
  preprocessor branches), with `FnDef` gaining the body's start byte:
  `if (!__ruharness_seen[N]) (__ruharness_probe)(N);` right after the opening `{` of each
  watched function's body, on the same line (the call parenthesised so a function-like macro
  cannot capture it; the inline test keeps a hot function to one byte load). Nothing else
  changes: line numbers, `__LINE__`, `__func__`. `N` is the index of the definition's `(file,
  canonical id)` pair among the facts' distinct pairs (`#if` variants of one function share
  it).
- **Unwatched** (listed in `unwatched`, never counted as "not run"): a definition under an
  ERROR node, or with a parse error in its head (one inside its body — a loop macro the parser
  cannot read — is not in the note's way; review M9); a body whose first byte is not a real
  `{`; a definition with a preprocessor directive between its declarator and its body (a line
  that starts with `#` — a brace inside `#if`, which compiles on one branch only; a `#` in a
  comment is none); a body that opens with a pragma in any spelling (`#pragma`, `# pragma`,
  `_Pragma`, one inside a leading `#if`) or with what the parser cannot read (a macro that may
  expand to one), and a function with `naked` as a word of its head (review M2, fix check
  N1/N4). Residual: `naked` given only on an earlier declaration — the probed build fails and
  the map says so with the compiler's words, never silently. A definition the facts do not
  record gets nothing — the facts do not know it either.

### 5.4 The probe runtime (harness-owned C)

`fnprobe.h` (declarations only: `extern unsigned char __ruharness_seen[];` and the function)
and `fnprobe.c`, embedded with `include_str!` like the boundary check's runtime, with the id
count passed as `-DRUHARNESS_FNPROBE_N=<n>`:
- `__ruharness_seen[n]` (at least one byte); an id is noted the first time it is hit.
- On the first hit the runtime opens `$TMPDIR/ruharness-fnprobe` with
  `O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC|O_NOFOLLOW` and moves the descriptor high —
  `F_DUPFD_CLOEXEC` at `min(900, soft RLIMIT_NOFILE − 16)`, keeping the original descriptor
  when that fails (a launchd-started process may have a soft limit of 256). Each hit writes
  its id as 4 bytes (little endian) — written at the first hit, so a crash, `_exit` or a
  timeout keeps what ran before. Two threads racing the first open may open twice; both
  append whole records. A write that fails (the program closed its inherited descriptors)
  reopens the path noted at the first open and writes once more; the probe saves and restores
  `errno`, so a program never sees it there; a notes file opened but empty is "unavailable"
  (review M3).
- The run reads the file back with `Extras::collect` (a plain name, `lstat`, the same inode,
  capped at `4 × n × 64` bytes).

**What a hostile target can do to its own map**: its C can write that file, redefine
`open`/`write`/`getenv`, change `TMPDIR` before the first note, or close the descriptor. It
can shape its map; it cannot shape a verdict — the map gates nothing (as design B's boundary
map, §B.9). Said in SCHEMAS.md's trust boundaries.

### 5.5 Events

Additive kind (SCHEMAS.md "The events stream"):

| `k` | fields |
|---|---|
| `scenario` | `feature`, `scenario`, `n`, `of`, `end`, `stable`, `probe_agrees`, `noted`, `functions` (the count) — one per scenario of `features map`, after its runs |

Plus `message` lines and the `header`/`result` frame. `verify` and `promote` emit `message`
lines around their feature step (a migrate turn's verdict carries its skips; the cockpit's
marker says them): "Running your 12 feature scenarios…" before; one per skip, in §6.1's words
("Skipped zlib/text: the C program's output differs between runs"); and "Your features file
has an error — no feature scenario ran" for `Invalid`. Its checks arrive as `check` events, as
today.

## 6. Scenario checks in `verify`

### 6.1 When and where

In `CAbiDifferential::verify_with`, **after every existing check** — after the sanitizers and
the opt-in boundary check (so a feature failure never suppresses the boundary check, which
runs only when every check before it passed) — and only when the run reached them (a verdict
that stopped at a gate records no feature field at all). By the snapshot passed in:
- `None` — nothing: no build, no check, no field, no event.
- `Invalid` — no feature check; `features` = `invalid`; the message of §5.5.
- `Valid` with scenarios, in this order:
  1. **Not part of the program** — the unit's `replaces` are not all among the program's
     top-level `.c` (the rule the shared whole builds impose): every scenario is skipped as
     `not-in-program`. (A unit whose `replaces` names no file at all stays the refusal it is
     today.)
  2. **The C program does not build** — `whole_c` fails to compile or link (a library with no
     `main()`, two `main()`s, a missing library): every scenario is skipped as
     `c-side-build-failed` (never the compiler's text in the verdict; the reason's words send
     the person to `harness features map`, whose refusal shows the compiler's first lines and,
     when the facts do not record exactly one `main` among the top-level `.c`, "features need a
     program with one main()"). The link
     decides, not the facts: the scanner also counts a `main` behind `#ifdef` and misses one
     made by a macro.
  3. Otherwise `whole_mixed` is built — a failure to link it is a failed check for every
     scenario, fail closed, worded without blame: `the mixed program did not link` (the
     candidate, or a plan whose replaced file defines something the rest needs) — and each
     scenario runs C, mixed, C (§4.2), emitting `feature:<feature>/<scenario>` or recording its
     `c-side-*` skip.
  The two builds are the whole-program check's, built once when both are configured (the
  "build the two programs" step is extracted from `whole_program`, returning its outcome as
  data). **When `[oracle.whole_program]` is configured**, it runs first and a failed whole
  build stays the error it is today (a target without features keeps today's behaviour); the
  skip and the failed-check wording apply when only the features need the builds. The verdict
  records `features`, `program` and `features_skipped` (§3).

The reasons in words (the marker, the messages, the feature View, F2's next step):

| reason | words | what to do |
|---|---|---|
| `c-side-unstable` | the C program's output differs between runs | change or remove the scenario |
| `c-side-crashed` | the C program crashed on it | change or remove the scenario (or fix the C) |
| `c-side-timed-out` | the C program took longer than the timeout | shorten the scenario, or raise `[oracle] timeout_secs` |
| `c-side-overflow` | the C program printed more than the output cap | change the scenario |
| `c-side-exec-failed` | the C program could not be started | a sandbox or harness problem — see the details |
| `c-side-build-failed` | the whole C program does not build | fix the build (the details show the compiler's words) |
| `not-in-program` | this unit's files are not part of the program | nothing to do: its verdicts skip the features |

Every scenario runs for every unit — also a scenario whose map says it never runs the unit:
the map is advisory, and a verdict must not depend on it. The cockpit uses the map to say
which checks exercise the unit (§8.4). There is no time budget: each run has the oracle's
per-run timeout, so a verify's feature step takes at most 16 scenarios × 3 runs × the timeout,
and only the C side's runs are the person's to shorten (§13).

### 6.2 The detail

The harness's own words: numbers and exit codes — never the arguments, never the program's
bytes (the verdict is committed, quoted to a model and shown to the chat). And never through
`RunFailure::Failed`, whose message embeds the argv and a stderr excerpt:
- pass: `exit 0; stdout 18234 bytes identical; stderr empty`;
- a mismatch: `exit 0 vs exit 1` / `stdout differs (lens 18234 vs 18230, first diff at byte
  9)` / `stderr differs (…)`, joined with `; `;
- the mixed side did not exit: `candidate run failed: signal 6` / `…: timed out after 120s` /
  `…: more output than the cap` / `…: could not start` — the existing lead-in, which the
  migrate judge's `classify` already reads as a crash or timeout; and `the mixed program did
  not link` (§6.1).
Lengths and offsets count the streams as a person reads them — the program's output with its temp
dir and program dir written as `$TMPDIR` and `$PROGDIR`, and no `$$` escape (§4.1 step 3) — so
they are the same on every machine and, for a program that prints no path, its own byte count.
An offset is the C side's, at the start of the first differing byte or of the token holding it.
(Dogfood on zopfli found the escaped counts: gzip output with one `$` byte read 206 bytes for
the program's 205. The program's raw counts would carry the temp dir's length, which changes
from run to run.) Two runs are "the same" on their end and rewritten streams only.
The cockpit shows the scenario's arguments next to a failed check (it has the features file);
the model and the chat get the id.

### 6.3 Verdict and attempt inputs

`run_verify` fills `VerdictInputs.features`, `.program` and `.features_skipped` after the
feature step (§3); `compute_inputs` stays pure and fills none of them. `migrate` records the
`features` and `program` digests of its run's snapshot in the attempt. `render_md` shows all
three.

## 7. Writing the features file

### 7.1 `harness features init` (CLI only)

```
harness features init [--target DIR] [--json]
```
Takes the writer lock; creates `migration/features/` with `safe_ledger_dir` (never through a
symlink) and refuses when `features.toml` exists or is anything but absent (`lstat`). Writes
the **starter**: a file that validates and holds no feature — `schema_version = 1` and
comments: what a feature and a scenario are, the samples in words, the argument rule, and a
commented example (when `[oracle.whole_program]` is configured, using its flags: "your
whole-program check already runs `-c`; a feature can run other flags too"). Nothing is guessed
about the program. The starter text is one harness-core function (`features::starter`), used
here and by the cockpit's Edit.

### 7.2 `harness features save` — the cockpit's Edit

```
harness features save --expect <blake3-of-current-bytes|none> --bytes N [--target DIR] [--json]
```
The new text on stdin, read by `read_answer`'s rules: a terminal on stdin is refused;
`--bytes` ≤ 64 KiB is checked before anything is read; exactly N bytes are read (a bounded
N + 1 read); the text must be UTF-8. Then, under the writer lock: `migration/features/` is
created or checked with `safe_ledger_dir`; `features.toml` is `lstat`ed — a regular file whose
bytes hash to `--expect`, or absent when `--expect none` (a symlink or a directory is refused);
the text is validated with the loader; refused (exit 1, the loader's message) when it does not
validate; otherwise written atomically. A concurrent harness write is never overwritten
(`--expect` under the lock); an outside editor racing the save is not covered (the lock only
excludes harness writers) — said in SCHEMAS.md.

**The cockpit's Edit** — one flow for "Write your features file" (no file yet) and "Edit the
features file":
1. Refused while a command runs, and when `migration/features` or the file is a symlink
   ("replace the symlink with a regular file outside the cockpit").
2. The file (or the starter, when there is none) is copied into a fresh private temp dir, and
   its digest noted (`none` when there was no file).
3. **Before the screen is handed over**, a dialog in the cockpit says which editor opens and
   how to use it, with **[Open \<editor\>]** and **Cancel**:
   - the editor: `$VISUAL`, else `$EDITOR`, else `nano` when it is on `PATH`, else `vi`; the
     label is the basename of the command's first word ("Edit the features file (in nano)"),
     and the button says it (**[Open nano]**). The hand edit keeps its own default and flow: it
     opens two files, which macOS's `nano` (pico) cannot, and its test pins `vi`;
   - nano: "Type your changes. Ctrl-O then Enter saves; Ctrl-X leaves.";
   - vi: "Press i to type. To save and leave: Esc, then :wq, then Enter. To leave without
     saving: Esc, then :q!, then Enter.";
   - another editor: "Save and close the file to come back here."
4. On return:
   - the editor returned within a second and nothing changed → "The editor returned at once —
     if it opened a window, use its 'wait' option (for VS Code: code -w)";
   - unchanged → "No change.";
   - an editor that exits non-zero after saving keeps the draft (the hand edit's rule);
   - changed → validated in-process:
     - valid → the armed dialog **Save the features file**, worded by what changed: "12
       scenarios in 5 features. From now on every Re-check runs them; verdicts made before
       show 'not checked since you changed them'." — or "Only names changed: no verdict is
       affected." — then `harness --json features save …` with the text on stdin;
     - not valid → the error (with its line and column) and **Edit again** (the editor opens on
       the draft; at that line when the editor is nano, vi, vim, nvim, emacs or micro, which
       take `+N`) or **Discard**.
5. A refused save because the file changed on disk (`--expect`): "The features file changed
   since you started editing. Your draft is kept at <path>." — with **Edit the new file** (the
   draft stays kept) and **Discard my draft**.
6. **The draft is kept** until it is saved or discarded: it joins the kept edits (announced on
   quit with its path, never removed by the cockpit), and the menu offers **Continue my
   features draft** and **Discard my features draft** while it exists.

The hand edit's pieces are reused, not changed: `editor_script` for the command, the suspend
path, `KEPT_EDITS` and the signal path for the draft; a one-file session sits beside the
hand edit's two-file one.

## 8. The cockpit

### 8.1 The model (un-gated, pure): `harness_tui::featmap`

`Snapshot::load` loads the `FeatureSnapshot` once and keeps it (the coverage of §3 needs its
digest); `load::read` builds `FeatureMap::build(&Snapshot, &Files, &MapState)` on the load
worker from the same snapshot — one read of the file per refresh; `MapState` is none,
unreadable (with the loader's message) or loaded (with its currency). harness-mcp's reads pay
one ≤ 64 KiB read and no map. Per feature and per unit it computes **the unit's result for the
feature** from the unit's latest verdict:
- **checked** — the verdict is fresh (`stale` empty) and its `features` and `program` equal
  today's; then, over this feature's scenarios: `failed` if any `feature:` check of it failed,
  else `could not run (reason)` if any of them was skipped, else `passed` if every one of
  them has a passing check, else `absent`;
- **not checked** — no verdict, a stale one, or one whose `features` or `program` differs
  from today's (or is empty or `invalid`);
- **outside** — decided from the plan, first: the unit's `replaces` are not all top-level `.c`
  of `source_dir` (its verdicts skip the features as `not-in-program`). An outside unit takes
  no part in F3, F8, F9 or F10.

It also holds, per feature: its scenarios with the map's record of each; its functions (the
union of its scenarios' `functions`) each mapped to its file and that file's owning unit
(`Files`' owner); the functions outside every unit, counted; per unit its functions touch,
"runs n of its m functions". Per unit: which features run its functions, and whether it has
unwatched definitions. Per function: which features ran it. The project totals: functions
some feature ran / functions watched.

### 8.2 Feature states

"Has Rust" means the unit's status is verified or merged. First match wins. Each state has
its own glyph, listed in Help under **Features** (its own sub-list, apart from the files' and
the units' legends); colour follows meaning (red for ✗, yellow for ⚑ and ↻, green for ✓ and ◉,
dim for the rest) and is never the only signal:

| # | word | glyph | rule | next step (the View's first line) |
|---|---|---|---|---|
| F1 | failing | `✗` | some unit's result for it is `failed` | open the unit (a link) |
| F2 | a scenario cannot run | `⚑` | some unit's result is `could not run` for a `c-side-*` reason, or the current map says a scenario is not stable or did not exit | the reason's "what to do" (§6.1), and Edit the features file |
| F3 | needs a re-check | `↻` | a unit that has Rust and is not `outside` has the result `not checked` or `absent` | Re-check u-…, as links |
| F4 | not mapped yet | `⋯` | no map, or the map lacks this feature, or it cannot be read ("map unreadable", with the reason) | Map the features |
| F5 | map out of date | `≃` | the map is out of date | Map the features (the View says what changed) |
| F6 | map incomplete | `◔` | a scenario's notes are unavailable, or its probe disagreed | the scenario and why |
| F7 | reaches no unit | `∅` | its functions touch no unit | "Its code is outside every unit (n functions), or the map could not watch it. Nothing to do — or add a scenario that reaches a unit." |
| F8 | all its units migrated | `✓` | every unit its functions touch has Rust, and each one's result is `passed` | the caveat below |
| F9 | holds so far | `◉` | some unit it touches has Rust, and each such unit's result is `passed`; the word carries the count: "holds so far · 1 of 6 units" | the caveat below |
| F10 | all C | `◌` | no unit it touches has Rust yet | none |
| F11 | see its units | `·` | none of the above (e.g. every unit it touches is outside) | the units, as links |

- F1 needs no map and comes first: a failure shows even while the map is out of date, and
  whatever else the verdict skipped. A stale red verdict is `not checked` (F3), not `failed`:
  the Rust may have been fixed since.
- F3 considers every unit that has Rust, not only the feature's (a scenario can fail through a
  unit its map never touched) — except units `outside` the features. A skip is never a reason
  for F3: a Re-check could not clear it.
- The state word carries a second condition when one applies below it: "failing · 1 scenario
  cannot run". F9's and F10's words add "· 1 unit has a red verdict" when a unit it touches
  has one.
- **F8 and F9, wherever they are shown** (the View, the summary's footnote on "hold so far",
  Help): "each unit was checked with only its own Rust swapped in — no build has them all in
  Rust together yet"; F8 adds "n functions it runs are outside every unit and stay C" when
  there are any.
- **A valid file in a target whose facts record no single `main()`**: every feature's View,
  the Features View and each unit's features line say "Features need a program with one
  main() — this target has none (a library?); not supported yet" (advisory, from the facts;
  the link decides in verify).

### 8.3 The tree

After `Units (n)`, always, a group **Features**:
- no file: `Features (none yet)`;
- a file that cannot be used: `⚠ Features (error)` — selectable; its View shows the error;
- a valid file: `Features (n)` (`Features (0)` for a starter), open by default, one row per
  feature in file order: the glyph, then the name (display-filtered, cut with `…`; when two cut
  names would read the same, the tail is replaced by ` ·<id>`), then the state word when it
  fits (the glyph and the View's title carry it otherwise).

New selections `Selection::Features` and `Selection::Feature(id)`; `parent()`,
`open_by_default`, `exists`, `surviving`, and every exhaustive match as for `Units`/`Unit`;
`glyph_style` gains an arm per new glyph. No scenario rows (a feature has at most 8; its View
lists them).

### 8.4 The Views

Vocabulary: a feature **runs** functions; the map **watches** the functions it could put a
note in (Help and the top of the Features View say so once). Every map-derived line depends on
the map being current: when it is out of date, those lines are shown under **"From the last
map — out of date (the scan changed / your scenarios changed / the program's C changed / made
on another platform)"**, and no negative claim ("reaches no unit", "none of your features runs
this unit") is made from it. Views put the state and the next step first, details after (54
columns at 80).

**Features (none yet)**: what a feature is in two sentences; when `[oracle.whole_program]` is
configured, "Your whole-program check runs `<its flags>` on three samples; a feature can run
other flags too, and the map shows which units each one reaches."; "On this row, press Enter
and choose Write your features file." When the facts are missing: "Scan the project first."
For a program without one `main()`: the library sentence (§8.2) and no item.

**Features (error)**: the snapshot's message, display-filtered; "On this row, press Enter and
choose Edit the features file." (for a symlink: "replace the symlink with a regular file
outside the cockpit"; for a newer schema: "update the harness"); "Until it is fixed, verdicts do
not run your features — Re-checks and migrations still work, and say so."

**Features (0)** and **Features (n)**:
- (0): "No features yet — choose Edit the features file to add yours.";
- one line per feature: glyph, name, state word; a dim second line: "3 units · 2 pass · 1
  fails · 1 not re-checked" (units, by this feature's result on each); the line links to the
  feature;
- the totals: "Your features ran 104 of the 111 functions the map watches (6 more could not be
  watched)." and, when the map is current, the count of functions no feature ran, with up to
  20 as links to their function rows;
- "Edited features.toml outside the cockpit? Press g to re-read."

**A feature**:
- title: its name; under it its id, its state in words, and the next step (§8.2's column);
- **Scenarios**: per scenario its arguments as the program sees them (`zopfli --zlib -c
  sample_text.txt`, the sample described in words); from the map: `exit 0 · stdout 18234
  bytes · stderr empty` (or its first line); a flag with a one-line reason when its output
  differs between runs, the run with notes behaved differently, no notes were recorded, **it
  compares little** (it exited non-zero or printed nothing to stdout: "its check compares only
  the exit status and stderr"), or a unit's verdict skipped it (the reason's words);
- **Where its code lives**: one row per unit its functions touch — the unit's glyph and word,
  "runs 3 of its 9 functions", and this feature's result on it (`✓ passed` / `✗ failed:
  zlib/text` / `– not re-checked` / `– could not run: <reason's words>` / `– no verdict`) —
  each a link; then "Outside every unit: n functions (headers, files with no exported
  functions)"; then, if any, **"Also fails on (its functions are not in them)"**;
- **Only this feature runs**: up to 20 of its specific functions (with ≥ 2 features), as
  links, or "everything it runs is also run by another feature";
- the F8/F9 caveat when either applies; the library sentence when it applies.

**A unit** — one line in the unit header when a valid features file exists, failures first:
- the map current and complete for every scenario:
  - "3 features run it · 1 failed: Compress to zlib" (or "· all passed", "· 2 not
    re-checked") — the full list, with "runs n of its m functions" per feature, is in the
    Features View, a link;
  - or, when no feature ran any of its functions and it has no unwatched definition: **"None
    of your features runs this unit's functions, so their checks pass whatever its Rust
    does."**; with unwatched definitions: "None of your features ran its watched functions; n
    of its functions could not be watched, so this is not proof.";
  - or, when its verdict skipped the features as `not-in-program`: "Not part of the program
    your features run — its verdicts skip them.";
- otherwise: "Which features run it: not known — map the features";
- the coverage marker (§3) on its verdict line when it is `Behind`.
The Re-check dialog repeats the first line; the Accept dialog repeats which features run the
unit (from the map, without marks — the attempt's own verdict is what Accept judges).

**A function**: its header gains "run by: Compress to gzip, Compress to zlib" or "run by none
of your features" (map current), or "not watched by the map" (`unwatched`).

**Checks** (the unit's checks strip and the verdict overlay): as today, failures first; the
passing feature checks share one chip, "✓ scenarios ×16 (4 run this unit)" (or "(not mapped
yet)" / "(map out of date)"), in check order; the overlay lists each feature check with the
feature's name and arguments, "(its feature does not run this unit's functions)" when so —
for a failed one, "…yet it failed: the map may be incomplete" — and the skipped scenarios with
their reasons' words. `check_words` stays stateless (`feature:zlib/text` → "feature
zlib/text").

**The project summary**: a line "Features: 5 — 1 failing, 2 hold so far¹, 2 all C · 1 unit not
re-checked · 1 scenario cannot run" (only the non-zero counts, each named by its state word;
"not re-checked" counts units with Rust whose result for some feature is `not checked` or
`absent`) — or
"Features: none yet — see Features", or "Features: features.toml has an error — see Features";
¹ the F8/F9 caveat, only when F8 or F9 occurs. Next-step rules after rule 4, each only when its
act is enabled:
5. a valid file with scenarios, and the map is missing or out of date → "Map the features —
   press Enter and choose Map the features";
6. a unit with Rust whose result for some feature is `not checked` or `absent` → "Re-check u-… — your
   features are not checked on it" (the project menu gains **Re-check u-…** for exactly this
   unit while the rule applies, so the Next step's act is where `recommended` looks).

### 8.5 The menu and the acts

| Where | Item | What it does |
|---|---|---|
| Project; Features (none yet) — only when there is no file | **Write your features file (in \<editor\>)** | the flow of §7.2 on the starter; its save is the confirmed `features save --expect none` |
| Project; Features; a feature; Features (error) — only when there is a file | **Edit the features file (in \<editor\>)** | the flow of §7.2 |
| Project; Features; a feature — while a draft is kept | **Continue my features draft**, **Discard my features draft** | §7.2 step 6 |
| Project; Features; a feature — only with a valid file with scenarios | **Map the features** | `harness --json features map --target <root>` (+ `--allow-unsandboxed` through `with_sandbox_flag`, as every act that runs code) — confirmed: "Builds the C program twice in a scratch copy under migration/build/.features — once as it is, once with a note at the start of every function; your C is not changed — and runs each of your 12 scenarios three times. Records which functions each ran in migration/features/map.json. Changes no verdict." |

"Map the features" is greyed with the reason, and re-checked at confirm time like Refresh the
plan: busy; the facts are missing or stale ("Scan the project first"); no sandbox. (No
single `main()` in the facts is said beside it, not a grey-out: the link decides.) `recommended` focuses Write on `Features (none yet)`, Edit on `Features (error)`, Map
on a feature in F4/F5, Edit on a feature in F2, and — on the project — the Next step's act.

**No other act is gated by features**: Re-check, Accept, Retry, Modify, hand edits and the
chat's acts run with whatever snapshot their command loads; their verdicts carry the marker
when the features could not all run. Dialog sentences: Re-check — "It also runs your 12 feature
scenarios on the whole program." (or "Your features file has an error: this Re-check will not
run your features." / "n of your scenarios cannot run and will be skipped: <reasons>" when the
current map or the latest verdict says so); Retry, Modify, Resume and the chat's Continue — "Each
judged turn also runs your 12 feature scenarios."; Scan — "A change in the C makes the features
map out of date."

Progress: the `scenario` events of `features map` narrate in the activity panel ("Mapped
zlib/text — exit 0, 104 functions (3 of 12)"); its result line counts the scenarios that need a
look ("Mapped 12 scenarios — 2 need a look"). A Re-check's result line adds the skips ("GREEN —
all 22 checks passed · 2 of your scenarios could not run"). After a save or a map, the result
line adds "commit migration/features/ with your work"; Help says it too.

### 8.6 Help

A **Features** section: what a feature and a scenario are; the file's shape (the §2.1
example); the samples in words; the argument rule (no `/`, runs in an empty folder); "runs" and
"watches"; the states and their glyphs (the Features legend); that changing a scenario means
verdicts made before show "not checked since you changed them" until re-checked (renaming does
not); the reasons a scenario cannot run and what to do (§6.1's table); that each unit is checked
with only its own Rust; that a features file with an error never blocks a Re-check; the Edit
dialog's editor instructions; "press g after editing outside the cockpit"; "commit
migration/features/ with your work".

### 8.7 Reading the files in-process: the preflight

`preflight::check` gains nothing that can fail the read: the features file's problems are the
snapshot's `Invalid` (§2.2), and the map's are its loader's value (§5.2). The cockpit reads
`features.toml` (≤ 64 KiB) and `map.json` (≤ 16 MiB) only through `read_regular`, both counted in
the preflight's byte budget when present (a link or an oversize file is the loaders' value,
never a refused read). The program digest (§2.4) reads the program's files: with a features
file, the preflight counts them — each file once, however many paths reach it — against the
hash budget, and refuses more than 50 000 of them (review T1: a thousand links to one large
file made every read hash tens of GiB). Samples are generated in memory. Verdicts'
`features_skipped` entries are parsed strictly (§2.3).

## 9. The chat and harness-mcp

The chat cannot read files (`--tools ""`; its tools are harness-mcp's reads). harness-mcp's unit
report gains one field, `features`: the coverage of §3 as its closed strings (`"current"`, or
the reasons), rendered through the same closed-value path as its other enumerations — never
the skip list's text. Its brief gains:
- "A `feature:<feature>/<scenario>` check runs one of the person's scenarios on the whole
  program. A passing one says nothing about a unit its feature does not run — the cockpit's
  Features view shows which do. Report feature checks separately from the others."
- "A verdict with no `feature:` checks says nothing about the person's features — never report
  them as passing. The unit's `features` field says whether they were checked: `current`, or
  why not."
- "You cannot see the features file or the map. Name a scenario by its id; you do not know its
  flags."
- The list of the person's own menu items gains Write/Edit the features file and Map the
  features.

## 10. Contracts and compatibility

- **No features file → today's bytes**: verify's checks, the verdict JSON, attempt records,
  `state status` output and events, harness-mcp's reports, the migrate prompts, the bench's
  replays and scores. Proved by the mini-target and zopfli verify tests (exact check lists, run
  on copies without the file) and by `bench check --replay` after the scenario checks (§12).
- New schemas in SCHEMAS.md: `ruharness-features` v1 (strict; every new key bumps the
  version), `ruharness-features-map` v1; `VerdictInputs.features`/`.program`/
  `.features_skipped` and the closed reason set; `AttemptRecord.features`/`.program`;
  `UnitReport.features`, the `unit` event's and harness-mcp's field; the `scenario` event and
  verify's messages; `features init`, `features save`, `features map`, their flags and exit
  codes; `OracleStrategy::verify_with`; the writer table; the trust boundaries (both files and
  the skip list hostile; the map shaped by its own C; the `.inc` gap; the unsandboxed
  residuals of §4.1; the outside-editor race of §7.2).
- `feature:` checks join the check vocabulary. Readers keyed on names: the migrate judge (the
  mixed side's lead-ins), the cockpit's chips and overlay, the bench (generic).
- docs/TUTORIAL.md gains a "Features" section; README a line.

## 11. Code: where each piece goes

| Crate | Change |
|---|---|
| harness-core | `features`: types, strict parser (line/column), `FeatureSnapshot`, `starter`, `load_map`, the digests, `Sample`; the `program` digest over the whole build's inputs; `VerdictInputs` and `AttemptRecord` fields; `UnitReport.features` (`unit_report` takes today's digests); `OracleStrategy::verify_with` (required) and `verify` (provided); `walk` gains the all-files mode with a byte cap; `Ledger` paths |
| harness-scan | `probe_source` (pure); `FnDef` gains the body's start; the unwatched cases |
| harness-oracle | `Confinement::run_scenario` (the one path per run, cwd, the one-to-one rewrite, the profile additions, the group kill, the outcome enum with `ExecFailed`); the samples from core; the shared whole builds; the scenario checks and skips; `map_features` (mirror, builds, runs); the probe runtime; one confinement setup shared by verify, boundary and map |
| harness-llm | `verify_with` in the judge under the run's snapshot; the attempt's digests; the replay's rule (§3); the brief's sentences |
| harness-cli | `features init`, `features save`, `features map`; the snapshot per run for verify/migrate/promote/override; the `unit` event's field |
| harness-mcp | the unit report's `features` field |
| harness-tui | `featmap` (un-gated); `load::read`; tree, Views, chips, overlay, summary and Next step, menu (the Edit flow for write and edit, Map, the kept draft), the editor dialog (a one-file session beside the hand edit's), `recommended`, narrate, Help |
| docs | SCHEMAS.md, TUTORIAL.md, README, this file |
| targets/zopfli | last: `migration/features/features.toml` (gzip, zlib, deflate, verbose, iterations, help, no file) and its `map.json`; u001 re-checked; the tests that load zopfli adjusted in the same commit |

## 12. Order of work and tests

Each step committed when green (`cargo fmt --check`, `clippy -D warnings`, `cargo test
--workspace`):

1. **Core** (built on the second revision; §R3 lists what changes): the parser (every §2.1
   rule, with its message; line and column), the snapshot (each problem a value), the digests
   (names excluded; sample bytes, the program's file name and the timeout included;
   order-independent), the program digest (a missing file; an unscanned top-level `.c`; the
   include closure only), coverage in `UnitReport` (every reason; no file → `None` and today's
   bytes), the attempt's fields.
2. **Scenario checks** (closes the spike's gap by itself): `run_scenario` (argv[0], the program
   directory and the input's path printed the same on both sides; `$$`; cwd; every end;
   `ExecFailed` for exits 65 and 71; the profile additions; the group kill), the shared builds,
   the checks after boundary, the details (a hostile program's stdout and the scenario's
   arguments never appear), every skip reason with no demotion from one, a slow or crashing
   mixed side as a failed check never a skip, the mixed link failure, C–mixed–C, `verify_with`
   at each call site, the replay rule, the messages; on the mini target (a toy program with two
   behaviours, a usage line printing argv[0], a scenario that writes a file, one that exits 1 on
   both sides, one unstable, one that times out on C, one in a unit outside the program, a
   second `main`) and on a test-time copy of zopfli with features. Then `bench check --replay`.
3. **The map**: `probe_source` (lines unchanged, `#if` variants, a brace inside `#if`, ERROR
   nodes, K&R, two functions of one name, macro-defined functions), the mirror (a `.inc` file,
   `__FILE__`, `../` refused, `source_dir` = root excludes `migration/`), the runtime (a crash
   keeps notes, a forged or torn file → unreadable, `ulimit -n 256`), `features map` (every
   refusal, the events, interrupted → no file), `features init` (never overwrites; through no
   symlink; the starter validates), `features save` (every refusal; `--expect`; stdin rules).
4. **The cockpit**, three commits: (a) `featmap` over fixtures (every unit result; every
   F-state and its order; a skip never yields F3; an incomplete or out-of-date map never yields
   F7 or a negative unit line; outside-units failures; specific functions) + tree + the Features
   and feature Views (goldens); (b) the unit, function, chips, overlay, summary and Next-step
   lines; (c) the Edit flow (a fake editor: write, edit, invalid → edit again, refused save,
   kept draft, the editor dialog), Map, `recommended`, Help, and a pty test: Map from the menu,
   the map appears.
5. **harness-mcp's field; the chat brief**; the live chat tests (the brief changed).
6. **Dogfood**: zopfli's features file and map, u001 re-checked, the zopfli-loading tests
   adjusted in the same commit; the cockpit driven headless on it.

Then the adversarial code review (3–4 lenses), fix pass, its check, mutation checks of the
named rules (the id alphabet in check names; the argument grammar; the digests' exclusions and
inclusions; the coverage marker never entering `stale`; C–mixed–C and the skip rule; the pass
rule of §4.2; `ExecFailed`; the one-to-one rewrite; the unit results and F1–F10 order; no skip
from the mixed side), and the DECISIONS handoff.

## 13. Later, and decided separately

- **A cumulative mixed build** (every verified unit in Rust together, every scenario): the
  end-state guarantee a user cares about. Revisit when a second unit of one target is verified.
- **Library features** (no `main`): a person-written scenario program under
  `migration/features/programs/`. Revisit when a library target wants features.
- **Inputs of the person's own**, stdin, environment, output files: revisit when a target's
  behaviour needs a specific input format, reads stdin, or writes the result to a file (zopfli
  without `-c` — the map already flags such a scenario as comparing little).
- **Folding `[oracle.whole_program]` into features** (its flags + the three samples are
  scenarios; its comparison would then use §4.2's rule): revisit when a second target adopts
  features; then the flag grammar, its constant and its check go, with a migration note for
  recorded verdicts.
- **Scenario cost** — every judged run runs every scenario three times, bounded only by the
  per-run timeout (a wall-clock budget was tried in the design and removed: the candidate could
  spend it and make later scenarios disappear). Revisit when scenario runs pass a quarter of a
  verify's time: cache the C side keyed by the program digest, the features digest and the
  toolchain (deterministic, and the candidate cannot touch it); or run only scenarios the map
  says reach the unit, as a documented, non-default option.
- **Per-scenario coverage digests**, so adding a scenario leaves the others' verdicts current;
  with it, a "Re-check every unit with Rust" act: revisit when re-checking after an edit is the
  obstacle people hit.
- **The limits** (16 features, 16 scenarios, 8 args): revisit when a real target hits one.
- **Line-level maps**: revisit when two features' function maps are equal but they behave
  differently.
- **Proposed features**: from `main`'s usage text (deterministic), or by a model in chat —
  proposals only, the person accepts by saving the file. Revisit when writing the file is the
  obstacle.
- **A harness-mcp read of the features and the map** (names fenced as untrusted): revisit with
  the chat's next change.
- **Non-C includes in the program digest**: revisit when a target includes `.inc`/`.def` files.
- **The scanner's gaps** the spike found (header `static inline` calls unresolved across files;
  function values without an edge; digraphs): separate tasks; the probe instruments
  definitions, not calls, so they do not affect the map.

## R. Design review — history

§R and §R2 name sections and rules of the first and second revisions; where a later revision changed a rule, §R3 (or §R2) says so — the rule text above governs.


Four reviewers on the first draft (a618c34): safety & trust (SAF-1–15), engine & contracts
(ENG-1–19 + 5 nits; 84 in all — the first draft of this section said 88), usability & honesty (USE-1–28), scope & simplicity (SCO-1–17). Two
verifiers checked every factual claim against the code: none refuted; SAF-7, ENG-5, ENG-6,
USE-1, USE-6, USE-18, USE-20, SCO-5, SCO-7, SCO-10, SCO-15 and SCO-17 partly (the part that
held is resolved below); the rest of the opinions had their premises confirmed. One claim was
refuted inside a finding: SCO-10's "F3's 'lacks one of this feature's checks' is dead" — a
verdict that stopped at an early gate lacks the feature checks without being stale — so F3
keeps it. Found by several lenses at once: argv[0] (ENG-1, USE-7), the path grammar (SAF-3,
ENG-2, SCO-12, USE-11), the fail-open pass rule (SAF-4/5, ENG-3), the missing sandbox gate
(SAF-2, ENG-10, SCO-6), the digest on every read path (SAF-7, ENG-8, SCO-1), the chat's
imagined file access (SAF-6, USE-2), the overclaiming states (SAF-1, USE-3/4/6).

| Finding(s) | Resolution (section) |
|---|---|
| SAF-1, USE-3, USE-4, USE-5, USE-6, USE-21 — states and the unit line claimed more than the evidence | F1–F9 rebuilt: failing first and needing no map; "fix its scenario"; re-check over every unit with Rust; map incomplete before any negative claim; F6 reachable; ✓ only when every unit is migrated, else "holds so far · k of n"; outside-units failures listed; negative lines only from a current, complete map, worded about watched functions (§8.2, §8.4) |
| SAF-2, ENG-10, SCO-6 — no sandbox gate | `--allow-unsandboxed` + the refusal; the act through `with_sandbox_flag` (§0, §5.1, §8.5) |
| SAF-3, ENG-2, ENG-9, SCO-12, USE-11 — args as paths; cwd = target root; absolute input paths in output | runs in their own empty temp dir with the binary and input copied in; `{input}` a bare fixed name; no `/`, no `..`; the grammar's safety stated as resting on the directory and sandbox (§2.1, §4.1) |
| ENG-1, USE-7 — argv[0] differs | argv[0] = `./<name>` on both sides and in the map (§4.1) |
| SAF-4, SAF-5, ENG-3 — fail-open "same kind of end"; lost signal numbers; details through `RunFailure::Failed` | a new status with signal numbers and overflow; the C side must exit normally; the mixed side's kill, timeout, overflow never pass; details never through `RunFailure::Failed` (§4.1, §4.2, §6.2) |
| ENG-4, USE-8, SAF-12, SCO-13 — unstable C side demotes and burns turns; blame on both-timeouts | C runs twice; an unusable scenario is an error: no verdict, no demotion, the judge stops; the map reports it first; slow scenarios flagged; cost trigger (§6.4, §5.2, §13) |
| USE-9, SAF-8, ENG-8, SCO-1, SAF-7, SAF-13/ENG-14, SAF-15 — features staleness floods state 7, CONTRADICTION, migrated share; other units' C unhashed; digest per unit on every read path; inputs hashed unsafely; samples by name | coverage is a marker, not `stale` (fence, contradiction, `fresh_green`, bench unchanged); a `program` digest; both digests once per read, no file I/O but `features.toml`; an invalid file is a value, never an `Err`; `inputs/` deferred; samples by bytes (§2.4, §3) |
| ENG-7 — attempts mix check sets | `AttemptRecord.features`; replay: superseded; resume refused on a change (§3) |
| SAF-6, USE-2 — the chat cannot read files | the claims removed; the brief says what the chat can and cannot see (§9) |
| SAF-9 — the committed map is hostile | strict loader: ids, `end` grammar, pairs present in today's facts, sizes; display-filtered; never gates (§5.2) |
| SAF-10, ENG-17 — probe forgeable, fragile | `O_CLOEXEC|O_NOFOLLOW`, a descriptor ≥ 900, parenthesised call; the residual disclosed as the boundary map's is (§5.4) |
| SAF-11 — forked children race the read | the group killed when the leader exits (§4.1) |
| SAF-14, SCO-16 — check names cut at 64 | ids ≤ 24 (§2.1) |
| ENG-5 — mirror include semantics, `__FILE__` | mirror every regular file under `source_dir`; `-fmacro-prefix-map`; `../` includes fail loudly (§5.3) |
| ENG-6 — brace inside `#if`, ERROR nodes, duplicate names | unwatchable definitions listed in `unprobed`, never "not run"; ids per (file, name) (§5.3) |
| ENG-11, USE-15, USE-10, SCO-14 — no `main`; `replaces` rule; typos found late | checked at the command's start with words; menus greyed with reasons, re-checked at confirm; argv in the failure detail (§2.2, §6.1, §6.2, §8.5) |
| ENG-12 — boundary suppressed | feature checks run last (§6.1) |
| ENG-15 — facts hash too narrow | the facts file's canonical bytes; platform in the map's inputs (§5.2) |
| ENG-16, SCO-15 — recovery claim; build-dir collision; `Base::new` needs a unit | no recovery (no unit touched); `build/.features/`; one extracted confinement setup (§5.1, §1, §11) |
| ENG-18 — strict keys | every new key bumps `schema_version`, stated (§2.1) |
| ENG-19, SCO-7, SCO-8 — the zopfli fixture (~55 tests), coarse steps, late bench | the no-file proof on copies without the file; zopfli's features land last with the tests adjusted; scenario checks before the map; bench after the core and oracle steps; cockpit in three commits; the listed tests (§10, §12) |
| USE-1, SCO-17 — no empty state, blank-page authoring | the group always shown; `features init` writes a validating starter; Edit the features file; the library case said (§7, §8.3–§8.5) |
| USE-12, USE-13 — "Observe" collides with the model triage; glyph clashes | `features map`, "Map the features", "not mapped yet"; the features' own glyphs and legend (§5, §8.2) |
| USE-14, USE-16, USE-19 — Next step's act not on the project menu; progress; dialog wording | the act on the project menu; `n`/`of` in events, build messages, a result count; no duration promised; dialog sentences (§5.5, §8.5) |
| USE-17, USE-18 — trivial passes swell counts; chips | feature passes grouped into one chip with "k run this unit"; the overlay marks checks whose feature does not run the unit (§8.4) |
| USE-20 — map lines from an out-of-date map | shown only under "From the last map — out of date (why)"; no negative claim from it; platform and program digests as inputs (§8.4, §5.2) |
| USE-22, USE-23, USE-24, USE-25, USE-26, USE-27, USE-28 — narrow names, `[[feature.scenario]]` order, wording, `g`, the error row, focus, links | the id after colliding cut names; flat `[[scenario]] feature = …`; samples in words; "Press g" lines; `Features (error)` selectable; Map focused in F4; links to function rows (§2.1, §8.3–§8.5) |
| SCO-2 — `features status` and its events have no consumer | cut; `features init` added instead (USE-1) (§5.5, §7) |
| SCO-3 — `inputs/` | deferred with a trigger (§0, §13) |
| SCO-4 — two whole-program mechanisms | the builds shared; folding `[oracle.whole_program]` into features is a §13 trigger; zopfli keeps both for now |
| SCO-5 — the new run primitive | named and specified (§4.1, §11) |
| SCO-9 — trim the map | toolchain strings dropped; `footprint` complete/unavailable + reason; `stable` kept — the map is where the person first learns a scenario is unusable (§5.2) |
| SCO-10, SCO-11 — trim the cockpit | kept: specific functions and "run by" (the view's point: which behaviours touch which code); `description` dropped; `check_words` stateless (§8.4) |
| Nits (ENG) | signal numbers kept; `render_md` shows the digests; `seen` at least one byte; the unusable-scenario error has its own words |

(The §R rows name sections of the first revision; where the second revision moved a rule, §R2
says where it went.)

## R2. Check of the revision — 58 findings, resolved in the second revision

Three checkers on the first revision (a4d806a): a row-by-row check of §R (62 resolved, 9
partial, 8 declined — 6 holding, 2 weak — 1 nit unresolved, and 16 inconsistencies inside the
document), an engine & safety check (CHK-E-1–15), a usability check (CHK-U-1–17). The engine
checker ran probes in a scratchpad (sandbox-exec's exit on a failed exec, a binary rewritten
in its own writable directory, `F_DUPFD_CLOEXEC` under `ulimit -n 256`); none was refuted on
reading the code.

The one decision that changed the shape: **nothing about features blocks work**. The first
revision refused `verify` (and so every Re-check, migration, promotion and hand edit) on an
invalid file and on a scenario whose C side is unusable, and then needed gates on every act,
a persisted record of the refusal, a start-of-migrate C-side check, and still spent a paid
turn (CHK-U-4/5, CHK-E-7, SAF-12). Now each such case is a **skip recorded in the verdict**
with a closed-set reason, and the cockpit shows it (§0, §3, §6.1).

| Finding(s) | Resolution in the second revision |
|---|---|
| CHK-E-1, CHK-E-2 — resume refusal strands attempts (and chat hand-offs); superseded breaks `--retry` and old attempts | no resume refusal (a resume re-judges every turn already); replay judges with the recorded features when it can, else reports a changed judge, never superseded (§3) |
| CHK-E-3 — `sandbox-exec` exec failure exits 71 and passes vacuously | `ExecFailed` from exit 71 + `sandbox-exec: execvp()`; C side → skipped, mixed side → candidate failure (§4.1, §4.2) |
| CHK-E-4 — the copied binary is writable by its own run | no copy into the temp dir: per-side build dirs, argv[0] absolute, `$PROGDIR` replaces the directory in both streams (§4.1) |
| CHK-E-5, inconsistency 2 — argv in details reaches prompts and the chat | details carry numbers and exit codes only; the id names the scenario; the cockpit shows arguments (§2.3, §6.2) |
| CHK-E-6, CHK-U-5, ENG-11 residual — units outside the program blocked | skipped as `not-in-program`; the unit line says so (§6.1, §8.4) |
| CHK-E-7, CHK-U-4, SAF-12, SCO-13 — unusable scenarios after paid turns; C,C,mixed weak; no budget | C–mixed–C; an unusable C side is a skip, never an error or evidence; a `4 × timeout_secs` budget, the rest skipped as `budget` (§4.2, §6.1) |
| CHK-E-8 — the group kill | where it runs, opt-in, the `setsid` residual (§4.1) |
| CHK-E-9, inconsistency 11/12 — how the snapshot reaches the oracle; `override` | `FeatureSnapshot` loaded once per command, `verify_with`; `verify` loads its own; `override` listed; preconditions checked in verify (§2.2, §6.1) |
| CHK-E-10 — program digest vs the build's file set; missing files | facts files + every top-level `.c`, `(path, hash or missing)` through one core function; the `.inc` gap named (§2.4) |
| CHK-E-11 — argv[0] and the timeout not in the digest | added (§2.4) |
| CHK-E-12, inconsistency 9 — the mirror's walk as described does not exist; `source_dir` = root | a new all-files walk mode with file and byte caps; `migration/` and `.git/` excluded; canonical paths (§5.3) |
| CHK-E-13, SAF-10 residual — `F_DUPFD_CLOEXEC` ≥ 900 fails under a 256 soft limit | `min(900, soft limit − 16)`, else the original descriptor (§5.4) |
| CHK-E-14 — relative program unsandboxed | absolute path always (§4.1) |
| CHK-E-15 — a recorded digest with the file since deleted; harness-mcp's rendering | coverage `None` when there is no file now; §9 says harness-mcp adds no coverage |
| CHK-U-1 — covering verdicts ignored freshness | covering = fresh and `Current`; a stale red counts as F3 (§8.2) |
| CHK-U-2, SAF-1/USE-6 residual — the negative line vs unwatched definitions; granularity | "runs 2 of its 9 functions"; the unconditioned sentence only without unwatched definitions, else "not proof"; F7 "runs no watched code"; marks defined (§8.4, §8.2) |
| CHK-U-3 — vi for a non-vim audience; editing the ledger in place; errors without lines | `nano` before `vi`, named, with how to leave; edit a private copy, validate, confirmed `features save` with `--expect`; Edit again / Discard; line and column (§7.2, §2.1) |
| CHK-U-6 — preflight failures would break the whole read | the features files never fail preflight: problems are values (§8.7, §2.2) |
| CHK-U-7 — the chip hidden, mis-nouned, undefined without a map | failures first, then one chip before other passes, "features 16 ok · 4 reach it" / "· map out of date" (§8.4) |
| CHK-U-8, USE-13 residual — glyph collisions and room | one glyph per state (✗ ⚑ ↻ ⋯ ≈ ◔ ∅ ✓ ◈ ◌), `glyph_style` arms, name advice ~14, the id replaces the tail on collisions (§8.2, §8.3, §2.1) |
| CHK-U-9 — "not mapped yet" for an out-of-date map | F4 not mapped yet / F5 map out of date; another platform shown, not discarded (§8.2, §5.2) |
| CHK-U-10 — the marker's wording and reach | per reason wordings, on the verdict line; the summary counts units not checked on the features (§3, §8.4) |
| CHK-U-11, USE-14 residual — summary silent; Next step without its act; `recommended` for the new nodes | summary states for none/error/counts; rules 5 and 6 only when enabled; `recommended` per node (§8.4, §8.5) |
| CHK-U-12 — the starter guessed and duplicated; examples zopfli-only | a starter with no features and a commented example from the target's own flags; generic Views; only the applicable item (§7.1, §8.4, §8.5) |
| CHK-U-13 — the brief | name scenarios by id, no flags; report feature checks separately; the menu items (§9) |
| CHK-U-14 — 80 columns and jargon | state and next step first; plain words ("notes", "the run with notes"); caps of 20; the Map dialog wording (§8.4, §8.5) |
| CHK-U-15 — bulk re-check | deferred with its trigger (§13) |
| CHK-U-16 — a demoted unit reads "all C" | F10's word adds the unverified Rust (§8.2) |
| CHK-U-17 — nits | `Features (0)` defined; `⚠` on the error row; the commit reminder; the overlay's "yet it failed"; verify's scenario message (§8.3–§8.5, §5.5) |
| Row check: ENG-17 — the call on every entry; the first-open race | the inline `seen` test; the race stated (§5.3, §5.4) |
| Row check: SCO-4, SCO-16 — reasons not stated | the comparison differs (§4.2's pass rule) and folding is §13's; the mirror copies every file because includes of non-C files must resolve (§5.3) |
| Row check: SAF-7 residual — `SchemaTooNew` on a read path | `Invalid`, saying a newer harness wrote it (§2.2) |
| Row check: SAF-9 nit — `unwatched` pairs not validated | validated like `functions` (§5.2) |
| Inconsistency 1 — "88" | 84 (§R) |
| Inconsistencies 3–5, 7, 10, 13 — §-references, `fx/`, "three acts", the caveat's reach | fixed; the writer table lists only real paths; two acts and a flow (§8.5); the caveat wherever F8/F9 are shown |
| Inconsistency 6 — the judge's error vs verify's error | there is no unusable-scenario error any more (§6.1) |
| Inconsistency 8 — `slow` in a deterministic file | dropped (§5.2) |
| Inconsistency 14 — "footprint" clashes with the boundary check's | the field is `noted`; the words are "notes" and "functions it ran" |
| Inconsistency 15 — `[file, name]` | defined as `[file, canonical id]` (§5.2) |
| Inconsistency 16 — two reads per refresh | `Snapshot::load` keeps the snapshot; `load::read` uses it (§8.1) |
| ENG nit — digest order vs run order | stated as intended (§2.4) |

## R3. Scoped check of the second revision — 37 findings, resolved in the third

Two checkers on 6ef128b: engine & safety (R2-E-1–15; two harmless `sandbox-exec` probes) and
usability & consistency (R2-U-1–22; a width probe with the workspace's `unicode-width`). One
high each, both on how skips were handled.

| Finding(s) | Resolution in the third revision |
|---|---|
| R2-E-1 (high) — the wall-clock budget counted the mixed side: a slow candidate could spend it and make later scenarios disappear, green; and wall-clock skips made verdicts nondeterministic | **the budget is gone**: every run has the per-run timeout, a slow mixed run fails its own check, no skip is ever decided by the candidate (§0, §4.2, §6.1); cost is §13's trigger (a C-side cache keyed by digests) |
| R2-U-1 (high), R2-E-2 — any skip made a verdict non-covering: every feature stuck in F3, F1 hidden behind it, rule 6 looping | coverage per feature: each unit's **result for the feature** (`passed`/`failed`/`could not run`/`absent`/`not checked`/`outside`) from a fresh verdict whose digests match; F1 from any such verdict whatever it skipped; F3 only for re-checkable reasons; `not-in-program` units `outside`; the marker separates re-checkable reasons from `skipped` (§3, §8.1, §8.2, §8.4) |
| R2-E-3 — no reason for a failed whole build; two `main`s; a mixed link failure unspecified | `c-side-build-failed`; `main-count` (not exactly one); a mixed link failure is every scenario's failed check (§6.1) |
| R2-E-4 — save/init through a symlinked directory; stdin | `safe_ledger_dir`, `lstat` of the file, `--expect none` = absent; `read_answer`'s stdin rules; the outside-editor race stated (§7.1, §7.2) |
| R2-E-5 — early-stopped verdicts read as covering | the fields are filled after the feature step only (§3, §6.1, §6.3) |
| R2-E-6 — the run profile is `allow default`: a survivor could signal the next C run | `(deny signal (target others))` and `(deny process-fork)` on scenario runs; the unsandboxed residual stated (§4.1, §4.2) |
| R2-E-7 — replay gaps | `invalid` replays with `None`; the attempt records `program`; a divergence is `Diverged` with an explaining first line; the replay picks its own snapshot, the following live run the command's (§3) |
| R2-E-8 — `verify` vs `verify_with`; call sites | `verify_with` required, `verify` provided; the four call sites named; `promote_attempt` takes the snapshot (§2.2) |
| R2-E-9, R2-E-10 — different directories per side; the path token not one-to-one | one path per run (`<build>/f/<name>`, copied before each run); `$` escaped as `$$` first, then `$TMPDIR`, then `$PROGDIR` (§4.1) |
| R2-E-11 — a profile error exits 65 and would pass vacuously | `ExecFailed` for `sandbox-exec: ` + 65 or 71 (§4.1) |
| R2-E-12 — the group kill's pgid reuse | stated as a residual (§4.1) |
| R2-E-13 — `compute_inputs` is pure | the fields are filled in `run_verify` (§6.3) |
| R2-E-14 — the skip list is hostile on disk; the chat sees no coverage | strict parsing (§2.3, §8.7); harness-mcp's unit report gains the closed `features` field (§9) |
| R2-E-15 — the program digest over all facts files; symlinked top-level `.c` | the top-level `.c` and their include closure; canonical paths as the build resolves them (§2.4) |
| R2-U-2 — F2 outlived the fix | F2 reads only verdicts whose digests match today's (§8.1, §8.2) |
| R2-U-3 — the refused save; the draft's life | a refused save keeps the draft (Edit the new file / Discard my draft); Continue/Discard in the menu; the draft is a kept edit announced on quit; non-zero editor exits keep it (§7.2) |
| R2-U-4 — Start vs Edit; too many steps | no Start item: **Write your features file** opens the starter and saves with `--expect none`; `features init` is CLI only (§7, §8.5) |
| R2-U-5 — the pre-editor line hidden; vi for this audience | a dialog in the cockpit before the editor opens, with per-editor instructions ([Open nano] / Cancel); one editor choice shared with the hand edit (§7.2) |
| R2-U-6 — F7's word | "reaches no unit", with its next step (§8.2) |
| R2-U-7, R2-U-8 — skips silent while running; words for one reason only | verify's messages per skip; the result line counts skips; one words-and-what-to-do table for every reason; the marker's order (§5.5, §6.1, §3, §8.5) |
| R2-U-9 — no single `main()` with a valid file | the library sentence on the Features and feature Views; states stop at F4 (§8.2) |
| R2-U-10 — glyphs echoing the unit legend; colours | a Features sub-list of its own in Help; colours by meaning; `≈` → `≃` and `◈` → `◉` (not East Asian Ambiguous) (§8.2) |
| R2-U-11, R2-U-12 — the save dialog's sentence; `+N`; an editor that returns at once; the label; `handedit` | the sentence by what changed; `+N` for a named list of editors; the "returned at once" hint; the label from the command's first word; `handedit` generalized (§7.2, §11) |
| R2-U-13, R2-U-14, R2-U-15 — the unit line, the chip, the summary at 54 columns | "3 features run it · 1 failed: …" with details in the View; "✓ scenarios ×16 (4 run this unit)" in check order; the summary lists non-zero counts by state word, the footnote on "hold so far" only (§8.4) |
| R2-U-16 — vocabulary | "runs" and "watches", defined once (§8.4, §8.6) |
| R2-U-17 — the error View for a symlink; an unreadable map | the specific next step; "map unreadable" under F4 (§8.2, §8.4) |
| R2-U-18 — the project menu has no per-unit Re-check; the Accept dialog's line | a project item **Re-check u-…** while rule 6 applies; Accept repeats which features run the unit, without marks (§8.4) |
| R2-U-19 — the chat reads a skipped verdict as passing | the brief's sentence and harness-mcp's field (§9) |
| R2-U-20 — hidden second conditions; F9/F10 wording | the state word carries a second condition; both mention unverified Rust (§8.2) |
| R2-U-21, R2-U-22 — nits; §R rows stating reversed rules | fixed; §R and §R2 are marked as history, the rule text governs |

**What this changes in step 1 (committed c25bff5)**: the program digest's file set (the
top-level `.c` and their include closure, canonical for symlinks — not every facts file);
`AttemptRecord.program`; the replay rule's `invalid`; `features::starter`; the reason set
(`budget` and `no-main` gone; `c-side-build-failed` and `main-count` new). The coverage enum
and its reasons stay; the per-feature results are the cockpit's (§8.1).

## R4. Scoped check of the third revision — 11 findings, resolved in the fourth

One checker on 0e8e7b4 (both lenses; harmless `sandbox-exec` probes of fork, spawn, signals,
abort and copy-over; pico and vim driven through a pty). No high; five medium, each resolved
as the checker proposed, with local edits — not checked again (as Build C's and D's last
passes; the build's own review follows).

| Finding | Resolution |
|---|---|
| R3-1 — `(deny signal (target others))` means "outside the run's process group", which the run can change | `(deny signal)` `(allow signal (target self))` (§4.1); a test that a run cannot signal its parent (§12 step 2) |
| R3-2 — `(deny process-fork)` turns a forking program's run into its error path on both sides | said in §4.1 and Help; such scenarios are flagged as comparing little |
| R3-3 — with `[oracle.whole_program]` configured a failed whole build is an error before the features run | stated: it stays today's error then; the skip and the failed-check wording apply when only the features need the builds (§6.1) |
| R3-4 — `main-count` from the facts miscounts `#ifdef` and macro `main`s, and is redundant with the link | removed from the reason set: the link decides (`c-side-build-failed`, with the one-`main` hint); the facts' count is advisory in the cockpit and in the map's refusal message (§5.1, §6.1, §8.2) |
| R3-5 — the hand edit opens two files; macOS nano (pico) takes one; vim's `:wq` stops on the first of two | the hand edit keeps its default and flow; the features Edit alone defaults to nano (§7.2) |
| R3-6 — a mixed link failure blamed on the candidate | worded without blame, still a failed check (§6.1) |
| R3-7 — an outside unit touched by a feature matched no state; its precedence | `outside` decided from the plan first; excluded from F3/F8–F10; a catch-all F11 (§8.1, §8.2) |
| R3-8 — "Re-check it" for `invalid` now and for a gate-stopped verdict | the wording depends on the case (§3) |
| R3-9 — the unit line under no single `main()` | the library sentence there too (§8.2) |
| R3-10 — `$$` through `run_with` would double-escape | `run_raw_with` (§4.1) |
| R3-11 — nits: `absent` in rule 6; "has Rust not yet verified"; the hard-coded button; the marker order | fixed (§8.4, §8.2, §7.2, §3) |

**What this changes in step 1 (5e4d1b9)**: `SkipReason::MainCount` goes.

## R5. Code review of the build — 35 findings, resolved in the fix pass

Four lenses on a38c2aa (trust, oracle, probe and map, cockpit and docs), each finding
checked by an independent verifier against the code: 33 confirmed (some re-rated), 2 refuted
as design-accepted (M4: a definition the facts do not name gets nothing, §5.3; O8: a
candidate as slow as the timeout fails, §4.2). The rule text above is updated where it
changed.

| finding | resolution |
|---|---|
| C1 (high) — an unchanged editor return deleted a kept draft | the draft keeps the text it started from; only a draft equal to it is dropped; a kept draft that comes back unchanged is checked and offered again; an editor that returns at once keeps a changed draft |
| C2 — a save refused because the file changed could never succeed | §7.2 step 5 built: **Edit the new file** (the old draft kept and named on quit, a new one from the file as it is) / **Discard my draft** / Esc |
| C3 — "needs a re-check" named no unit | each feature carries its units to re-check (the F3 rule); its View lists them as links; rule 6 and the summary use the same list |
| C4, C11 — hand-edit words for the features draft; "invalid plan" | the draft's own words in every notice and on quit; the loader's own message; "at that line" only with a line |
| C5, C6 — negative claims from an incomplete, missing or out-of-date map | one `complete` flag (a current map, every scenario noted and agreeing): only then "run by none", "(n run this unit)", "no feature ran", "only this feature runs", "its functions are not in them"; else neutral words ("Fails on", "(map incomplete)", "not mapped") |
| C7 — the result was cut at 80 columns | the result right after the unit's id |
| C8, C13 — "not re-checked" for a C unit; grammar and counts | "still C"; per-state words ("2 need a re-check", "1 holds so far"); the real count of scenarios that cannot run; units outside the program counted apart |
| C9 — Cancel kept an untouched draft; the draft items not greyed | an untouched draft is dropped on Cancel; both items greyed while a command runs; Discard asks first |
| C10 — a crashed scenario "compares little" | "it did not exit (…) — it cannot be a check" first |
| C12 — Help and the tutorial | the file's shape, the editors' keys, the "·" legend and §6.1's reasons in Help; two tutorial sentences corrected |
| C14 — outside-the-program skips as "could not run"; an older verdict read with today's map | "not part of the program — nothing to do"; "(from an earlier features file)" on the chip and in the overlay |
| §8.4/§8.5 gaps | the Retry/Modify/Resume/Continue sentence, the Scan sentence, the Re-check dialog's unit line, the Accept dialog's features, `recommended` for Write/Edit/Map, **[Open nano]** |
| O1 — a dangling top-level `.c` link failed verify once features existed | a skip (`c-side-build-failed`) in the feature step; the shared program list unchanged |
| O2 — the program digest follows the facts' closure only | the `facts-stale` sentinel when a program file is unrecorded or changed since the scan (§2.4) |
| O3 — without the sandbox a candidate can rewrite the C binary | the C program's bytes checked before every C run (a change fails, never skips); the residual named for the whole-program check too (§4.1, SCHEMAS.md) |
| O4 — a plan-caused link failure explained to the model as a behaviour difference | its own explanation: the plan's doing, keep the translation |
| O5, M1 (high) — `source_dir = "."` made every unit outside and no `main()` | one lexical rule (`directly_in`) for the cockpit and the CLI; a current verdict decides inside or outside first |
| O6, M10 — program digests taken after the builds | taken before |
| O7 — a false "other C changed" after the features file was deleted | compared only when today has a program digest |
| O9 — run dir names changed length | fixed width |
| M2 — a leading `#pragma` or a naked function broke the probed build | unwatched; neutral build-failure words |
| M3 — a closed notes descriptor lost notes and set `errno` | reopen once; `errno` saved; an empty notes file is unavailable |
| M5 — `__DATE__`/`__TIME__` differed between separate compiles (verify too) | `SOURCE_DATE_EPOCH=0` for every tool child |
| M6 — `migration/` and `.git/` counted toward the mirror's limit | pruned inside the walk |
| M7 — an include outside `source_dir` resolved to a system header | refused before any build, named |
| M8 — the notes file appeared in the program's folder | the program's cwd is `run/` inside the temp dir |
| M9 — errors anywhere in a body, or a `#` in a comment, left a function unwatched | the head only; a line that starts with `#` |
| T1 — the program digest outside the read budget | counted by the preflight, each file once, at most 50 000 files |
| T2 — escape bytes from a hostile file on the terminal | unknown keys quoted; every human CLI line and error shown with control characters as `?` |

**The check of the fix pass** (two checkers on 71d9e1a) found M7 not fixed (the facts-based
check could never fire), O3 partial (with the whole-program check configured the C program's
bytes were noted after its candidate runs), O2 partial (headers reached with `<…>`), and new
issues — the features draft (P1: a busy ledger's `t` retry resent old text and could remove a
newer draft; N1–N3 of the draft: a failed "Edit the new file", Esc on the Discard question, the
draft's dir not named), the cockpit (N4: a unit's own line followed the plan's paths; N5: no
words for a map without notes), the digest (N2: `facts-stale` that no scan cleared; N3: two
sentinels equal in the replay), the probe (N1: a leading `_Pragma` or unreadable code, N4:
`naked` inside a name). All fixed in the second pass, each with a test; the rule text above
says the result. Mutation checks: 60 mutants of the named rules and the fixes, all killed.
