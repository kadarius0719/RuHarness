# harness-tui — the review cockpit

Status: DESIGN, REVIEWED (2026-09-24; §R holds the resolutions of the 20 confirmed findings of
the adversarial review — the text below is the post-review design). Sources: the §15 spike
(DECISIONS.md "TUI track: §15 research spike"), docs/CLI-HARDENING.md (the CLI this cockpit
drives), docs/SCHEMAS.md (the ledger it reads), docs/REPLAY-DESIGN.md (the replay rules the
two CLI additions must keep).

## 0. What it is, and is not

A thin, read-mostly terminal cockpit for reviewing migrations: units and attempts on the
left, the C function beside its Rust translation in the middle, the oracle's verdict on top,
the running command at the bottom. Three acts: **Accept** (promote a green attempt),
**Modify** (a steer note that becomes a new oracle-judged attempt seeded from the one being
reviewed), and a **labelled hand edit** (a human attempt, judged like any other and never
counted as the pipeline's). Everything else is reading.

Not a chat client (chat lives in Claude Code through `harness-mcp`, a later milestone that
reuses this crate's read model as a library), not an editor (a hand edit happens in
`$EDITOR` and comes back through `harness override`), not a second writer: **every write is
a spawned `harness --json` command**; the TUI never touches the ledger and never takes the
writer lock; the ledger is re-read — it is the truth — when a spawned command has been
reaped.

## 1. Crate, features and dependencies

`crates/harness-tui` = a **library** (`harness_tui::model`: the snapshot, the pair locator,
the display filter — depends on harness-core, tree-sitter, tree-sitter-rust only) **plus a
binary** `harness-tui` behind the default feature `tui` (`required-features = ["tui"]`;
ratatui, crossterm, similar, tree-sitter-highlight, signal-hook are optional and enabled by
`tui`). `harness-mcp` will depend on it with `default-features = false` — the "feature
gated" of the brief. The workspace root gains `default-members` = every crate except
harness-tui, so a plain `cargo build` at the root stays lean; `cargo test --workspace` and
CI still build it. `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`. Workspace
`rust-version` 1.89 → 1.90 (tree-sitter-highlight 0.27 declares 1.90; ratatui 0.30, 1.88).

| crate | why | cost (measured in the spike's scratch project) |
|---|---|---|
| ratatui 0.30 (`default-features = false`, `crossterm`) + crossterm 0.29 | the terminal (`tui` only) | 61 unique |
| similar 3 | line diff between two Rust sides (`d`) (`tui` only) | +1 |
| tree-sitter-highlight 0.27 | highlighting (`tui` only) | +core already in the tree |
| tree-sitter-rust 0.24.2 | locating shims and logic fns by name (lib) | +1 (`tree-sitter-language`), same tree-sitter 0.27 core; ships `HIGHLIGHTS_QUERY` |
| signal-hook 0.4 | the TUI's own signal path (`tui` only) | already in the lock |

C highlighting uses the workspace's tree-sitter-c 0.24 (its `HIGHLIGHT_QUERY`). Rejected
(spike): syntect (+31), editor widgets, cursive / tui-realm / termwiz. No tokio: threads +
channels, like the CLI. `cargo audit` on the new tree is a gate.

## 2. The read model (`harness_tui::model`, pure, tested with fixture ledgers)

A `Snapshot` is rebuilt from `<target>/migration/` with harness-core only — on a key, after
a spawned command has been REAPED, and on a 2 s tick while a command runs or an `awaiting`
is outstanding (an external answerer may resume the run itself).

- **Facts freshness** — the same count `state status` prints (`facts fresh` / `facts STALE
  (n files)`), computed from `facts.jsonl` file hashes vs the tree.
- **Units** — `harness_core::status::unit_report` per plan unit: status, freshness,
  verdict state and stale list, contradiction / write-in-flight (LIVE holder only), attempts,
  and a new additive field `promotion_interrupted` (an attempt id from a `.promote-<id>/`
  marker, or `legacy` for a bare `.<crate>.prev`, reported only when no live writer holds the
  ledger): the rail says "promotion of <id> interrupted — the next writing command recovers
  it" instead of a contradiction glyph. The CLI's `state status` renders the same field.
- **Attempts** — `attempts::load_unit_attempts` (plus each `attempt-verdict.json` and
  `candidate/`), with provider kind (`human` shown as a label everywhere), `seeded_from`,
  `steer_note`, turns, outcome. **The ledger defines no order over a unit's attempts**; the
  rail sorts them deterministically: bound to the current inputs first, then by base id,
  each base before its `.rN` samples.
- **Provenance** — `harness_core::attempts::provenance` (moved out of harness-cli's bench,
  the ONE implementation of R-5): among green attempts bound to the CURRENT `unit_source`
  AND `driver` whose `candidate_digest` equals the unit crate's content hash
  (`hash::crate_content_hash`), after collapsing a steer attempt that reproduced its seed's
  candidate into the seed: exactly one model attempt → `Pipeline(id)` (`*` in the rail);
  several → `Ambiguous(ids)` (no `*`, the unit header says "ambiguous provenance"); only a
  `human` attempt → `Human(id)` (`*h`, "promoted from a hand edit"); none → `None` (on a
  verified unit: "provenance unknown"). The `bound` flag of the attempts summary (unit
  source only) is NOT the R-5 binding and is not used for this.
- **Verdicts** — `oracle-latest.json` and the selected attempt's `attempt-verdict.json`.
- **Function pairs**, per plan symbol (public first, then internal `<file>::<name>`):
  - *C side*: the `facts.jsonl` symbol record's file + 1-based span, sliced from the tree
    ONLY when that file's current hash equals its `facts.jsonl` file record's hash (the
    `state status` / triage freshness rule); the span is clamped to the file's length.
    Otherwise the side reads "facts predate <file> — run `harness scan`" (never shifted
    lines).
  - *Rust side*: in the selected crate (the unit crate, or an attempt's `candidate/`), every
    `src/**/*.rs` file is parsed with tree-sitter-rust; the **shim** is the `extern "C"`
    function named like the symbol (with `#[no_mangle]`, or an `export_name` equal to it)
    in any file — so the M0 layout (an inline `mod ffi` in `lib.rs`) works as well as the
    executor's `ffi.rs`. The **logic callee**: the call expressions in the shim's body, in
    order; a callee path's last segment is mapped back through the file's `use` declarations
    (lists and `as` aliases) and resolved against the crate's non-shim function definitions
    (preferring `logic.rs`); the first one that resolves and is not another plan symbol's
    shim is shown under the shim. No resolvable callee → "logic fn not identified" (the shim
    alone); no shim → "not found in <crate>" (the pair stays listed). The corpus scan in the
    review (144 promoted shims): 131 call through a `logic::` path, 11 through a `use`
    import (4 of them renamed, 1 aliased), 2 have no logic call at all.
- **Supersessions** (`superseded.jsonl`) — shown on the attempt they name.

**Display filter** (per rendered line; highlighting and diffing work on the raw text
first): expand `\t` to the next multiple of 8 display columns (carried across styled spans),
drop a trailing `\r`, map every other control character (C0, DEL, C1) to `?`, keep all other
text (`unicode-width` gives cell widths), cut at 4 KiB on a char boundary. No escape
sequence in a C comment or a model reply ever reaches the terminal, and tab-indented targets
render (neither shipped corpus has a tab; a synthetic fixture pins it).

## 3. Views (`view`, `tui` feature; `TestBackend` golden buffers)

```
┌ facts fresh ─┬ u-lib · a-13c941dfff95 (green, *) · boundary ✓ … ──────────────┐
│ ● u-lib      │ read_scalefactors                     ⇄  read_scalefactors (ffi) │
│ ✗ u-dec  RED │ int read_scalefactors(bitstream *bs,  │ pub extern "C" fn read_…│
│ ◐ u-hdr stale│ …                                     │ …  → logic::read_scale… │
│              │ ───────────────────────────────────── │ ────────────────────────│
│ attempts     │ static int helper(…)                  │ logic fn not identified │
│ a-13c9 green*│ …                                     │                          │
│ a-28d8 green │                                       │                          │
│   superseded │ [verdict] symbol-set ✓ capabilities ✓ … boundary ✓ (31 calls …) │
├──────────────┴────────────────────────────────────────────────────────────────┤
│ run: harness --json migrate u-dec --target … --no-promote --from a-… --steer … │
│ turn 1 steer → oracle · [check] differential-driver ✗ first diff at byte 812 … │
└────────────────────────────────────────────────────────────────────────────────┘
```

- **Rail**: the facts line, units with a glyph (green fresh / red / stale / contradiction /
  write-in-flight with the holder's command / promotion interrupted / provenance state),
  then the selected unit's attempts in the defined order (`*` pipeline provenance, `*h`
  human, `superseded`, `human`, `steer ← a-…` tags).
- **Pairs**: a vertically stacked list of function pairs, each two columns padded to equal
  height with filler lines; the function boundary is the unit of navigation. `d` compares the
  Rust side of the selected attempt with the provenance attempt's (`similar` line marks;
  disabled when provenance is not `Pipeline`/`Human`).
- **Verdict strip**: one chip per check; `v` expands the selected check's detail.
- **Run panel**: the spawned argv, then its events as they arrive (turn-start/turn-end with
  the Turn's result, check lines, `awaiting` with the response path and the CLI's `resume`
  hint as text, `error` with its kind, `result`), then — after the child is reaped — its exit
  (`exit N`, `interrupted (SIGINT)`, or `exited without result (exit N)`).
- **Narrow fallback**: below 110 columns (measured from the backend; `--layout stacked`
  forces it, `--layout split` forbids it) the pairs collapse to C-then-Rust per pair and the
  rail to a top line.
- **Keys**: `j/k` scroll, `]f`/`[f` pairs, `J/K` units, `Tab` rail focus, `Enter` select
  attempt, `d` diff, `v` verdict detail, `a` Accept, `m` Modify, `e` hand edit, `r` retry,
  `R` resume, `x` cancel, `g` reload, `?` keys, `q` quit (`Q` cancel + quit). **Every act
  shows the exact argv and asks `y/n`** — there is no automatic spawn.

## 4. Acts: every write is a spawned `harness --json …`

The TUI keeps the exact argv of each spawn (`Vec<OsString>`, the resolved binary first:
`harness` from `PATH` or `--harness <path>`). `--target` goes after the subcommand (it is not
global). It passes `--no-promote` on every `migrate` and never `--promote`, and
`--allow-unsandboxed` only when started with it.

| key | argv after the binary | enabled when (from the snapshot; the CLI re-checks everything) |
|---|---|---|
| `a` Accept | `--json promote <unit> <attempt> --target <root> [--replace]` | attempt green, its last turn green, not promoted (or `--replace` offered when the unit is verified or the attempt promoted) |
| `m` Modify | `--json migrate <unit> --target <root> --no-promote --from <attempt> --steer <note>` | the selected attempt is finished, bound (R-5 binding) and has a `candidate/`; the note (1..2000 printable bytes) is typed in a one-line input, `Esc` cancels |
| `e` hand edit | `--json override <unit> <dir> --target <root> [--note <text>]` | the selected crate is in the executor layout; `e` copies its `src/logic.rs` + `src/ffi.rs` into a fresh temp dir, hashes them, suspends the TUI, runs `sh -c '$EDITOR "$@"' -- <dir>/src/logic.rs <dir>/src/ffi.rs` (so `EDITOR="code --wait"` works), resumes; an editor exit ≠ 0 aborts; unchanged hashes → "no change; nothing to record", no prompt; disabled while a command runs |
| `r` retry | the argv of the attempt's own run shape (`migrate … --no-promote --retry`, plus `--from`/`--steer` for a steer attempt) | the attempt is finished |
| `R` resume | the stored argv of the run that ended `awaiting`, unchanged | that attempt is still `in-progress` in the snapshot and its awaited response file exists (non-empty and parses as a JSON object); stays available after a failed resume |
| `x` cancel | — (`/bin/kill -INT <child pid>`) | the child is running (`try_wait` is `Ok(None)`, so a reused pid is never signalled) |

**Resume.** The watcher never spawns: when the awaited response file exists it marks the run
panel "response present for a-…"; `R` asks and re-spawns the stored argv (never the event's
`resume` string, a human hint without `--json`). On the resumed run's first `turn-start`, the
attempt must equal the one awaited; a mismatch is shown, not chased.

**Child process.** Spawned with `process_group(0)` (std; a terminal hangup then reaches only
the TUI's group, never kills the harness by the default action — the harness always cancels
cleanly or runs to completion with its timeouts enforced), stdin `/dev/null`, stdout the
NDJSON pipe (a reader thread → a channel), stderr on a second reader thread (both drained, so
neither pipe can fill), one command at a time (the writer lock would refuse a second anyway;
its `locked` error is shown with the holder). The snapshot is reloaded only after reader EOF
and `wait()` — never on the `result` event alone (on a signal the CLI emits `result` before it
dies holding the lock line, and `kill -0` reads an unreaped zombie as alive). "Interrupted"
comes from `ExitStatus::signal()`, not from the event.

**The TUI's own signals.** It registers SIGINT, SIGTERM and SIGHUP (signal-hook): if the
child is running, `/bin/kill -INT <child>` and wait ≤ 1 s (the CLI's 250 ms courtesy budget
plus the group kill); restore the terminal best-effort (leave the alternate screen, disable
raw mode); die by the signal. A panic hook restores the terminal before printing. A
SIGKILLed TUI leaves the harness running to completion (timeouts enforced, ledger
consistent; its events go to a dead pipe and are dropped).

## 5. What the CLI grows (step 1 of the order of work)

### 5.1 `harness migrate <UNIT> --steer <NOTE> --from <ATTEMPT>` — a steer attempt

A NEW attempt seeded from a finished attempt of the unit. `--steer` and `--from` are
required together (the ledger has no order, so there is no default seed; `--steer` alone is
refused, listing the unit's finished attempts bound to the current inputs). Refusals (exit
1, before anything is sent): `--from` is not a clean attempt id, not an attempt of the unit
(`attempts::load_pinned`), not finished, not bound to the current `unit_source` AND `driver`
(the R-5 binding, `attempts::current_binding`), has no `candidate/` or a `candidate/` that
does not match its `candidate_digest`, or has no `attempt-verdict.json`; the note is empty,
over 2000 bytes, or not printable (`\n` and `\t` allowed).

**The first turn** is of kind `steer`: a repair-shaped request built from committed evidence
only —
- `[CURRENT RUST]`: the seed's `candidate/src/{logic.rs,ffi.rs}`;
- `[FAILURE CLASS]`/`[EVIDENCE]`: from the seed's stored `attempt-verdict.json` — green →
  the fixed lead-in "green — every check passed; the guidance below asks for a change
  anyway"; red → `classify` + `verdict_explanation` + the failed-check details quoted and
  scrubbed exactly as a repair turn's, but WITHOUT the driver-output excerpt (that reads
  gitignored `migration/build/<unit>/`, which belongs to whatever ran last; the stored
  `differential-driver` detail keeps the replay-stable summary). Stored verdicts carry no
  machine paths (0 of 103 committed ones), so the section is a pure function of the ledger;
  the R-3 leak check runs on it like on any repair request;
- `[GUIDANCE]`: the note, verbatim;
- `[HISTORY]` (empty for turn 1), `[TASK]` = the repair task.
Then ordinary repair turns (`[HISTORY]` reads `1. steer -> …`).

**Record** (additive, `skip_serializing_if = None`): `seeded_from: "<attempt id>"`,
`steer_note: "<note>"` — so HEAD can render turn 1 from the ledger alone. `Turn.kind` gains
`steer`. `prompt_digest` = the digest of the FIRST turn (translate, or steer). The id
derivation is frozen and unchanged: its last input is the first turn's request key, so a
steer attempt never collides with a translate attempt, and the same steer (seed, note,
inputs, provider kind, model) is the same attempt — `--retry` samples it like any other.

**Engine** (harness-llm): the job's first turn is a property, `FirstTurn::{Translate,
Steer{seed, note}}`. `Job::run` renders the first request from it. The verification paths
(`--provider replay` with `--attempt`, a finished trace-backed re-run, `--retry`'s
re-verification, `replay_divergences`) load the record FIRST and build the first turn from its
`seeded_from`/`steer_note` (a mismatch with the command line's `--steer`/`--from` is an
error). The seed is loaded with `load_pinned` and its `candidate/` checked against its digest;
a missing, unfinished or modified seed is an integrity error (never a divergence). `drive`
journals the first turn's kind from the job, and the evidence-only drift rule applies to a
steer first turn as to a repair turn (`index > 0 || kind == steer`). A prompt fixture pins the
branch (a red `differential-driver` seed, proving no excerpt; a green seed).

**Resume hint.** `resume` echoes `--from`/`--steer`, and every interpolated value is
POSIX single-quoted (`'` → `'\''`), `--target` included. The `awaiting` event gains an additive
`args` array (the argv after `harness`, without `--json`, verbatim) for clients that want to
re-run without parsing prose; `resume` stays a human hint.

### 5.2 `harness override <UNIT> <DIR> [--note <TEXT>] [--target …] [--allow-unsandboxed]` — a labelled human attempt

The hand edit's only way into the ledger. It reads exactly `<DIR>/src/logic.rs` and
`<DIR>/src/ffi.rs`, and refuses (exit 1) when `<DIR>/src` holds any other file, when a
`Cargo.toml` or `src/lib.rs` is present and differs from the harness-owned texts, when `<DIR>`
is inside the target's `migration/` tree, or when a file is a symlink. It takes the writer
lock, recovers promotions, and checks the migrate preconditions (plan staleness, R6). It
refuses when an existing attempt of the unit bound to the current inputs has byte-identical
`candidate/src/{logic.rs,ffi.rs}` ("identical to attempt a-… (<outcome>); nothing to
record") — so a no-op edit can never make provenance ambiguous.

Then it runs the migrate stage's ONE judge in `attempts/<id>/`: the deny scan (with the
unit's stdio externs), `write_candidate` (harness manifest + `lib.rs` + the two files), the
oracle, `attempt-verdict.json`, `candidate_digest` and `toolchain` set post-build — exactly
as for a model turn; the promoted crate keeps the compiler-enforced shape. The record:
`provider: "human"`, `provider_kind: "human"`, `model: "-"`, `note` (additive, printable, ≤
400 bytes), one turn `{kind: "human", result: "green" | the judge's failure class,
request_key: "", response_hash: blake3(logic ‖ NUL ‖ ffi), tokens null}`, `outcome` green or
red. Id = the frozen derivation over `(unit, unit_source, driver, "human", "-",
response_hash)`, so the same edit is the same attempt (a re-run on the same source is
refused as identical). A judge harness error (driver-shape, boundary C-side) records nothing
and removes the attempt dir. `override` never promotes; Accept (`harness promote`) promotes a
green human attempt like any other (its last turn is green).

**The benchmark never counts a hand edit as the pipeline's.** R-5 is extended by provider
kind (the core `provenance` function): a verified crate whose provenance is `Human` is a
PROBLEM in `bench score`/`bench check` ("the verified crate was promoted from a human
(override) attempt — not pipeline provenance"), so `--write` refuses it, exactly as an
unattributed hand edit is refused today. `bench check --replay` reports human attempts
`skipped (human)` (nothing to replay). Every surface that lists attempts shows the label.

## 6. Process model summary

UI thread: crossterm events + the event channel + a 60 ms tick; no blocking I/O. Reader
threads for stdout (NDJSON → typed `Event`, unknown `k` kept as `Other`, a non-JSON line shown
as such) and stderr. A command ends when both readers hit EOF and `wait()` returns; then the
snapshot is reloaded and the exit shown. Quit with a running child asks; `Q` cancels (SIGINT)
and quits.

## 7. Tests

- **CLI** (harness-llm/harness-cli, before any TUI code depends on them): steer rendering and
  its fixtures (red `differential-driver` seed: no excerpt even with differing
  `build/<unit>/drv_*.out` present; green seed); rendering twice across a `verify` that
  rewrites the build dir → byte-identical, same id; seed refusals (no `--from`, unfinished,
  unbound, modified candidate, no verdict); replay of a steer attempt reproduces with
  `drifted == []` after `migration/build/` is removed; e2e through the external hand-off with a
  note containing spaces, both quote types and `$x`, resumed by running the event's `resume`
  through `sh -c` → the same attempt id finishes, no second attempt dir; `override`: red and
  green human attempts, the identical-source refusal, the shape refusals, override + promote,
  and the `provenance` function's four outcomes (unit tests in harness-core, including a
  steer attempt that reproduced its seed and a red attempt whose digest equals the crate);
  `bench`'s human-provenance PROBLEM (unit test on a synthetic case).
- **model**: fixture ledgers — the tractor cases (executor layout, 89 promoted crates, the two
  `superseded.jsonl`), the zopfli ledger (inline `mod ffi`, provenance `None`), synthetic
  dirs: a bare-imported renamed callee (024's `run` → `run_logic`), an `as` alias, a shim
  with no logic call, a C file edited after the scan (stale label, not shifted lines), a
  rescan without a replan (spans shown), a tab-indented C file.
- **view**: `TestBackend` goldens for wide, narrow and `?`; filler lines; a failed check; the
  run panel mid-run; the tab fixture (indentation at columns 8/16; a 4 KiB cut inside a
  multibyte character lands on a char boundary).
- **actions**: the NDJSON reader against event streams recorded from the CLI e2e tests
  (migrate, awaiting, locked, promote rolled back); `R` availability after a failed resume;
  the watcher never spawns; reload happens only after reaping; an unchanged hand edit spawns
  nothing; SIGHUP to the TUI's group kills the spawned harness's driver group (the spinning
  driver fixture of the CLI e2e) and the harness dies by SIGINT.

## 8. Order of work

1. harness-core: `current_binding`, `unit_crate_digest`, `provenance` (bench switches to
   them, same problem texts), `AttemptRecord.{seeded_from, steer_note, note}`,
   `UnitReport.promotion_interrupted`.
2. harness-llm + harness-cli: `--steer`/`--from` (FirstTurn, verification from the record,
   fixtures, quoting, `args`), `harness override`, bench's human PROBLEM and replay skip;
   SCHEMAS.md; the CLI tests above. Adversarial code review + fix pass; commit.
3. `harness-tui`: model (lib) + tests; MSRV 1.90; `default-members`.
4. Views, acts, process model, signals; tests; README; DECISIONS.md. Adversarial code
   review + fix pass; commit.

## R. Design review — 20 confirmed findings and their resolutions

Four lenses (ledger & replay, read model & pairing, process & acts, scope & consistency),
every finding attacked by an independent verifier against the code; 20 confirmed, 0 refuted.

| id | finding (short) | resolution |
|---|---|---|
| LEDGER-H1 | steer `[EVIDENCE]` would read the driver excerpt from gitignored build scratch — wrong evidence, unreproducible, id depends on scratch | §5.1: evidence from the stored verdict only, no excerpt; verdicts carry no machine paths; fixture + rewrite-the-build-dir test |
| LEDGER-H2 | turn 1 of a steer attempt unimplementable as "nothing else changes": drive journals `translate`, verification renders HEAD's translate request, the note is not in the record | §5.1: `FirstTurn` on the job, verification builds it from the record (`seeded_from`, `steer_note`), kind `steer`, prompt_digest of the first turn, evidence-only drift rule extended |
| LEDGER-H3 / SCOPE-1 | scores.json cannot tell a human-promoted crate from a model's: a hand edit would score strict-pass | §5.2: `provenance` → `Human` is a PROBLEM; `--write` refuses |
| LEDGER-H4 | the human attempt's judged shape, hashes and turn result unspecified; a verbatim crate copy bypasses the harness-owned manifest/lint structure | §5.2: exactly logic.rs + ffi.rs through the one judge; hashes and the turn defined |
| LEDGER-M1 | "latest finished attempt" is undefined: the ledger has no order | §5.1: `--from` required with `--steer`; §2: a defined rail order |
| RM-1 / PROC-M3 | a no-op hand edit (or a forking editor) files a second green attempt with the same digest → ambiguous provenance | §5.2: identical-source refusal; §4: hash before/after, `sh -c '$EDITOR "$@"'`, non-zero exit aborts |
| RM-2 / SCOPE-4 | the promoted-attempt rule restated without binding/multiplicity; the rule lives privately in harness-cli; the model in a binary cannot be shared | §2: core `provenance` (one implementation); §1: lib + feature-gated bin |
| RM-3 | C sliced by facts spans with no freshness guard shows shifted lines | §2: per-file hash guard, clamp, stale label; facts line in the rail |
| RM-4 | the `logic::`-path rule misses bare imports, aliases, inline `mod ffi` | §2: shims found in every file; callee via `use`/alias mapping resolved against crate fns |
| RM-5 | tabs are control characters: `?` or dropped by ratatui | §2: tab expansion at render; char-boundary cut; tab fixture |
| PROC-H1 / SCOPE-2 | the `resume` string has no `--json`/`--harness`, is unquoted, lacks the steer note | §4: `R` re-spawns the stored argv; §5.1: quoting, `--from`/`--steer` in the hint, `args` array |
| SCOPE-3 / PROC-M2 | the automatic resume contradicts the y/n rule, races the answerer, dead-ends on a torn file | §3/§4: no automatic spawn; `R` act gated on state (in-progress + file parses), reload on the tick while awaiting |
| PROC-H2 | a hangup kills the harness child by the default action (its SIGHUP handler is off under pipes) → orphaned sandboxed groups; the TUI's terminal not restored on signals | §4: child in its own process group; the TUI's signal path (INT the child, restore, die by the signal) |
| PROC-M1 | reload on `result` sees the killed CLI's lock line (zombie reads alive) → false write-in-flight; an interrupted promotion then shows as a contradiction | §4: reload only after reaping; `promotion_interrupted` in `UnitReport` |
| SCOPE-5 | the zopfli fixture cannot exercise the pair locator or provenance | §7: tractor fixtures, zopfli for the inline-`mod ffi` and `None` cases, synthetic dirs |

**Follow-ups, not in this milestone:** `verify` lacks the R6 gate (from CLI hardening);
driver-attempt Accept; queueing on contention; per-function verdict dots (the boundary
report's per-call figures) once the verdict records them per symbol.
