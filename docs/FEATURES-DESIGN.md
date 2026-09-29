# Features: what a user does with the program, mapped to the code and checked — design

Status: DESIGN, first draft (2026-09-29), not yet reviewed.

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
- docs/SCHEMAS.md (the ledger's contracts, the events stream, the trust boundaries).

## 0. What it is, and is not

**A feature** is something a person does with the program and can see the result of —
"compress a file to zlib", "print the help", "report a file that does not exist". The
person names it. It is defined by one or more **scenarios**: one run of the whole program
with fixed arguments and at most one fixed input file. What the person perceives of a run is
its **exit status, stdout and stderr**.

The harness does three things with features, all deterministic (Tier 0, no model):
1. **Observe** (`harness features observe`): run every scenario on a probed copy of the
   original C and record which of the scanner's functions each one executed — its
   **footprint**. Functions map to files and to the plan's units.
2. **Verify**: every `verify` of a unit (and every judged turn of `migrate`) runs every
   scenario on the all-C program and on the mixed program (all C, minus the unit's
   `replaces`, plus the unit's Rust), and compares exit status, stdout and stderr byte for
   byte. One check per scenario: `feature:<feature>/<scenario>`.
3. **Show** (the cockpit): a **Features** group in the tree; each feature's View shows its
   scenarios, the units its footprint runs through with their states, and this feature's
   check on each unit's verdict; each unit's View lists the features that run through it,
   or says that none does.

**Why both.** The spike found that zopfli's whole-program check runs `zopfli -c <sample>`
only — the gzip path. A unit on the zlib path passes that check without the check ever
running its code. The footprint says which scenarios exercise a unit; the scenario checks
make those runs part of the verdict. Together they answer "which of the things a user does
still behave the same with this unit in Rust, and which of them never touch it".

**What stays true:**
- The oracle is the definition of done; the footprint never gates anything. A green verdict
  still means every check passed; features add checks, never waive one.
- The ledger is the truth; the cockpit never writes it; every write is a spawned
  `harness --json …` command, confirmed first.
- Target-owned files are hostile input (SCHEMAS.md "Trust boundaries"). The features file
  is target-owned: nothing in it reaches a model prompt except ids from a closed alphabet
  (§2.3), and every run it defines is confined like the whole-program check's.
- No new crate. No new tool on the allowlist: the probe needs `cc` only.
- Additive contracts: a target without a features file behaves byte-for-byte as today —
  same checks, same verdict bytes, same events (the TRACTOR bench is unaffected; §9).

**Not in this design** (each has a "revisit when", §12):
- Features of a library without a `main` (all TRACTOR cases): a scenario runs a program.
- A scenario's output file (only exit status and the two streams are observed), stdin input,
  environment variables.
- A cumulative mixed build (every verified unit in Rust at once): each verdict swaps in its
  own unit only, as today.
- Line-level footprints; proposing features from usage text or by a model; editing features
  from the cockpit or the chat.

## 1. Where it lives

```
migration/features/
  features.toml        the person's features and scenarios (hand-written; §2)
  inputs/              the scenarios' input files (hand-placed; optional)
  observed.json        the footprints (written by `harness features observe` only; §4)
```

All three are committed. They are under `migration/`, so a bench case may hold them without
breaking its corpus lock (core `CASE_LOCAL_FILES` guards only the case root). `observed.json`
is derived, deterministic and small; it is committed so the map shows on a cold start (§2.2
of the briefing), like `observer/findings.jsonl`.

Writer table additions (SCHEMAS.md):

| File | Writer |
|---|---|
| `migration/features/features.toml`, `inputs/**` | a person |
| `migration/features/observed.json` | `harness features observe` |
| `migration/build/features/**` (gitignored) | `harness features observe` (scratch) |

## 2. `features.toml` (`ruharness-features`, v1)

### 2.1 Shape

```toml
schema_version = 1

[[feature]]
id = "gzip"
name = "Compress a file to gzip (the default)"
description = "What most people run: zopfli -c file > file.gz"

[[feature.scenario]]
id = "text"
args = ["-c", "{input}"]
input = "sample:text"

[[feature.scenario]]
id = "empty"
args = ["-c", "{input}"]
input = "sample:empty"

[[feature]]
id = "zlib"
name = "Compress a file to zlib"

[[feature.scenario]]
id = "text"
args = ["--zlib", "-c", "{input}"]
input = "sample:text"

[[feature]]
id = "help"
name = "Show the help"

[[feature.scenario]]
id = "flag"
args = ["-h"]
```

- `id` (feature and scenario): `^[a-z0-9][a-z0-9-]{0,31}$`, unique among features, and
  unique among a feature's scenarios.
- `name`: the person's words, 1–80 characters, no control characters. Shown in the cockpit
  (display-filtered) and in `harness features status`. **Never** in a check name, a verdict,
  an event's `check` line, or a prompt.
- `description`: optional, ≤ 500 characters, no control characters except `\n`. Shown only.
- `args`: 0–8 entries, each 1–64 bytes, each one of:
  - `{input}` — replaced by the input file's path; exactly once when `input` is set, never
    otherwise;
  - a flag: `^-{1,2}[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (`--i5`, `--level=3`, `-c`);
  - a word: `^[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` (a sub-command or a value: `compress`, `9`).
  No `/`, no leading `.`, no spaces: an argument cannot name a path; the only path a run is
  given is its input's, which the harness chooses.
- `input`: optional; one of
  - `sample:text`, `sample:rand`, `sample:empty` — the whole-program check's built-in
    deterministic samples (the same bytes, `write_samples`);
  - `inputs/<path>` — a file under `migration/features/inputs/`: clean relative segments
    (`plan::is_clean_segment` each), a regular file (not a symlink, no symlinked parent
    inside `migration/features/`), ≤ 4 MiB.
- Limits: ≤ 16 features, ≤ 8 scenarios per feature, ≤ 32 scenarios in all; the file
  ≤ 64 KiB; the inputs it names ≤ 16 MiB together.
- **Strict**: an unknown key, a wrong type, a duplicate id, a limit passed is refused with
  the key's path and the rule (`features.toml: feature "zlib" scenario "text": args[2]
  "./x" is not allowed — an argument cannot be a path; use input = "inputs/…"`). Unlike the
  ledger's machine-written files, this one is typed by hand: a typo must fail loudly, not be
  passed over (§13.2 of the briefing: "validated with helpful errors").
- A newer `schema_version` is refused (`SchemaTooNew`), as for every ledger file.

### 2.2 Loading: `harness_core::features`

One loader, used by the oracle, the CLI and the cockpit:

`features::load(root) -> Result<Option<Features>, Error>` — `Ok(None)` when
`migration/features/features.toml` does not exist (the directory may exist without it).
It reads through `ledger::read_regular` (no symlink, bounded, the opened handle checked) and
refuses a symlinked `migration/features/` directory. It validates everything in §2.1,
including that every `inputs/…` file exists, is regular and within the caps (`lstat`, then
the size — the bytes are read only by the digest, §2.4, and by the oracle).

### 2.3 Why ids have a closed alphabet

Check names reach model prompts: a red verdict's failed checks are quoted to the repair turn
(`oracle_evidence` quotes `check.name`). A person's `name` could carry anything — the
features file is target-owned, and a hostile target could write an instruction there. The
check name is built from ids only (`feature:zlib/text`), whose alphabet cannot spell a
prompt section header or a sentence with spaces. The detail is the harness's own wording
(§5.2).

### 2.4 The digest

`features::digest(root, &Features) -> String` — `blake3:` over a canonical rendering of
what the scenarios RUN: for each feature by id, each scenario by id: the args, and the
input's kind plus the input's content hash (`sample:*` by name; `inputs/…` by
`hash::file_hash`). Names and descriptions are not in it: rewording a feature does not
invalidate a verdict. The empty string when there is no features file.

## 3. The probe

### 3.1 The probed copy (harness-scan, pure)

`harness_scan::probe_source(rel_path, source, id_of) -> Result<Vec<u8>, Error>` — parse the
file with the scanner's own grammar and its own function collection (`collect_functions`,
`canonical_id`), and insert `__ruharness_probe(N);` immediately after the opening `{` of
each function body, on the same line, where `N = id_of(canonical id)`. Nothing else in the
file changes: line numbers, `__LINE__`, `__func__` and every byte outside the insertions are
the original's. A function the scanner does not record (a definition inside a macro, a
declarator it cannot parse) gets no probe — the facts do not know it either.

`id_of` maps the facts' canonical symbol names (deduplicated: `#if` variants of one function
share its id) to 0-based indices in facts order. A definition whose canonical id is not in
the facts is an error ("the facts are stale — scan again"), never a guessed id.

### 3.2 The runtime (harness-owned C)

Compiled into the probed program only; its declaration reaches every translation unit by
`-include <build>/features/probe.h` (clang and gcc both take `-include`), so no source line
moves:

```c
/* probe.h */
void __ruharness_probe(unsigned id);
/* probe.c (N = the number of ids, rendered in) */
static unsigned char seen[N];
static int fd = -2;
void __ruharness_probe(unsigned id) {
  if (id >= N || seen[id]) return;
  seen[id] = 1;
  if (fd == -2) { /* open once: $TMPDIR/ruharness-probe, O_WRONLY|O_CREAT|O_APPEND, 0600 */ }
  if (fd >= 0) { unsigned char b[4] = { id, id >> 8, id >> 16, id >> 24 }; (void)write(fd, b, 4); }
}
```

- A hit is written the first time it happens, so a run that crashes, exits through `_exit`
  or times out keeps what it did before.
- The file lives in the run's own temp dir (the confinement's `TMPDIR`, the only place a
  run may write) and is read back by `Extras::collect` after the child is gone, capped at
  `4 × N × 64` bytes (a forked child writes its own hits; duplicates are merged).
- A record that is not a multiple of 4 bytes, or names an id ≥ N, makes the footprint
  **unreadable** for that scenario (§4.2) — never a partial guess.

### 3.3 The builds

`harness features observe` (§4) builds, in `migration/build/features/` (recreated):
- `mirror/<repo-relative path>` — every file the facts record, copied; each `.c`/`.h` with
  at least one recorded function replaced by its probed version (§3.1);
- `program` — the probed program: the mirror's copies of the top-level `*.c` in
  `source_dir` (the whole-program check's set, non-recursive), plus `probe.c`, compiled with
  `-include probe.h`, `-I` the mirror's `source_dir` and `include_dirs` first and then the
  originals (a header the facts do not record resolves to the original), the oracle's
  `-ffp-contract=off`, `extra_link_args`;
- `plain` — the same sources unprobed, compiled exactly as the whole-program check compiles
  `whole_c`.

Every compile is the allowlisted `cc` under the tool profile, every run a confined run
(fresh `TMPDIR`, the run profile: reads only the binary and the scenario's input, writes
only its temp dir, execs nothing but itself), as for every oracle run.

## 4. `harness features observe`

### 4.1 The command

```
harness features observe [--target DIR] [--json]
```
- Takes the writer lock (`features observe` as holder), and recovers an interrupted
  promotion first, as every writing command does.
- Refuses, before building anything, when: there is no features file; it does not
  validate; the facts are missing or stale for any file (a file whose hash differs from its
  facts record — the ids would not match; "scan again first"); a top-level `.c` in
  `source_dir` has no facts record.
- Builds `program` and `plain` (§3.3). A build failure is an error that names the step
  (it is the person's program or the harness, never evidence of anything).
- For each scenario, in file order: runs `plain` twice and `program` once, all confined, with
  the scenario's argv (`{input}` replaced by the input's canonical path — a sample written
  into `migration/build/features/`, or the file under `inputs/`, the one path the run may
  read).
- Writes `observed.json` atomically (`write_atomic`) once every scenario has run. An
  interrupted run (cancel, signal) writes nothing and says so; the old file stays.
- Prints one line per scenario and a summary; with `--json`, the events of §4.3.
- Exit 0 when `observed.json` was written, even if some scenario's run was not clean
  (that is a finding recorded in it, §4.2); 1 on a refusal or an error.

### 4.2 `observed.json` (`ruharness-features-observed`, v1)

```json
{
  "schema": "ruharness-features-observed",
  "schema_version": 1,
  "inputs": { "facts": "blake3:…", "features": "blake3:…", "toolchain": ["cc: Apple clang 21.0.0 …"] },
  "scenarios": [
    {
      "feature": "zlib", "scenario": "text",
      "run": "exit 0",
      "stable": true,
      "probe_agrees": true,
      "footprint": "complete",
      "functions": ["ZopfliCompress", "ZopfliZlibCompress", "src/zopfli/zlib_container.c::adler32", "…"]
    }
  ]
}
```

- `inputs.facts`: the facts records hash (`facts_records_hash`, moved from harness-cli into
  `harness_core::facts` — COCKPIT-WRAPPER-DESIGN §15 already planned the move);
  `inputs.features`: the digest (§2.4); `inputs.toolchain`: `cc --version`'s first line and
  the sandbox mode, as verdicts record them (informational; not a staleness input).
- `run`: how the plain run ended, in the harness's words: `exit N`, `killed by a signal`,
  `timed out after Ns`, `output over the cap`.
- `stable`: the two plain runs agree (exit status, stdout, stderr). An unstable scenario
  cannot be a check (it would fail at random); `verify` still runs it (§5.3), and the View
  says why it fails.
- `probe_agrees`: the probed run's exit status and streams equal the first plain run's. When
  false, the footprint is of a program that behaved differently (a program that prints
  addresses or timings, or reads its own binary) and the View says so.
- `footprint`: `complete` (the collected file was read), `missing` (the run left no file: it
  hit no probed function — possible for a program that fails before `main`'s body — or it
  changed its `TMPDIR`), `unreadable` (§3.2). `functions` is empty unless `complete`.
- `functions`: canonical ids, sorted, each at most once.
- Units and files are NOT stored: they are derived at read time from the plan and the facts
  (a re-plan must not make the file wrong). Staleness: the file is **current** iff
  `inputs.facts` and `inputs.features` equal today's; otherwise **out of date** (the View
  says which changed).
- The loader (`features::load_observed`) is strict about shape, lenient about unknown fields
  (it is machine-written: open-schema rules), bounded at 16 MiB, `read_regular`.

### 4.3 Events

Additive kinds (SCHEMAS.md "The events stream"; consumers skip unknown kinds):

| `k` | fields |
|---|---|
| `scenario` | `feature`, `scenario`, `run`, `stable`, `probe_agrees`, `footprint`, `functions` (the count) — one per scenario, after its runs |
| `observed` | `path`, `scenarios` (the count) — after `observed.json` was written |

Plus `message` lines as for every command, and the `header`/`result` frame.

## 5. Scenario checks in `verify`

### 5.1 When they run

In `CAbiDifferential::verify`, right after the whole-program checks (step 6) and before the
sanitizers: when `features::load` returns features, the oracle builds `whole_c` and
`whole_mixed` exactly as the whole-program check does (once — shared with it when both are
configured), then for each scenario, in file order, runs it confined on both, and emits
`feature:<feature>/<scenario>`. Without a features file nothing changes: no build, no check,
no field (§9).

A features file that does not validate is an `Error::InvalidPlan` before any subprocess
(like a bad `[oracle.whole_program]`), so `verify` never runs half a feature set.

Every scenario runs for every unit — also the scenarios whose footprint does not touch the
unit. They are cheap, and a run that the footprint says cannot reach the unit is exactly the
case to prove, not assume (a Rust crate that changes a global a scenario reads would show
here). The View uses the footprint to say which checks *exercise* the unit (§7.4).

### 5.2 The comparison and the detail

Each side is run with `Runner::built_status`-style reporting (the bench's form: how the run
ended is data, not a failure), under the confinement's `run_with`. The check passes iff both
sides ended the same way — both exited with the same code, or both were killed by the same
kind of end — and both streams are byte-identical. A timeout on either side never passes.

Details are the harness's own words, lengths and offsets only — never the program's bytes
(the program is the person's; the verdict is committed and may be quoted to a model):
- pass: `exit 0; stdout 18234 bytes identical; stderr empty`;
- fail: `exit 0 vs exit 1` / `stdout differs (lens 18234 vs 18230, first diff at byte 9)` /
  `stderr differs (…)`, joined with `; `;
- timeouts and kills: `C-side run failed: timed out after 120s` /
  `candidate run failed: killed by a signal` — the existing lead-ins, so the migrate judge's
  `is_c_side` and `classify` read them exactly as they read the whole-program check's.

### 5.3 An unstable C side

A mismatch triggers one more run of `whole_c`. If it differs from the first C run, the check
fails with `C-side run failed: the C program's output is not stable across runs (exit …;
stdout …)` — a C-side failure (never candidate evidence; `verify` still demotes a verified
unit, as for every C-side failure; the View says "this scenario cannot be a check; change
it"). `harness features observe` reports the same instability up front (`stable: false`).

### 5.4 Verdict inputs and staleness

`VerdictInputs` gains `features: String` — the digest (§2.4) — with
`#[serde(default, skip_serializing_if = "String::is_empty")]`: without a features file the
verdict's bytes are today's. `status::compute` adds `features` to a verdict's `stale` list
when the digest now differs from the recorded one (both empty = fresh). So:
- adding, removing or changing a scenario makes every existing verdict stale → the cockpit
  shows "Re-check" (state 7) — the verdict does not cover the scenarios as they are now;
- rewording a feature's `name` changes nothing.

`compute_inputs` fills the field; the `unit` event's `verdict.stale` gains the value
`features`. Promotion's and `migrate`'s staleness rules read `stale` generically — each
place that lists the values is updated (§10 lists them).

## 6. `harness features status`

```
harness features status [--target DIR] [--json]
```
Read-only (no lock). Prints the features, whether `observed.json` is current, and per
feature: its scenarios with their runs, the units its footprint runs through (from today's
plan), and this feature's checks on each unit's latest verdict. With `--json`: one
`feature` event per feature — `id`, `scenarios [{id, run, stable, probe_agrees,
footprint}]`, `units [{id, functions, checks: [{name, passed}] }]`, `outside_units`
(function count) — and one `features` event (`observed`: `current | out-of-date | none`,
`functions_run`, `functions_total`). The `name` is included verbatim (§2.1 bounds it);
consumers display-filter it. This is also what harness-mcp may expose later (§12).

## 7. The cockpit

### 7.1 The model (un-gated, pure): `harness_tui::featmap`

`FeatureMap::build(&Snapshot, &Files, Option<&Features>, Option<&Observed>) -> FeatureMap`,
computed on the load worker (`load::read`, next to `walk_tree`), never in `Snapshot::load`
(harness-mcp's reads do not pay for it). It holds, per feature:
- its scenarios and their observed runs;
- its footprint: the union of its scenarios' `functions`, each mapped to its defining file
  (facts) and that file's owning unit (`Files`' owner: the non-blocked unit whose `files`
  hold it);
- the functions outside every unit (headers, static-only files), counted and listed;
- **specific** functions: run by this feature only (with N ≥ 2 features);
- per unit of its footprint: the unit's state (`files::UnitState`) and this feature's checks
  on the unit's current verdict: `passed`, `failed`, `absent` (the verdict predates the
  feature or does not exist), and whether that verdict is stale.

And per unit: the features whose footprint runs through it, and per function: the features
that run it. And the project totals: functions run by some feature / all functions.

### 7.2 Feature states

Glyph and word, first match wins (colour never the only signal):

| # | word | glyph | rule |
|---|---|---|---|
| F1 | not observed | `?` | no `observed.json`, or it is out of date, or this feature's scenarios are not in it |
| F2 | failing | `✗` | some unit's current verdict has a failed `feature:<id>/…` check |
| F3 | needs a re-check | `⚠` | a unit of its footprint whose verdict is stale, or lacks one of this feature's checks, while the unit has Rust (status verified/merged, or an attempt promoted) |
| F4 | checked in Rust | `✓` | at least one unit of its footprint has Rust, and every such unit's current verdict passes every one of this feature's checks |
| F5 | all C | `○` | no unit of its footprint has Rust yet |
| F6 | runs no unit's code | `–` | its footprint touches no unit (only headers or static-only files, or nothing) |

"Checked in Rust" is worded carefully in the View: each unit is swapped in **on its own**
by its verdict; there is no build with every migrated unit in Rust at once (§12).

### 7.3 The tree

After `Units (n)`: a group **Features (n)**, present when the features file exists (or does
not validate — then one note row, "features.toml has an error — see the View"). Under it,
one row per feature, in file order: its glyph, its `name` (display-filtered, cut with `…`),
its state word when it fits. New selections: `Selection::Features` and
`Selection::Feature(id)`; `parent()`, `open_by_default` (the group opens by default),
`exists`, `surviving` and the exhaustive matches as for `Units`/`Unit`.

No scenario rows: scenarios are listed in the feature's View (a feature has at most 8).

### 7.4 The Views

**Features (the group):**
- one line per feature: glyph, name, state word, "runs through k units", its checks
  summary (`✓ 3 · ✗ 1 · – 2 not yet run`), each a link;
- the totals: "Your features run 104 of 111 functions. 7 functions no feature runs:" and
  the list (up to 20, then "…"), each function a link to its file row;
- when `observed.json` is out of date or missing: "The map is out of date (the scan
  changed | features.toml changed) — choose Observe the features." — a fact, like the
  project summary's Next step.
- when the file does not validate: the loader's error, display-filtered, and "Fix
  migration/features/features.toml, then press g."

**A feature:**
- title: the name; under it the id and the description;
- **Scenarios**: per scenario its argv as the program sees it (`zopfli --zlib -c
  <sample:text>`), its observed run, and a flag when it is unstable, when the probed run
  disagreed, or when the footprint is missing/unreadable — each with a one-line reason;
- **Where it runs**: one row per unit of its footprint — the unit's glyph and word, the
  number of this feature's functions in it, and this feature's checks on its verdict (`✓` all
  passed / `✗ zlib/text` / `– not run yet` / `⚠ stale`) — each a link to the unit; then
  "Outside every unit: n functions (headers, static-only files)";
- **Only this feature**: its specific functions (links), or "every function it runs is
  also run by another feature";
- the state's next step in words (F1: Observe the features; F3: Re-check u-…; F2: open the
  failing unit's verdict).

**A unit (added to today's unit screen, under the checks):** "Features that run this unit:
Compress to zlib ✓, Compress to gzip ✓" — each with this feature's checks on this verdict —
or, when features exist and the map is current, **"No feature runs this unit's code. Its
whole-program and feature checks pass without calling it."** (worded as a fact; this is the
spike's zlib finding, shown where it matters).

**A function:** its header line gains "run by: gzip, zlib" or "run by no feature" when the
map is current.

**The project summary** gains one line: "Features: 5 (✓ 2, ✗ 1, ○ 2) · they run 104 of
111 functions", a link to the group; and a Next-step rule after rule 4: features exist and
the map is out of date or missing → "The features map is out of date — Observe the
features".

**Check words** (`narrate::check_words`): `feature:<id>/<scenario>` → `feature "<name>":
<scenario>` when the feature is known, else `feature <id>/<scenario>`. The raw name stays in
the details.

### 7.5 The menu and the act

`Enter` on the Features group or on a feature offers **Observe the features** — a new act,
`Act::ObserveFeatures`, argv `harness --json features observe --target <root>`, confirmed in
the armed dialog like every act ("Run every feature's scenarios on a probed copy of the C
program and record which functions each one runs. Writes migration/features/observed.json.
Takes about as long as the scenarios do, twice over."), greyed while busy, cancellable
(SIGINT to the group, as every act). Its `scenario` events narrate in the activity panel
("Observed zlib/text — exit 0, 104 functions"). A feature's View links to its units; a unit's
Re-check is the existing act.

No act writes `features.toml`: the person edits it. Help gains a **Features** section: what a
feature is, the file's shape (the §2.1 example), where inputs go, what the states mean, and
"the chat can read the file and help you write it; you save it".

### 7.6 Reading the files in-process: the preflight

The cockpit reads `features.toml` and `observed.json` in-process, so
`preflight::check` gains them (the contract both the cockpit and harness-mcp run before a
read): `migration/features` a real directory when present (not a symlink);
`features.toml` ≤ 64 KiB and `observed.json` ≤ 16 MiB, each a regular file when present;
the `inputs/` files are never read by the cockpit (the digest the cockpit compares is the
recorded one against a recomputation — see below), so they are checked only by the loader's
`lstat`s and caps.

Staleness in the cockpit: `observed.inputs.facts` against the snapshot's facts hash (already
in hand), and `observed.inputs.features` against `features::digest` — which hashes the input
files (≤ 16 MiB in all, §2.1). It runs on the load worker, only when a features file exists.

## 8. Chat and harness-mcp

Nothing changes in v1. The chat can `Read` the features file and the observed map like any
ledger file; its brief gains two sentences: what features are, and that it may suggest
scenarios for the person to write but never claims a feature is preserved except from a
verdict's `feature:` checks. harness-mcp gains nothing (a `harness_features` read tool is
§12).

## 9. Contracts and compatibility

- **No features file → today's bytes.** `verify`'s checks, the verdict JSON (the
  `features` field is skipped when empty), `state status`'s events, the migrate prompts
  (built from the verdict), the bench's replays and scores: all unchanged. Tested by the
  existing zopfli/mini-target verify tests (exact check lists) and proved by `bench check`
  (§11).
- `feature:` checks join the check-name vocabulary; readers keyed on names: the migrate
  judge (`is_c_side`, `classify`, `oracle_evidence` — no change needed: the lead-ins carry
  it; a test proves each path), the cockpit's `check_words` and its checks panel grouping
  (grouped like `whole-program:*`), the bench (reports checks generically).
- New schemas documented in SCHEMAS.md: `ruharness-features` v1, `ruharness-features-
  observed` v1; `VerdictInputs.features`; the `stale` value `features`; the events `scenario`,
  `observed`, `feature`, `features`; the CLI subcommands `features observe` and `features
  status` and their exit codes.
- docs/TUTORIAL.md gains a short "Features" section.

## 10. Code: where each piece goes

| Crate | Change |
|---|---|
| harness-core | `features` module: types, strict loader, `digest`, `load_observed`, the observed types; `facts::records_hash` (moved from harness-cli); `VerdictInputs.features`; `status::compute`'s `features` staleness; `Ledger` paths |
| harness-scan | `probe_source` (pure; shares `collect_functions`/`canonical_id`) |
| harness-oracle | the scenario checks in `verify` (shared whole builds); `observe_features` (mirror, builds, runs, collect); the status-reporting confined run |
| harness-cli | `features observe`, `features status`; events |
| harness-tui | `featmap` (un-gated); preflight; `load::read`; tree, view, menu, act, narrate, Help |
| docs | SCHEMAS.md, TUTORIAL.md, README, this file |
| targets/zopfli | `migration/features/features.toml` (the dogfood: gzip, zlib, deflate, verbose, iterations, help, a missing file) and its `observed.json`; u001's verdict re-checked |

## 11. Tests and the order of work

Each step committed when green (`cargo fmt --check`, `clippy -D warnings`, `cargo test
--workspace`):

1. **Core**: the loader (every §2.1 rule, one test each, with the message), the digest
   (names excluded; inputs' bytes included), `load_observed`, the staleness value; proptest
   round-trip of the observed file.
2. **Scan**: `probe_source` — line numbers unchanged (every line count equal), only the
   insertions differ, `#if` variants share an id, a function unknown to the facts is an
   error, K&R and attribute-laden definitions; zopfli's whole tree probed compiles.
3. **Oracle**: scenario checks on the mini target (a toy program with two behaviours); the
   comparison rules (exit codes, streams, timeouts, the unstable C side); no features file →
   the exact check list of today (existing tests); `observe_features` end to end on the
   mini target; the probe's crash-survival (a scenario that aborts after some calls); a
   forged or malformed probe file → `unreadable`.
4. **CLI**: `features observe` refusals (no file, invalid, stale facts), the events,
   interrupted → no file written; `features status --json`.
5. **Cockpit**: `featmap` over fixtures (every state F1–F6, specific functions, outside
   units, reverse maps); the tree rows; the Views (goldens); the act's argv; narrate; Help;
   the preflight additions; a pty test that Observe runs from the menu and the map appears.
6. **Dogfood**: zopfli's features.toml; `harness features observe`; re-verify u001
   (the verdict gains its feature checks — all pass, since u001 is katajainen, run by every
   compressing scenario); the cockpit shows u-zlib_container run by the zlib feature only.
7. **Bench**: `bench check --replay` — `198 reproduce …, 0 problem(s)`, `no regression`.

Then: an adversarial code review from 3–4 lenses, findings verified; fix pass; its check;
mutation checks of the named rules (the id alphabet in check names, the arg grammar, the
digest's exclusions, the stale rule, `stable`, `probe_agrees`, the pass rule of §5.2, F1–F6
order); DECISIONS handoff.

## 12. Later, and decided separately

- **A cumulative mixed build** (every verified unit in Rust together, all scenarios): the
  end-state guarantee a user cares about. Revisit when a second unit of one target is
  verified.
- **Library features** (no `main`): a person-written scenario program under
  `migration/features/programs/` linked against the library. Revisit when a library target
  wants features (TRACTOR cases don't: one unit each).
- **Output files, stdin, environment**: revisit when a target's behaviour is a file it
  writes (zopfli without `-c`), reads stdin, or depends on the environment.
- **Line-level footprints** (which lines of a shared function a feature runs): revisit when
  two features' function footprints are equal but their behaviours differ.
- **Proposed features**: from `main`'s usage text (deterministic), or by a model in chat
  (Tier 2) — proposals only, the person accepts by writing the file. Revisit when writing
  the file is the obstacle people hit.
- **harness-mcp `harness_features`** (read): revisit with the chat's next change.
- **The scanner's gaps** found by the spike (header `static inline` calls unresolved across
  files; function values without an edge; digraphs): separate tasks; they do not affect the
  probe, which instruments definitions, not calls.
