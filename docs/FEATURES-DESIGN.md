# Features: what a user does with the program, mapped to the code and checked — design

Status: DESIGN, revised after its adversarial review (2026-09-29): four lenses, 88 findings
(§R). The revision is not yet checked.

This implements the user's direction recorded 2026-09-23 ("feature workflows": user-facing
behaviours mapped to the code that implements them, so a migration preserves what a user
perceives, not only per-function correctness), in the scope the user chose on 2026-09-29
(DECISIONS.md "Feature-workflow view: §15 spike"): **scenarios + verify**.

Sources:
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
1. **Check** — in `verify` (and so in every judged turn of `migrate` and every promotion):
   every scenario runs on the all-C program and on the mixed program (all C minus the
   unit's `replaces`, plus the unit's Rust); one check per scenario,
   `feature:<feature>/<scenario>`, passes only when both exited normally with the same code
   and byte-identical streams.
2. **Map** — `harness features map`: every scenario runs on a copy of the original C that
   notes each function it runs (the **probe**), and the result — which of the scanner's
   functions each scenario ran — is written to `migration/features/map.json`. Functions map
   to files and to the plan's units at read time.
3. **Show** — the cockpit: a **Features** group in the tree; each feature's View shows its
   scenarios and what they did, the units its functions live in with their states, and its
   checks on each unit's verdict; each unit's View says which features run its code — or
   that none does, when the map can say so.

**Why both.** The spike found that zopfli's whole-program check runs `zopfli -c <sample>`
only — the gzip path — so a unit on the zlib path passes it without the check ever running
its code. Scenario checks make the person's behaviours part of every verdict; the map says
which of those checks actually exercise a unit, and which units no behaviour reaches.

**What stays true:**
- The oracle is the definition of done; the map never gates anything. A green verdict still
  means every check passed; features add checks, never waive one.
- The ledger is the truth; the cockpit never writes it; every write is a spawned
  `harness --json …` command, confirmed first. (Editing `features.toml` in the person's
  editor, §8.5, is the person writing it, as with any file.)
- Target-owned files are hostile input (SCHEMAS.md "Trust boundaries"). `features.toml` and
  `map.json` are target-owned: nothing in them reaches a model prompt except ids from a closed
  alphabet (§2.3), everything shown passes the display filter, and every run they define is
  confined like the whole-program check's — in an empty directory of its own (§4).
- A command that builds and runs target code refuses under `sandbox: none` unless
  `--allow-unsandboxed` is given (the rule every such command follows).
- No new crate. No new tool on the allowlist: the probe needs `cc` only.
- **Additive**: a target without `features.toml` behaves byte-for-byte as today — same
  checks, same verdict bytes, same attempt records, same events. The TRACTOR bench is
  unaffected (§10).

**Not in this design** (each has a "revisit when", §13):
- Features of a library without a `main` (all TRACTOR cases): a scenario runs a program.
- Input files of the person's own (`inputs/`), stdin, environment variables, and a file the
  program writes as the thing compared: v1 compares exit status and the two streams, with
  the three samples as inputs.
- A cumulative mixed build (every verified unit in Rust at once): each verdict swaps in its
  own unit only, as today — and the cockpit says so wherever it reports a feature.
- Line-level maps; features proposed from usage text or by a model; a read tool for the
  chat.

## 1. Where it lives

```
migration/features/
  features.toml        the person's features and scenarios (§2)
  map.json             which functions each scenario ran (written by `harness features map` only; §5)
migration/build/.features/   scratch of `features map` (gitignored; the leading dot cannot be a unit id)
```

Both files are committed. They are under `migration/`, so a bench case may hold them without
breaking its corpus lock. `map.json` is derived and deterministic on one platform (§5.2
records the platform); it is committed so the map shows on a cold start, like
`observer/findings.jsonl`.

Writer table additions (SCHEMAS.md):

| File | Writer |
|---|---|
| `migration/features/features.toml` | a person; `harness features init` creates a starter (never overwrites) |
| `migration/features/map.json` | `harness features map` |
| `migration/build/.features/**`, `migration/build/<unit>/fx/**` (gitignored) | `features map`; `verify` (scratch) |

## 2. `features.toml` (`ruharness-features`, v1)

### 2.1 Shape

Flat tables — a scenario names its feature, so a scenario placed anywhere in the file
belongs to the feature it says:

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

- `id` (feature and scenario): `^[a-z0-9][a-z0-9-]{0,23}$`; unique among features, and
  unique among one feature's scenarios. (`feature:` + 24 + `/` + 24 = 57 bytes: within the
  64 at which a repair prompt cuts check names.)
- `name`: the person's words, 1–60 characters, no control characters. Shown in the cockpit
  (display-filtered). **Never** in a check name, a verdict, an event, or a prompt. Help
  advises names of 24 characters or fewer (the tree's narrow width).
- `feature` (a scenario's): the id of a `[[feature]]` in the file.
- `args`: 0–8 entries, each 1–64 bytes, each one of:
  - `{input}` — the input's file name (§4.2); exactly once when `input` is set, never
    otherwise;
  - a flag: `^-{1,2}[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (`-c`, `--i5`, `--level=3`);
  - a word: `^[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (a sub-command or a value: `compress`, `9`,
    `nosuchfile`);
  - and none may contain `..`.
  No `/`: an argument cannot be an absolute path or reach a parent directory. A word or an
  `=` value can still name a file **relative to the run's directory** — which is the run's
  own fresh, empty temp dir (§4.1): nothing is there but the program and its input, and a
  file the program writes there is discarded. (That makes "a file that does not exist" an
  easy scenario to write.) The safety of the grammar rests on that directory and on the
  sandbox, not on the syntax.
- `input`: optional; `sample:text` (about 30 KB of English-like text), `sample:rand` (16 KiB
  of pseudo-random bytes) or `sample:empty` — the whole-program check's deterministic
  samples, the same bytes. The cockpit describes them in words, never by the token alone.
- Limits: ≤ 16 features; ≤ 8 scenarios per feature and ≤ 16 in all; the file ≤ 64 KiB; every
  feature has at least one scenario.
- **Strict**: an unknown key, a wrong type, a duplicate or unknown id, a limit passed is
  refused with the key's path and the rule, e.g. `features.toml: scenario "text" of feature
  "zlib": args[2] "a/b" is not allowed — an argument cannot contain "/"`. The file is typed
  by hand: a typo must fail loudly, not be passed over (§13.2 of the briefing). Consequence,
  stated in SCHEMAS.md: **every new key bumps `schema_version`** (an exception to the
  ledger's pass-over-unknown-fields rule), and a newer version is refused (`SchemaTooNew`).

### 2.2 Loading: `harness_core::features`

One loader, used by the oracle, the CLI and the cockpit:

`features::load(root) -> Result<Option<Features>, Error>` — `Ok(None)` when
`migration/features/features.toml` does not exist. It refuses a symlinked
`migration/features/`, reads through `ledger::read_regular` (no symlink, the opened handle
checked, bounded at 64 KiB), and validates §2.1.

Commands load it **at their start**, before anything else runs or is sent: `verify`,
`migrate`, `promote` and `features map` refuse with the loader's message when the file does
not validate (§6.4) — never after a paid model turn.

### 2.3 Why ids have a closed alphabet

Check names reach model prompts: a red verdict's failed checks are quoted to the repair turn
(`oracle_evidence` quotes `check.name`). The features file is target-owned; a hostile target
could write an instruction into a `name`. The check name is built from ids only
(`feature:zlib/text`), an alphabet that cannot spell a sentence or a prompt section header.
Details are the harness's own words (§6.2). No other feature text reaches a verdict, an event,
harness-mcp or the chat.

### 2.4 The two digests

Recorded in every verdict and attempt made while a features file exists, and in the map
(§5.2, §6.3):
- **`features`** — `blake3:` over a canonical rendering of what the scenarios run: for each
  feature by id, each scenario by id: its args and its input, the sample by its **bytes**.
  Names are not in it: rewording a feature changes nothing. The samples' bytes come from the
  in-memory generator (`samples()`), so computing the digest reads no file but
  `features.toml`.
- **`program`** — `blake3:` over what the whole program is built from: the facts' file set
  with each file's current hash (every scanned `.c`/`.h`), plus `[target] source_dir`,
  `include_dirs` and `[oracle] extra_link_args`. A change to any unit's C changes it.

Both are computed **once per read** — once per `verify`, once per `Snapshot::load`, once per
`state status` — never per unit, from hashes the reader already computes (the snapshot hashes
every facts file for its stale paths), and passed to whoever needs them. An invalid
`features.toml` yields the sentinel `invalid` for `features` (never an `Err` on a read path:
the cockpit, `state status` and harness-mcp keep working, and say the file has an error).

## 3. Which verdicts cover the features: a marker, not staleness

A verdict made before a scenario was added (or before another unit's C changed) did not run
the scenarios as they are now. That is **not** the same as the verdict being stale: every
check it recorded is still true of the unit's own inputs. Treating it as stale would flip
every verified unit to "needs attention", count none of them as migrated, and report a
CONTRADICTION for each (status.rs's rule) — on every edit of the file, while the person is
still writing it.

So:
- `VerdictInputs` gains `features: String` and `program: String`, both
  `#[serde(default, skip_serializing_if = "String::is_empty")]`, filled only when a features
  file exists. The `stale` list, `fresh_green`, the contradiction rule, `verified_in_place`,
  the bench's `fresh_green` and fence's `STALE_INPUTS` are **unchanged**.
- `UnitReport` gains `features: FeatureCoverage` — `None` (no features file, and the verdict
  records none), `Current`, or `Behind(reasons)` with reasons among `scenarios` (the verdict's
  `features` differs from now — including a verdict that has none while a file exists),
  `program` (its `program` differs) and `invalid` (the file does not validate). The `unit`
  event gains `features: "current" | [reasons]` (additive; absent without a features file).
- The cockpit shows it as a marker on the unit, not a state: **"your features: not checked
  since they changed — Re-check"** (or "…since other C changed"). The unit keeps its state
  (✓ migrated stays ✓). The feature states (§8.2) are where it counts.
- `AttemptRecord` gains the `features` digest (skipped when empty). The trajectory replay
  treats a mismatch as **superseded inputs**, not drift; a resumed attempt whose `features`
  digest differs from now is refused ("the features changed since this attempt started —
  start a new one"), so no attempt mixes turns judged under different scenario sets.
  Promotion re-judges, so binding (`--from`, promote) is unchanged.

## 4. Running a scenario

### 4.1 The confined scenario run

A new confined run, `Confinement::run_scenario`, used by `verify` and by `features map`. For
each run:
1. A fresh temp dir (as every confined run): the only place the run may write, removed after.
2. The built binary is **copied into it** as `<name>` — the target's `[target] name` when it
   matches `^[A-Za-z0-9][A-Za-z0-9_.-]{0,31}$`, else `program` — and the sample, when the
   scenario has one, is written into it under its fixed file name (`sample_text.txt`,
   `sample_rand.bin`, `sample_empty`).
3. The child runs with **cwd = the temp dir** and **argv[0] = `./<name>`**; `{input}` is the
   bare file name. So argv, every path the program prints about itself or its input, and the
   files it writes are identical on both sides and on every machine; the temp dir's own
   path is replaced by `$TMPDIR` in both streams, as for every confined run.
4. The run profile allows reading and executing only the copy, reading the input, writing
   only the temp dir — the whole-program check's profile, with the copy as the binary.
5. When the leader exits, its **process group is killed** before anything is read back (a
   forked child cannot outlive the run, race the read, or pile up across scenarios).
6. The outcome is reported as data: `Exited(code)`, `Signaled(n)`, `TimedOut`, `Overflow`
   (more output than the cap), `SpawnFailed`, with both streams (for the first two). This is
   a new method beside `built_status`, which the bench keeps as it is.

### 4.2 What each side must do

- **The C side must be usable**: it exits normally (any code), within the timeout, without
  overflowing — and gives the same result twice. A scenario whose C side is not usable is a
  mistake in `features.toml`, not evidence about any unit (§6.4).
- **The mixed side must match**: the same exit code and byte-identical stdout and stderr. A
  signal, a timeout or an overflow on the mixed side never passes — it is the candidate's
  crash, as everywhere else (the migrate brief's "a crash, a panic, or a timeout is a
  failure").

## 5. `harness features map`

### 5.1 The command

```
harness features map [--target DIR] [--allow-unsandboxed] [--json]
```
- Takes the writer lock (holder `features map`). It touches no unit, so there is no
  promotion recovery.
- Refuses, before building anything, with a message that says what to do, when: there is no
  features file; it does not validate; the facts are missing, or stale for any file ("scan
  again first" — the probe's ids come from the facts); a top-level `.c` in `source_dir` has no
  facts record; the facts record no public `main` in a top-level `.c` of `source_dir`
  ("features need a program with a main()"); `sandbox: none` without `--allow-unsandboxed`.
- Builds, in `migration/build/.features/` (recreated), with the allowlisted `cc` under the
  tool profile:
  - **`plain`** — the top-level `*.c` of `source_dir`, compiled exactly as the whole-program
    check compiles `whole_c`;
  - **`probed`** — the same, from a mirror (§5.3), with the probe runtime (§5.4).
  Messages say what is happening ("Building the C program…", "Building a copy that notes each
  function it runs…").
- For each scenario, in file order: `plain` twice, `probed` once, each a scenario run (§4.1).
- Writes `map.json` atomically once every scenario has run. An interrupted run (cancel,
  signal) writes nothing and says so; the old file stays.
- Exit 0 when `map.json` was written — even when some scenario needs a look (that is a
  finding recorded in the file); 1 on a refusal or an error.

### 5.2 `map.json` (`ruharness-features-map`, v1)

```json
{
  "schema": "ruharness-features-map",
  "schema_version": 1,
  "inputs": { "facts": "blake3:…", "features": "blake3:…", "program": "blake3:…",
              "platform": "macos-aarch64" },
  "unprobed": [["src/zopfli/x.c", "src/zopfli/x.c::odd"]],
  "scenarios": [
    {
      "feature": "zlib", "scenario": "text",
      "end": "exit 0",
      "stdout_bytes": 18234, "stderr_bytes": 0,
      "stderr_head": "",
      "stable": true,
      "probe_agrees": true,
      "slow": false,
      "footprint": "complete",
      "functions": [["src/zopfli/zlib_container.c", "ZopfliZlibCompress"], ["…", "…"]]
    }
  ]
}
```

- `inputs.facts`: the hash of the facts file's canonical bytes (a re-scan with a changed
  scanner changes it); `features`, `program`: the digests (§2.4); `platform`: OS and
  architecture — a map from another platform may reflect other `#if` branches, so it counts as
  out of date there. No toolchain strings: they are machine-specific churn and not a staleness
  input.
- `unprobed`: definitions the probe could not watch (§5.3), as `[file, name]`.
- Per scenario:
  - `end`: how the first plain run ended — `exit N`, `signal N`, `timed out`, `too much
    output`, `could not start`;
  - `stdout_bytes`, `stderr_bytes`: numbers only; `stderr_head`: the first line of stderr, at
    most 100 bytes, with every byte outside printable ASCII replaced by `?` at write time
    (the person's own program's words, shown only in the cockpit, display-filtered again);
  - `stable`: the two plain runs ended the same way with identical streams;
  - `probe_agrees`: the probed run ended as the first plain run did, with identical streams —
    when false, the map is of a program that behaved differently (it prints timings or
    addresses, reads its own binary);
  - `slow`: a plain run took longer than 10 s (every Re-check runs it three times);
  - `footprint`: `complete` (the probe file was read) or `unavailable` with `reason` (`none
    written` — the run hit no watched function; `unreadable` — not whole 4-byte records, an id
    out of range, or not a regular file);
  - `functions`: `[file, name]` pairs, sorted, each once; empty unless `complete`.
- Units are **not** stored: they are derived from today's plan at read time.
- **Current** iff all four inputs equal today's; otherwise **out of date**, with which input
  changed.
- **Hostile input**: the file is committed, so the loader (`features::load_map`) is strict —
  shape, the §2.1 id alphabet, `end` from its closed grammar, sizes (≤ 16 MiB, `read_regular`),
  every `[file, name]` present in today's facts (an unknown pair is counted, never shown). A
  forged file with correct digests is still "current": the View says the map is what the
  last `features map` recorded, and a person can always map again. It never gates anything.

### 5.3 The probed copy

- **The mirror**: every regular file under `source_dir` (the scanner's walk: contained, no
  symlinks followed, bounded at 20 000 files and 256 MiB) copied to
  `migration/build/.features/mirror/<repo-relative path>`; each file the facts record that has
  a watched function replaced by its probed version. The compile uses `-I` the mirror's
  `source_dir` and `include_dirs`, `-fmacro-prefix-map=<mirror>=<root>` (so `__FILE__` reads
  as in the original build), `-include <build>/probe.h`, `-ffp-contract=off` and
  `extra_link_args`. An include that leaves `source_dir` (`../x.h`) does not resolve in the
  mirror: the build fails and `features map` says so, naming the file — it never maps a
  different program silently.
- **The insertion** — `harness_scan::probe_source(rel_path, source, id_of)`, pure, built on the
  scanner's own `collect_functions`/`canonical_id` (not on mutate.rs, which skips preprocessor
  branches): `(__ruharness_probe)(N);` right after the opening `{` of each watched function's
  body, on the same line (parenthesised, so a function-like macro cannot capture it). Nothing
  else changes: line numbers, `__LINE__`, `__func__`. `N` is the index of the definition's
  `(file, name)` among the facts' distinct pairs (`#if` variants of one function share it; two
  functions of one name in two files do not).
- **Not watched** (listed in `unprobed`, never counted as "not run"): a definition under an
  ERROR or MISSING node; a body whose first byte is not a real `{`; a definition with a
  preprocessor directive between its declarator and its body (a brace inside `#if`, which
  compiles only on one branch). A definition the facts do not record gets nothing — the facts
  do not know it either.

### 5.4 The probe runtime (harness-owned C)

`fnprobe.h` (declaration only) and `fnprobe.c`, embedded with `include_str!` like the boundary
check's runtime, with the id count passed as `-DRUHARNESS_FNPROBE_N=<n>`:
- `seen[n]` (at least one byte); an id is recorded the first time it is hit.
- On the first hit the runtime opens `$TMPDIR/ruharness-fnprobe` with
  `O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC|O_NOFOLLOW`, moves the descriptor to 900 or above
  (`F_DUPFD_CLOEXEC`) and closes the original; each hit writes its id as 4 bytes (little
  endian). A write-at-first-hit survives a crash, `_exit` or a timeout.
- The run reads the file back with `Extras::collect` (a plain name, `lstat`, the same inode,
  capped at `4 × n × 64` bytes; a program with more than 64 forked processes reads back as
  `unreadable`).

**What a hostile target can do to its own map**: its C can write that file, redefine
`open`/`write`/`getenv`, change `TMPDIR` before the first probe, or close descriptor 900.
It can shape its map; it cannot shape a verdict — the map gates nothing (as design B's
boundary map, §B.9). Said in SCHEMAS.md's trust boundaries.

### 5.5 Events

Additive kinds (SCHEMAS.md "The events stream"):

| `k` | fields |
|---|---|
| `scenario` | `feature`, `scenario`, `n`, `of`, `end`, `stable`, `probe_agrees`, `footprint`, `functions` (the count) — one per scenario, after its runs |

Plus `message` lines and the `header`/`result` frame.

## 6. Scenario checks in `verify`

### 6.1 When and where

In `CAbiDifferential::verify`, **after every existing check** — after the sanitizers and the
opt-in boundary check (so a feature failure never suppresses the boundary check, which runs
only when every check before it passed) — when a features file exists:
- builds `whole_c` and `whole_mixed` as the whole-program check does, once (shared with it
  when both are configured; the shared "build the two programs" step is extracted from
  `whole_program`);
- for each scenario, in file order: runs the C side twice and the mixed side once (§4);
- emits one check per scenario, `feature:<feature>/<scenario>`.

Without a features file: nothing — no build, no check, no field, no event.

Every scenario runs for every unit, also a scenario whose map says it never runs the unit:
the map is advisory, and a verdict must not depend on it. The cockpit uses the map to say
which checks exercise the unit (§8.4).

Preconditions, checked when the features file is loaded (the command's start): the program
has a `main` (as §5.1), and the unit's `replaces` are among the program's top-level `.c` (the
whole-program check's rule, which the shared builds impose). Otherwise the command refuses,
naming the unit and the rule.

### 6.2 The detail

The harness's own words: lengths, offsets, exit codes — never the program's bytes (the
verdict is committed and may be quoted to a model). And never through `RunFailure::Failed`,
whose message embeds the argv and a stderr excerpt:
- pass: `exit 0; stdout 18234 bytes identical; stderr empty`;
- a mismatch: `exit 0 vs exit 1` / `stdout differs (lens 18234 vs 18230, first diff at byte
  9)` / `stderr differs (…)`, joined with `; `, followed by the scenario's argv as the program
  saw it — `(ran: ./zopfli --zlib -c sample_text.txt)` — built only from the grammar-checked
  args and fixed names, so the repair turn knows what was run;
- the mixed side did not exit: `candidate run failed: signal 6` / `…: timed out after 120s` /
  `…: more output than the cap` — the existing lead-in, which the migrate judge's `classify`
  already reads as a crash or timeout.

### 6.3 Verdict and attempt inputs

`compute_inputs` fills `VerdictInputs.features` and `.program` (§2.4, §3) from the loaded
file and the tree; `migrate` records the `features` digest in the attempt. `render_md` shows them.

### 6.4 An unusable scenario is an error, not a red verdict

When a scenario's C side is not usable (§4.2 — it crashed, timed out, overflowed, or its two
runs differ), `verify` **fails with an error**, writes no verdict and demotes nothing:
`feature scenario zlib/text cannot be a check: the C program's two runs differ (stdout lens …)
— change the scenario in migration/features/features.toml`. The same in a migrate turn: the
judge returns the error, as it does for a boundary check's C side (migrate.rs), and the run
stops — it never feeds a C-side problem to a model or spends the turn budget on it.

This makes one bad scenario stop every Re-check until the file is fixed — by design: the
person's features file, not a unit's Rust, is what is wrong, and the cockpit says so before
anything is armed (§8.6). `features map` finds the same problems first (`stable`, `end`) and
the feature's View names them.

## 7. `harness features init`

```
harness features init [--target DIR] [--json]
```
Takes the writer lock; refuses when `migration/features/features.toml` exists (it never
overwrites the person's file). Writes a commented starter that validates:
- a header comment: what a feature and a scenario are, the samples in words, the argument
  rule, "save, then press g in the cockpit";
- when `[oracle.whole_program]` is configured: one feature, `whole-program`, "What the
  whole-program check runs", with one scenario per sample using its args — so the person
  starts from what the harness already runs;
- otherwise: one feature, `run`, "Run it on a text file", with one scenario
  `args = ["{input}"]`, `input = "sample:text"`, commented as "a guess — change it".

## 8. The cockpit

### 8.1 The model (un-gated, pure): `harness_tui::featmap`

`FeatureMap::build(&Snapshot, &Files, &FeaturesState) -> FeatureMap`, computed on the load
worker (`load::read`), where `FeaturesState` is `None` / `Invalid(message)` /
`Valid { features, map: Option<(Map, Currency)> }`. `Snapshot::load` itself only computes the
two digests (§2.4) for the units' coverage (§3). It holds, per feature:
- its scenarios with the map's record of each (when current);
- its functions: the union of its scenarios' `functions`, each mapped to its file and that
  file's owning unit (`Files`' owner);
- the functions outside every unit (headers, files with no exported functions), counted;
- per unit of its functions: the unit's state, and this feature's checks on the unit's
  verdict — `passed`, `failed`, `absent` — and the verdict's coverage (§3);
- the verdicts that failed this feature's checks on units **outside** its functions (§6.1's
  case) — listed, never hidden.

And per unit: which features run its code; per function: which features run it; the project
totals: functions some feature ran / functions the probe watched.

### 8.2 Feature states

Glyph and word — the glyphs are the features' own, with their own section in Help's legend;
colour is never the only signal. "Has Rust" means the unit's status is verified or merged.
"Covering verdict" means the unit's verdict's coverage is `Current` (§3). First match wins:

| # | word | glyph | rule |
|---|---|---|---|
| F1 | failing | `✗` | a covering verdict of any unit has a failed `feature:<id>/…` check |
| F2 | fix its scenario | `⚠` | the current map says a scenario of it is not usable (`stable` false, or `end` not `exit N`) — verify will refuse it |
| F3 | needs a re-check | `⚠` | a unit that has Rust has no covering verdict, or one without this feature's checks |
| F4 | not mapped yet | `⋯` | no map, the map is out of date, or it lacks this feature |
| F5 | map incomplete | `⋯` | a scenario's footprint is unavailable or its probe disagreed — its View names the scenario |
| F6 | runs no unit's code | `∅` | its functions touch no unit |
| F7 | all its units migrated | `✓` | every unit its functions touch has Rust, and every unit with Rust passes this feature's checks on a covering verdict |
| F8 | holds so far | `◑` | some unit it touches has Rust (all passing, as F7); the word carries the count: "holds so far · 1 of 6 units" |
| F9 | all C | `◌` | no unit it touches has Rust yet |

F1 needs no map and comes first: a failure is shown even while the map is out of date. F3
considers every unit that has Rust, not only the feature's: a scenario can fail through a unit
its map never touched.

**Wording of F7 and F8, wherever they appear** (the View, Help, the summary): "each unit was
checked with only its own Rust swapped in — no build has them all in Rust together yet".

### 8.3 The tree

After `Units (n)`, always, a group **Features**:
- no file: `Features (none yet)`;
- an invalid file: `Features (error)` — selectable; its View shows the error;
- otherwise `Features (n)`, open by default, one row per feature in file order: glyph, name
  (display-filtered, cut with `…`; when two cut names would read the same, the id is shown
  after them), the state word when it fits.

New selections `Selection::Features` and `Selection::Feature(id)`; `parent()`,
`open_by_default`, `exists`, `surviving` and every exhaustive match as for `Units`/`Unit`. No
scenario rows (a feature has at most 8; its View lists them).

### 8.4 The Views

Every map-derived line depends on the map being current. When it is out of date, those lines
are shown under **"From the last map — out of date (the scan changed / your scenarios changed
/ the program's C changed / made on another platform)"**, and no negative claim ("runs no
unit", "no feature runs this unit") is made from it.

**Features (none yet)**: one sentence on what a feature is; the zlib example in words ("the
whole-program check runs only gzip; a 'zlib' feature would make every Re-check run zlib too");
"Press Enter: Start a features file"; for a program without `main()`: "Features need a program
with a main() — this target is a library; not supported yet" and no item.

**Features (error)**: the loader's message, display-filtered; "Press Enter: Edit the features
file"; "Re-checks and migrations are refused until it is fixed."

**Features (n)**:
- one line per feature: glyph, name, state word, "runs through k units", its checks as
  "k units pass · 1 fails · 2 not re-checked" — each line a link;
- the totals: "Your features ran 104 of 111 watched functions (6 could not be watched)."
  and, when the map is current, the count of functions no feature ran, with the first 20 as
  links to their function rows;
- "Edited features.toml? Press g to re-read." (there is no file watcher).

**A feature**:
- title: its name; under it its id and state in words, with the next step (F1: open the
  failing unit; F2: fix the scenario, naming it; F3: Re-check u-…, as links; F4: Map the
  features; F5: the scenario and why);
- **Scenarios**: per scenario its argv as the program sees it (`./zopfli --zlib -c
  sample_text.txt`, the sample described in words), and from the map: `exit 0 · stdout 18 234
  bytes · stderr empty` (or its first line), and a flag with a one-line reason when it is
  unstable, slow, its probe disagreed, its footprint is unavailable, or **it compares little**
  (it exited non-zero, or printed nothing to stdout: "its check compares only the exit status
  and an error message");
- **Where its code lives**: one row per unit its functions touch — the unit's glyph and word,
  the number of this feature's functions in it, and this feature's checks on its verdict
  (`✓ passed` / `✗ zlib/text` / `– not re-checked since the features changed` / `– no
  verdict`) — each a link; then "Outside every unit: n functions (headers, files with no
  exported functions)"; then, if any, **"Also fails on (its code is not in them)"**: units
  outside it whose verdict failed its checks;
- **Only this feature runs**: its specific functions (with ≥ 2 features), as links, or
  "everything it runs is also run by another feature";
- the F7/F8 sentence (§8.2) when either state applies.

**A unit** — one line in the unit header (cut with "+n — see Features"), when a features file
exists:
- map current and complete for every scenario: "Features that run its code: Compress to zlib
  ✓, Compress to gzip ✓" — or **"None of your features runs this unit's watched functions, so
  their checks pass whatever its Rust does."** — or, for a unit whose files are not among the
  program's top-level `.c`: "Not part of the program your features run.";
- otherwise: "Which features run it: not known — map the features";
- the coverage marker (§3) when its verdict is behind: "your features: not checked since they
  changed — Re-check".
The Re-check and Accept dialogs repeat the first line.

**A function**: its header gains "run by: Compress to gzip, Compress to zlib" or "run by none of
your features" (map current), or "not watched by the map" (`unprobed`).

**Checks** (the unit's checks strip and the verdict overlay): passing `feature:*` checks group
into one chip, "your features ×16 (4 run this unit)"; each failure is its own chip. In the
verdict overlay, a feature check whose feature does not run the unit says "(its feature does
not run this unit's code)". `check_words` stays stateless: `feature:zlib/text` → "feature
zlib/text".

**The project summary**: a line "Features: 5 — 1 failing, 2 hold so far, 2 all C · they ran 104
of 111 watched functions" (or "Features: none yet — see Features"), a link to the group; and a
Next-step rule after rule 4: features exist and the map is missing or out of date → "Map the
features — press Enter and choose Map the features".

### 8.5 The menu and the acts

| Where | Item | What it runs |
|---|---|---|
| Project; Features (none yet) | **Start a features file** | `harness --json features init --target <root>` — confirmed: "Writes migration/features/features.toml with a starter you then edit. Never overwrites." |
| Project; Features; a feature; Features (error) | **Edit the features file** | the person's editor on `migration/features/features.toml` — a small new flow that reuses `editor_command`/`editor_script` and the cockpit's suspend (the hand edit's copy-and-stage is for crate files only); refused when the file is a symlink or missing; the snapshot is re-read on return |
| Project; Features; a feature | **Map the features** | `harness --json features map --target <root>` (+ `--allow-unsandboxed` through `with_sandbox_flag`, as every act that runs code) — confirmed: "Builds the C program twice — once as it is, once with a note at the start of every function — and runs each of your 12 scenarios three times. Records which functions each ran in migration/features/map.json. Changes no verdict." |

"Map the features" is greyed, with the reason, and re-checked at confirm time like Refresh the
plan: busy; no features file; the file has an error; the facts are missing or stale ("Scan the
project first"); no `main()`. The project menu offers it too, so the Next step's act is where
`recommended` looks for it. On a feature in state F4, it is focused first.

**Gates the features file adds elsewhere**: while it does not validate, Re-check, Accept,
Retry, Modify and the chat's Migrate are greyed: "features.toml has an error — fix it first
(see Features)". The Re-check dialog gains: "It also runs your 12 feature scenarios on the
whole program." The Scan dialog gains: "A change in the C makes the features map out of date."

Progress: the `scenario` events narrate in the activity panel ("Mapped zlib/text — exit 0, 104
functions (3 of 12)"); the result line counts the scenarios that need a look ("Mapped 12
scenarios — 2 need a look").

### 8.6 Help

A **Features** section: what a feature and a scenario are; the file's shape (the §2.1
example); the samples in words; the argument rule (no `/`, runs in an empty folder); what the
states and glyphs mean; that changing a scenario makes each verdict "not checked since the
features changed" (renaming does not); that each unit is checked with only its own Rust; and
"edit the file, then press g". Help's legend gains the feature glyphs.

### 8.7 Reading the files in-process: the preflight

`preflight::check` gains: `migration/features` a real directory when present (not a
symlink); `features.toml` ≤ 64 KiB and `map.json` ≤ 16 MiB, each a regular file when present.
Nothing else is read (samples are generated in memory).

## 9. The chat and harness-mcp

The chat cannot read files (`--tools ""`; its tools are harness-mcp's reads) and harness-mcp
gains nothing in v1, so the chat sees `feature:` check names in a unit's verdict and nothing
else about features. Its brief gains: "A `feature:<id>/<scenario>` check runs one of the
person's scenarios on the whole program. A passing one says nothing about a unit its feature
does not run — the cockpit's Features view shows which do. You cannot see the features file;
describe a scenario in words (the flags and which sample), never as TOML."

## 10. Contracts and compatibility

- **No features file → today's bytes**: verify's checks, the verdict JSON, attempt records,
  `state status` output and events, the migrate prompts, the bench's replays and scores.
  Proved by the mini-target and zopfli verify tests (exact check lists, run on copies without
  the file) and by `bench check` after the core and oracle steps (§12).
- New schemas in SCHEMAS.md: `ruharness-features` v1 (strict; every new key bumps the
  version), `ruharness-features-map` v1; `VerdictInputs.features`/`.program`;
  `AttemptRecord.features`; `UnitReport.features` and the `unit` event's
  `features`; the `scenario` event; `features init`/`features map`, their flags and exit codes;
  the writer table; the trust boundaries (both files hostile; the map shaped by its own C).
- `feature:` checks join the check vocabulary. Readers keyed on names: the migrate judge (the
  mixed side's lead-ins; the unusable-scenario error), the cockpit's chips and overlay, the
  bench (generic).
- docs/TUTORIAL.md gains a "Features" section; README a line.

## 11. Code: where each piece goes

| Crate | Change |
|---|---|
| harness-core | `features`: types, strict loader, `load_map`, the digests, `samples()` (moved from the oracle); `facts::canonical_hash`; `VerdictInputs` and `AttemptRecord` fields; `UnitReport.features` (status takes the digests as a parameter); `Ledger` paths |
| harness-scan | `probe_source` (pure); `FnDef` gains the body's start; the unwatchable cases |
| harness-oracle | `Confinement::run_scenario` (copy, cwd, argv[0], group kill, status + streams); the shared whole builds; the scenario checks; `map_features` (mirror, builds, runs); the probe runtime; one shared confinement setup for verify, boundary and map |
| harness-llm | the judge's unusable-scenario error; the attempt's digests; the replay's superseded rule; the brief's sentences |
| harness-cli | `features init`, `features map`; features loaded at the start of verify/migrate/promote; the `unit` event's field |
| harness-tui | `featmap` (un-gated); preflight; `load::read`; tree, Views, chips, menu, the three acts, gates, narrate, Help |
| docs | SCHEMAS.md, TUTORIAL.md, README, this file |
| targets/zopfli | last: `migration/features/features.toml` (gzip, zlib, deflate, verbose, iterations, help, no file) and its `map.json`; u001 re-checked; the tests that load zopfli adjusted in the same commit |

## 12. Order of work and tests

Each step committed when green (`cargo fmt --check`, `clippy -D warnings`, `cargo test
--workspace`):

1. **Core**: the loader (every §2.1 rule, one test each, with its message), the digests
   (names excluded; sample bytes included; computed once), `load_map` (hostile shapes,
   unknown pairs), coverage in `UnitReport` (no file → `None` and today's bytes; an invalid
   file → `Behind(invalid)`, never an `Err`), the attempt's field; proptest round-trip of the
   map.
2. **Scenario checks** (closes the spike's gap by itself): `run_scenario` (argv[0], cwd, the
   copy, the group kill, every end), the shared builds, the checks after boundary, the
   details (no program bytes — a hostile program's stdout never appears), the unusable-
   scenario error and no demotion, the judge's error, the command-start load; on the mini
   target (a toy program with two behaviours, a usage line printing argv[0], a scenario that
   writes a file, one that exits 1 on both sides, one unstable, one that times out) and on a
   test-time copy of zopfli with features. Then `bench check --replay`.
3. **The map**: `probe_source` (lines unchanged, `#if` variants, a brace inside `#if`, ERROR
   nodes, K&R, two functions of one name, macro-defined functions), the mirror (a `.inc`
   file, `__FILE__`, `../` refused), the runtime (a crash keeps hits, a forged or torn file →
   unavailable, fork), `features map` (every refusal, the events, interrupted → no file),
   `features init` (never overwrites; the starter validates).
4. **The cockpit**, three commits: (a) `featmap` over fixtures (every F-state and its order,
   an incomplete map never yields F6 or "no feature runs this unit", outside-units failures,
   specific functions) + tree + the Features and feature Views (goldens); (b) the unit,
   function, chips, overlay and summary lines; (c) the acts, gates, Help, preflight, and a pty
   test: Map from the menu, the map appears.
5. **The chat brief**; the live chat tests (the brief changed).
6. **Dogfood**: zopfli's features file and map, u001 re-checked, the zopfli-loading tests
   adjusted in the same commit; the cockpit driven headless on it.

Then the adversarial code review (3–4 lenses), fix pass, its check, mutation checks of the
named rules (the id alphabet in check names; the argument grammar; the digests' exclusions;
the coverage marker never entering `stale`; `stable`, `probe_agrees`; the pass rule of §4.2;
F1–F9 order; the unusable-scenario error never writing a verdict), and the DECISIONS handoff.

## 13. Later, and decided separately

- **A cumulative mixed build** (every verified unit in Rust together, every scenario): the
  end-state guarantee a user cares about. Revisit when a second unit of one target is verified.
- **Library features** (no `main`): a person-written scenario program under
  `migration/features/programs/`. Revisit when a library target wants features.
- **Inputs of the person's own** (`inputs/`), stdin, environment, output files: revisit when a
  target's behaviour needs a specific input format, reads stdin, or writes the result to a file
  (zopfli without `-c` — the map already flags such a scenario as comparing little).
- **Folding `[oracle.whole_program]` into features** (its flags + the three samples are
  scenarios): revisit when a second target adopts features; then the flag grammar, its
  constant and its check go, with a migration note for recorded verdicts.
- **Scenario cost**: every Re-check runs every scenario three times. Revisit when scenario runs
  pass a quarter of a verify's time (cache the C side keyed by the program digest, the
  features digest and the toolchain; or run only scenarios the map says reach the unit, as a
  documented, non-default option).
- **Per-scenario coverage digests**, so adding a scenario leaves the others' verdicts covering:
  revisit when re-checking after an edit is the obstacle people hit; with it, a "Re-check every
  unit with Rust" act.
- **The limits** (16 features, 16 scenarios, 8 args): revisit when a real target hits one.
- **Line-level maps**: revisit when two features' function maps are equal but they behave
  differently.
- **Proposed features**: from `main`'s usage text (deterministic), or by a model in chat —
  proposals only, the person accepts by writing the file. Revisit when writing the file is the
  obstacle.
- **harness-mcp `harness_features`** (read, names fenced as untrusted): revisit with the chat's
  next change.
- **The scanner's gaps** the spike found (header `static inline` calls unresolved across files;
  function values without an edge; digraphs): separate tasks; the probe instruments
  definitions, not calls, so they do not affect the map.

## R. Design review — 88 findings (four lenses), verified, resolved

Four reviewers on the first draft (a618c34): safety & trust (SAF-1–15), engine & contracts
(ENG-1–19 + 5 nits), usability & honesty (USE-1–28), scope & simplicity (SCO-1–17). Two
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
