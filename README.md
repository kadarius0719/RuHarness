# RuHarness

RuHarness moves a program written in **C**, an older programming language, to **Rust**, a newer
one that rules out a whole family of **memory mistakes** (a program reading or writing parts of the
computer's memory it should not, a common cause of crashes and security holes). It moves one piece
at a time, and proves each piece behaves the same as the C it replaces by running both and
comparing what they print. An AI model may write the Rust; it never decides whether the Rust is
right.

- **Who it is for.** Anyone with a C program they want in Rust without reading every line of the
  translation. A careful non-programmer can follow the guides.
- **Where it runs.** A Mac. On Linux it runs only without its safety box (the *sandbox*, a
  locked-down space for code a model wrote); Windows has not been tried.
- **What it touches.** It adds one folder, `migration/`, inside the project, plus one small trust
  file in your Library folder. Your C files are never changed.

## Where to go

- **Get it running in an hour** → [docs/FIRST-HOUR.md](docs/FIRST-HOUR.md): the tools, the
  built-in example, mapping liblzg, migrating one unit, planting a bug.
- **Move your own C program** → [docs/topics/map.md](docs/topics/map.md): point the harness at a
  C project of yours, accept a program, work on it.
- **Understand one part** → [docs/topics/](docs/topics/), seven short pages:
  - [judge.md](docs/topics/judge.md): how the judge decides GREEN or RED
  - [drivers.md](docs/topics/drivers.md): the small test programs the judge runs
  - [cockpit.md](docs/topics/cockpit.md): the full-screen view and its chat
  - [features.md](docs/topics/features.md): named runs of the whole program, checked every time
  - [speed.md](docs/topics/speed.md): timing the Rust against the C
  - [map.md](docs/topics/map.md): mapping your own C project
  - [models.md](docs/topics/models.md): who writes the Rust, and how to hand it a question
- **Something went wrong** → the exit-code table [just below](#if-something-goes-wrong) and
  [docs/TESTING-GUIDE.md](docs/TESTING-GUIDE.md)'s [Troubleshooting](docs/TESTING-GUIDE.md#troubleshooting).

The full walk with every screen is [docs/TESTING-GUIDE.md](docs/TESTING-GUIDE.md); the ideas in
full are [docs/TUTORIAL.md](docs/TUTORIAL.md).

## If something goes wrong

`echo "exit=$?"`, run right after a command, prints its exit code:

| Code | Meaning |
|---|---|
| `0` | success, or GREEN |
| `1` | the harness refused or stopped with an error (the message says why); also "waiting for an answer" at a hand-off |
| `2` | the command was typed wrong |
| `10` | the judge said RED |
| `130` | you stopped it with Ctrl-C, and it stopped everything it had started; run it again |

For any other line, the guide's [Troubleshooting](docs/TESTING-GUIDE.md#troubleshooting) says what
to do, and its [Known quirks](docs/TESTING-GUIDE.md#known-quirks-in-this-version) lists the few
out-of-date messages.

---

**Reference: look things up here; a newcomer can stop reading here.** Newcomers: look things up in
the tutorial's friendly [appendix](docs/TUTORIAL.md#16-appendix-reference) instead; the tables
below are written for developers.

## Command reference

Every command is `harness <command>`; `cargo run -p harness-cli -- <command>` is the same program
run from the source. Each takes `--target <folder>` (default: the current folder) and, on a mapped
project, `--tool <id>`; `harness <command> --help` lists every option.

| Command | What it does |
|---|---|
| `scan` | Parses the C (tree-sitter) into `migration/facts.jsonl`: files with include edges, symbols with canonical ids and signatures, call refs. |
| `plan` | Clusters files into units (cycles collapse into one), hashes each unit's include closure (`source_hash`), and reconciles `migration/plan.toml`: statuses, comments and unknown fields survive; order is re-derived from `depends_on`. |
| `gen-driver <unit>` | Asks a model for the unit's differential driver, then validates it against the original C only: strict build, `driver-shape`, every symbol called, three identical runs, `-O0` == `-O2`, ASan/UBSan, mutation adequacy (broken copies of the C must change its output; equivalent mutants discarded). Green: `units/<id>/driver.c` + `driver-validation.json`. |
| `migrate <unit>` | Asks the provider for exactly `src/logic.rs` (safe Rust) and `src/ffi.rs` (the C-ABI shim) in a harness-owned crate whose `lib.rs` confines `unsafe` to the shim; runs the oracle; up to three stateless repair turns. Each attempt is recorded under `units/<id>/attempts/<id>/` (turns, tokens, candidate, `prompt_digest`). Green is promoted (crash-safe two-rename) and re-verified in place. Refuses a unit whose driver is not freshly validated. |
| `verify <unit>` | Refuses if the source changed since planning; else runs the oracle (symbol-set, capabilities, driver-shape, differential driver, whole program all-C vs mixed, sanitizers, features) and writes a content-bound verdict, `oracle-latest.json` (blake3 digests of all it tested). Red demotes `verified → in-progress` and keeps `oracle-last-green.json`. |
| `promote <unit> <attempt>` | Promotes a recorded green attempt and verifies it in place: the explicit act a review's Accept is (`--replace` over a verified unit). |
| `override <unit> <dir>` | Records a hand edit (exactly `src/logic.rs`, `src/ffi.rs` of `dir`) as a labelled `human` attempt, judged like a model reply; never promotes. |
| `state status` | Staleness: facts, plan hashes and verdict digests vs the tree; contradictions; verdicts "made elsewhere". |
| `features init\|save\|map` | Named runs of the whole program, checked on every `verify` (`feature:<name>/<scenario>`); `map` records which functions each runs. Guide Part 8; docs/FEATURES-DESIGN.md. |
| `project map\|accept\|ask` | A whole C project's programs, files and held choices; `accept` writes a tool's `harness.toml`. docs/PROJECT-MAP-DESIGN.md. |
| `perf init\|save\|run\|show` | The C against the Rust on your workloads (CPU time, instructions, memory), information only, in `migration/perf/`. Guide Part 11; docs/PERF-DESIGN.md. |
| `detect` | Deterministic hazard detectors (macros, function pointers, unions/bitfields, setjmp/signals/threads, variadics, mutable globals, allocator ownership) into content-keyed findings; what the suite cannot see (pointer arithmetic, aliasing) stays a standing caveat. |
| `observe` | Model triage of findings per unit (confirm/dismiss/uncertain; nonce-delimited code, content hashes binding each verdict) into `triage.jsonl` and `observations.md`, ranked by a deterministic risk score. |
| `review <finding>` | A person's review: `--uphold-dismiss` or `--reinstate`. A dismissal keeps full risk weight until upheld. |
| `bench …` | The benchmark: `vendor`, `verify-corpus`, `init`, `status`, `score [--write]`, `boundary`, `check [--replay]` (the regression suite, exit 10 on a regression; `--replay` re-judges every recorded trajectory from its evidence, zero tokens; docs/REPLAY-DESIGN.md). targets/tractor/README.md. |
| `sync-runtime` | Regenerates the managed block of the target's `AGENTS.md`; `--check` for CI. |

| Flag | Meaning |
|---|---|
| `--adopt` | Trust a ledger made on another computer, once per checkout: deletes its build folders, writes a fresh token; its verdicts show "made elsewhere" until verified here. |
| `--target DIR`, `--tool ID` | The project; on a mapped project, which accepted tool. With two or more tools and no `--tool`, the command refuses (exit 1) and names them: "… pick one with --tool (t-a, t-b)"; the cockpit opens on its Project view, a row to open each tool. |
| `--provider NAME` | `external` (file hand-off, the default), `replay`, `anthropic` (`ANTHROPIC_API_KEY`), or a profile from `$RUHARNESS_PROVIDERS`. A target's `harness.toml` can only name a profile; endpoints and keys live in your file. |
| `--model NAME` | The model sent to the provider; with `external`, who answers (recorded). |
| `--retry` | Records a new sample; finished attempts are never overwritten. |
| `--promote`, `--no-promote` | Replace a verified unit's Rust with a new green candidate; or record green without promoting (`[llm.migrate] promote_on_green = false` makes that the only path). |
| `--steer <NOTE> --from <ATTEMPT>` | A new attempt seeded from a finished one (its code, verdict and your note on every turn); a note starting with `-` is attached: `--steer='- keep the loop'`. Never counted as unassisted. |
| `--requester=chat` | Labels the attempt as asked in a chat (hand-offs in `traces/chat/`, never scored); with `--answer=FILE --answer-key=KEY` (`-` reads stdin, framed by `--answer-bytes=N`) files the answer to pending request KEY and resumes, else `answer-refused`. |
| `--attempt ID` | With `--provider replay`: which recorded attempt to re-check. |
| `--allow-unsandboxed` | Where no sandbox exists (Linux): accept running untrusted code unconfined. |
| `--json` | Global: stdout becomes newline-delimited `ruharness-events` (docs/SCHEMAS.md "CLI hardening"); human logs stay on stderr. |

Exit codes as above; for `migrate`, `10` also means blocked, truncated or format. Machine consumers
read the ledger, not stdout. Writing commands hold a writer lock on the target's ledger
(`migration/.lock`, or `migration/tools/<id>/.lock` for a mapped tool; `project map`, `project
accept` and `sync-runtime` take the project lock `migration/map/.lock` first); a second writer fails
fast naming the holder. Ctrl-C kills every live sandboxed process group and the
harness dies by the signal. Target and model code builds and runs under `sandbox-exec` (no network,
no reads of your home folder, writes confined to the build folder, scrubbed environment, timeouts),
and the symbol-set check rejects a candidate exporting more than the unit's symbols or a pre-main
constructor.

## The cockpit (`harness-tui`)

```bash
harness-tui --target targets/zopfli
```

Files and units on the left, the selection on the right, a chat pane; `Enter` opens a menu of what
can be done now. On a ledger not yet adopted it first asks `Adopt this folder? Type y and Enter to
adopt`. Keys, menus and symbols: [docs/TUTORIAL.md](docs/TUTORIAL.md); the walk: guide Parts 4 and 9.
Flags (`harness-tui --help`): `--target DIR` (default `.`), `--tool ID`, `--harness PATH` (default
on PATH, else next to the cockpit), `--provider NAME` (repeatable, default `external`; the target
never chooses it), `--allow-unsandboxed`, `--layout split|stacked`, `--no-mouse`, `--theme
NAME` (`terminal`, the default, uses your terminal's own colours; `dark` or `light` are designed
palettes, `mono` has none; also `RUHARNESS_THEME`; `NO_COLOR` picks `mono`; a settings file
`cockpit.toml` can keep the theme and change any colour: [docs/topics/cockpit.md](docs/topics/cockpit.md)), `--no-chat`,
`--chat-runtime PATH` (the `claude` to run), `--harness-mcp PATH` (default next to the cockpit,
else on PATH), `--chat-model NAME`. `cargo build` at the root skips it. Without installing,
build the three programs together and run the cockpit from the build folder:
`cargo build -p harness-cli -p harness-tui -p harness-mcp && target/debug/harness-tui --target
DIR` (built apart, a cockpit newer than its `harness-mcp` says so in the chat's connection dialog,
with the rebuild line to paste). Install the three programs (`harness`, `harness-tui`, `harness-mcp`) with the three
`cargo install --locked --path crates/…` lines of the testing guide's Step 0.19
([docs/TESTING-GUIDE.md](docs/TESTING-GUIDE.md), Part 0); reinstall the cockpit whenever you
reinstall harness-cli, as it uses the `harness` first on your PATH.

The chat's safety rule: the chat is your own Claude Code, reading the project through
`harness-mcp --cockpit`, with no tool that writes. Every act it wants waits until you review it in
the same armed dialog as any act; it is labelled `requester: chat`, never scored (docs/CHAT-PANE-DESIGN.md).

## The ledger in chat (`harness-mcp`)

A stdio MCP server (the Model Context Protocol: how an agent runtime such as Claude Code calls
outside tools). It reads the ledger as data and poses labelled review acts (`harness_status`,
`harness_unit`, `harness_steer`, `harness_request`, `harness_answer`, `harness_retry`,
`harness_promote`), each a spawned `harness --json …` under the writer lock, sandbox and oracle. It
records steer attempts only, never a fresh translation, and refuses every pending blind hand-off.
Protocol and design: [docs/MCP-DESIGN.md](docs/MCP-DESIGN.md). Install it with the testing guide's Step 0.19
(`cargo install --locked --path crates/harness-mcp`), and adopt the target first (the `--adopt` step there, or the cockpit's question): it refuses a ledger made
elsewhere and never adopts. In the project's `.mcp.json`; on a mapped project, add the tool:

```json
{ "mcpServers": { "ruharness": { "command": "harness-mcp", "args": ["--target", "targets/zopfli"] } } }
```

```json
{ "mcpServers": { "ruharness": { "command": "harness-mcp", "args": ["--target", ".", "--tool", "t-lzg"] } } }
```

Flags (`harness-mcp --help`), all yours, none reachable from a tool argument: `--target DIR`,
`--tool ID`, `--target-root DIR` (repeatable; a call may name a target strictly inside one; never
`/` or `$HOME`), `--harness PATH`, `--provider NAME` (repeatable; default `external` only: no
credentials, no spend), `--allow-unsandboxed`; `--cockpit` is the cockpit's mode (reads, and acts
that only ask). **Never serve `targets/tractor/cases` to a chat agent:** the blind protocol answers
hand-offs there and the committed tree holds a pending one. Close the ledger to the runtime's own
file tools (best effort: a shell can still write any file), in `.claude/settings.json`:

```json
{ "permissions": { "deny": ["Edit(/targets/**/migration/**)", "Bash(harness *)", "Bash(cargo run -p harness-cli *)"] } }
```

## Project status and benchmark

| Milestone | Scope | Status |
|---|---|---|
| **M0** end-to-end thread | One leaf unit migrated and differentially verified | ✅ |
| **M1** ledger + schemas | Fact model, plan, content-bound verdicts, multi-unit ordering, workspace, CI | ✅ |
| **M2** observer | Hazard detectors, risk scoring, model triage, human review loop, runtime view | ✅ |
| **M3** executor + provider #2 | `migrate` (translate → oracle → repair), sandboxed execution, two wire adapters live | ✅ |
| **M4** benchmark | TRACTOR B01 library suite (100 cases), driver generation with C-vs-C self-validation, held-out scoring, scores as regression suite | ✅ |
| **M5** extension proof | External detector plugin + `EXTENDING.md` | — |
| **M6+** Phase 2 spike | Second language frontend, golden-test oracle | — |

Since M4: the cockpit and its chat, harness-mcp, features, perf and the project map. zopfli is
vendored at a pinned commit (DECISIONS.md). Benchmark: DARPA TRACTOR public corpus v2, Battery-01
library cases, macOS arm64, `targets/tractor/scores.json`; per-case strict pass (every non-UB
held-out vector passes) over scorable cases.

| Split | Strict pass | Oracle-verified | Blind spots | Non-UB vectors |
|---|---|---|---|---|
| public (80 cases) | **70/77** (90.9%) | 70 | 0 | 908/950 |
| released-hidden (20 cases) | **16/17** (94.1%) | 16 | 0 | 86/87 |

A *blind spot* is a unit the oracle verified that still fails a held-out vector. M4 found three,
diagnosed in DECISIONS.md: an oracle hole (stderr not compared), fixed; the corpus's own undefined
behaviour, excused with disclosure (`unmarked-UB`); an FFI-boundary bug, closed 2026-09-24 when
`read_scalefactors_lib` was re-baselined. Public-vector scores (the vectors predate the models'
training cutoff), **not comparable** to the First TRACTOR Evaluation Report; see
targets/tractor/README.md.

## Repository layout and working on the harness

```text
crates/harness-core/    fact model, schemas, plan, verdicts, observer, risk, planner, adoption
crates/harness-scan/    C frontend (tree-sitter): facts, mutation sites, driver lint
crates/harness-detect/  built-in hazard detectors (c-treesitter-v1)
crates/harness-llm/     provider profiles + adapters, triage, the trajectory engine
crates/harness-oracle/  c-abi-differential oracle: sandbox, gates, validate_driver, scorer
crates/harness-cli/     the `harness` binary
crates/harness-tui/     the cockpit: read model + events reader (a library) and front end
crates/harness-mcp/     the stdio MCP server (reuses harness-tui's library)
devtools/               scripts for testing RuHarness itself: cockpit driver, map spike, guide gates
docs/                   FIRST-HOUR.md (the one-hour start), topics/ (seven short pages),
                        SCHEMAS.md (normative ledger schemas), TESTING-GUIDE.md, TUTORIAL.md,
                        AGENT-BRIEFING.md; designs: TUI, CHAT-PANE, MCP, CLI-HARDENING, FEATURES,
                        FEATURES-PROBE-REDESIGN, PERF, PROJECT-MAP (+ -INVESTIGATION),
                        ORACLE-HARDENING, REPLAY, M4, COCKPIT-WRAPPER; NEXT-SESSION.md; reviews/;
                        README.md (what every document is for, current or history)
providers.example.toml  example provider profiles (user-level; never read from a target)
targets/tractor/        TRACTOR B01 suite: suite.toml, corpus.lock, cases/, heldout/, scores.json,
                        handoff-tools/
targets/zopfli/         the example: harness.toml, AGENTS.md (generated), migration/ (the ledger)
DECISIONS.md            engineering log: spikes, decisions, milestone handoffs
```

Unit crates under `targets/` are not workspace members: the oracle builds them via
`--manifest-path`, so a broken unit never bricks the harness's own build. Stable Rust (pinned in
`rust-toolchain.toml`) and a C compiler with ASan/UBSan. CI (GitHub Actions, macOS + Ubuntu; the Ubuntu job has
been red since 2026-09-17, DECISIONS.md 2026-10-03):
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`
(including an end-to-end migrate-and-verify of u001 in a temp copy), `cargo deny`; `Cargo.lock` is
committed. Every dependency is justified in DECISIONS.md ([docs/AGENT-BRIEFING.md](docs/AGENT-BRIEFING.md)
§11): serde/serde_json, toml/toml_edit, blake3, thiserror, tree-sitter (+C and Rust grammars),
clap, anyhow, signal-hook, libc, ureq (provider HTTP); the cockpit adds ratatui (on crossterm),
similar, tree-sitter-highlight and unicode-width. Working on RuHarness: the briefing first, then
docs/NEXT-SESSION.md and DECISIONS.md's last entries.

## License

Harness crates: MIT OR Apache-2.0. `targets/zopfli/` is Google's zopfli, Apache-2.0 (see its
`COPYING`); `katajainen_rs` is a derivative of it and stays Apache-2.0.
