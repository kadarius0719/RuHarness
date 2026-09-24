# RuHarness

A harness that migrates existing codebases to safe, idiomatic Rust **incrementally
and verifiably** — LLM agents supply judgment, deterministic tooling supplies
measurement, and a differential oracle is the only definition of done.

**Phase 1 (current): C → Rust.** C is the starting point because the verification
oracle is cheapest there: the migrated Rust exposes the identical C ABI, so old and
new implementations can be differentially tested behind the same interface, and the
project keeps building and passing its tests as a mixed C/Rust link at every commit.
Phase 2 (later) targets dynamic languages, where types and ownership must be
invented rather than translated — same skeleton, different planner and oracle.

## Core principles

- **The oracle is the product.** Models are swappable commodities; the verification
  machinery is the durable value. "Compiles and passes the oracle" is the *only*
  definition of done for a migrated unit — never an agent's judgment of correctness.
- **State lives on disk, not in conversation.** All pipeline state is committed
  plain text in the target repo (the *ledger*); anything binary or regenerable is a
  derived, gitignored cache. Any agent, from any provider, resumes any stage cold by
  reading the ledger — and every `verified` claim is **content-bound**: verdicts
  record blake3 digests of exactly what was tested.
- **Deterministic tools measure; agents interpret.** Parsing, graph construction,
  planning order, test execution, and diffing are plain code with byte-reproducible
  output. LLMs rank, explain, plan, and write code — nothing a script could do.
- **Provider-agnostic by construction; migrate in dependency order behind an FFI
  seam; unsafe Rust only at the boundary shim.**

The normative ledger schemas are in [docs/SCHEMAS.md](docs/SCHEMAS.md); every
engineering decision, research spike, and milestone handoff is in
[DECISIONS.md](DECISIONS.md).

## Status

| Milestone | Scope | Status |
|---|---|---|
| **M0** — end-to-end thread | One leaf unit migrated and differentially verified | ✅ |
| **M1** — ledger + schemas | Fact model, plan, content-bound verdicts, multi-unit ordering, workspace, CI | ✅ |
| **M2** — observer | Gotcha detectors, risk scoring, LLM triage, human review loop, runtime-view sync | ✅ |
| **M3** — executor + provider #2 | `harness migrate` (translate → oracle → repair), sandboxed execution, two wire adapters proven live | ✅ |
| **M4** — benchmark | TRACTOR B01 library suite (100 cases), LLM driver generation with C-vs-C self-validation, held-out scoring with the corpus's own runners, scores as regression suite | ✅ |
| **M5** — extension proof | External detector plugin + `EXTENDING.md` | — |
| **M6+** — Phase 2 spike | Second language frontend, golden-test oracle | — |

The working target is a vendored [zopfli](https://github.com/google/zopfli) (pinned
commit in `DECISIONS.md`). Its plan currently holds 11 units in dependency order;
`u001-katajainen` (length-limited Huffman codes) is migrated to safe Rust behind the
identical C ABI and oracle-verified, with zero human-written Rust.

**M4 benchmark result** (DARPA TRACTOR public corpus v2, Battery-01 library cases,
macOS arm64, recorded in `targets/tractor/scores.json`). Headline = per-case strict
pass (every non-UB held-out vector passes) over scorable cases:

| Split | Strict pass | Oracle-verified | Blind spots | Non-UB vectors |
|---|---|---|---|---|
| public (80 cases) | **70/77** (90.9%) | 70 | 0 | 908/950 |
| released-hidden (20 cases) | **15/17** (88.2%) | 16 | 1 | 83/87 |

A *blind spot* is a unit the oracle verified that still fails a held-out vector:
exactly what the benchmark exists to find. M4 found three, all diagnosed in
`DECISIONS.md`: one was a real oracle hole (stderr was not compared) — fixed, the unit
re-migrated; one was the corpus's own undefined behavior — its two vectors are now
excused, with disclosure, by a sanitized-C pass (`unmarked-UB`), which leaves that case
unscorable; one (an FFI-boundary bug in the Rust) remains open. M4 recorded 14/18.
These are public-vector scores (the vectors predate the answering models' training cutoff),
**not comparable** to the First TRACTOR Evaluation Report (different case set,
platform and scoring harness). See `targets/tractor/README.md` for what a score means.

## How it works, in plain English

**The problem.** You have C code you want in Rust. An AI model can write the Rust,
but you can't trust it: it may compile and still behave differently. So the question
this project answers is not "can a model translate C?" but **"how do we *know* a
translation is right without reading it?"**

**The idea.** Translate one small piece at a time, and keep the rest of the program
in C. The new Rust piece exposes exactly the same C function names and signatures, so
it can be dropped into the program in place of the C file it replaces. Then run the
old and the new side by side and compare.

There are three moving parts:

1. **The harness** — the `harness` command-line tool in this repo. It is ordinary,
   deterministic code: it reads the C, works out which files depend on which, decides
   a safe order to migrate them, asks a model for a translation, and keeps records.
   It never *judges* whether a translation is correct.
2. **The oracle** — the judge. For one piece ("unit") it:
   - builds a small test program twice — once linked with the original C, once with
     the new Rust — feeds both ~500 identical inputs, and requires the outputs to
     match **byte for byte**;
   - builds the *whole* real program both ways (all C vs. C-with-the-Rust-piece) and
     requires identical output on sample files;
   - checks the Rust library exports *only* the functions it is supposed to (so it
     can't cheat by replacing `printf`), and runs the C side under memory checkers.

   All green → the unit is **verified**. Anything else → **red**. That is the only
   definition of "done" — nobody's opinion, including the model's, counts.
3. **The ledger** — a folder of plain text files inside the target project
   (`targets/zopfli/migration/`). Everything the harness knows lives there: the facts
   it scanned, the plan, every verdict, every attempt. Nothing important lives in a
   chat window, so anyone (or any AI agent) can pick the work up cold by reading it.
   Verdicts record fingerprints (hashes) of exactly what was tested, so "verified"
   can't silently go stale — if the code changes, `harness state status` says so.

**The flow:**

```
scan ──▶ plan ──▶ detect ──▶ observe ──▶ migrate ──▶ verify
read C   order    flag risky   AI reviews   AI writes    the oracle
         units    C patterns   the flags    Rust + tests  judges it
```

**Why it's safe to run.** Model-written code is treated as hostile: it is compiled and
run inside a sandbox (no network, can't read your home folder, time-limited), and the
project being migrated can't choose where your API key is sent — that lives in *your*
config, not the project's.

## Quick start (5 minutes, no AI or API key needed)

You need Rust and a C compiler (on a Mac: `xcode-select --install`). From the repo
root:

**1. Install the tool**
```bash
cargo install --path crates/harness-cli
```

**2. See the state of the migration** — what's verified, what's pending, is anything stale
```bash
harness state status --target targets/zopfli
```

**3. Run the judge on the piece that's already migrated** — expect eight `PASS` lines and `GREEN`
```bash
harness verify u001-katajainen --target targets/zopfli
```

**4. Watch it catch a bug.** Open
`targets/zopfli/migration/units/u001-katajainen/katajainen_rs/src/lib.rs`, find
`bitlengths[leaves[0].count as usize] = 1;` and change the `1` to `2`. Run step 3
again: `differential-driver` now `FAIL`s, the verdict is `RED`, and the unit is demoted
from `verified`. Undo everything with:
```bash
git checkout targets/zopfli
```

**5. See the plan and the risk report**
```bash
harness plan --target targets/zopfli
```
then open `targets/zopfli/migration/observer/observations.md`.

### Trying the AI part

**With a free local model** ([Ollama](https://ollama.com)). In one terminal run
`ollama serve`; in another:
```bash
export RUHARNESS_PROVIDERS=$PWD/providers.example.toml
```
```bash
harness migrate u001-katajainen --target targets/zopfli --provider ollama-openai --model llama3.2-1b-32k --retry
```
You'll see each turn (`translate`, then `repair`s) and a final outcome. A tiny model
will fail — that's the point: the oracle catches it and the attempt is recorded under
`migration/units/u001-katajainen/attempts/`. (The example profile file explains how to
create the `llama3.2-1b-32k` model.)

**With no model at all** — the harness writes the prompt to a file and waits:
```bash
harness migrate u001-katajainen --target targets/zopfli --model my-test
```
Answer it by creating the matching `….response.json` next to the `….request.json` it
names, then run the same command again.

### What the flags mean

| Flag | Meaning |
|---|---|
| `--target DIR` | Which project to work on (the folder containing `harness.toml`). Always `targets/zopfli` here. |
| `u001-katajainen` | The *unit* — one piece of the plan. Ids are listed by `harness state status`. |
| `--provider NAME` | Which AI backend to use: `external` (file hand-off, the default), `replay`, `anthropic` (needs `ANTHROPIC_API_KEY`), or a profile from your providers file such as `ollama-openai`. |
| `--model NAME` | The model name sent to that backend. |
| `--retry` | Finished attempts are never overwritten; this records a *new* sample instead. |
| `--promote` | Replace an already-verified unit's Rust with a new green candidate. |
| `--no-promote` | Record a green attempt without promoting it; `harness promote <unit> <attempt> [--replace]` promotes it later, explicitly (`[llm.migrate] promote_on_green = false` makes that the only path). |
| `--attempt ID` | With `--provider replay`: which recorded attempt to re-check. |
| `--allow-unsandboxed` | Only needed where no sandbox exists (e.g. Linux): accept running untrusted code unconfined. |

Exit codes, for scripting: `0` success/green · `1` the harness refused or errored ·
`2` bad command line · `10` the oracle said red.

## Command reference

Prerequisites: stable Rust (pinned via `rust-toolchain.toml`) and a C compiler
(clang with ASan/UBSan; developed on macOS, CI also runs Ubuntu).

```bash
cargo run -p harness-cli -- scan --target targets/zopfli
```
Parses the C sources (tree-sitter), writes the canonical fact model to
`migration/facts.jsonl`: files with include edges, symbols with canonical ids and
signatures, call refs.

```bash
cargo run -p harness-cli -- plan --target targets/zopfli
```
Clusters files into migration units (dependency cycles collapse into one unit),
computes each unit's `source_hash` over its include closure, and **reconciles**
`migration/plan.toml` — statuses, human comments, and unknown fields survive every
replan; execution order is re-derived from `depends_on`, never trusted from block
order.

```bash
cargo run -p harness-cli -- verify u001-katajainen --target targets/zopfli
```
Refuses if the unit's source changed since planning (re-plan first). Otherwise runs
the unit's oracle — for `c-abi-differential`: a differential driver linked against
original C vs the Rust staticlib over ~500 deterministic cases (byte-compared),
the whole program built all-C vs mixed C/Rust (gzip output byte-compared over three
samples), and an ASan/UBSan run — then writes a **content-bound verdict**
(`oracle-latest.json`, digests of everything tested) and updates the plan status.
Red demotes `verified → in-progress` and preserves `oracle-last-green.json`.

```bash
cargo run -p harness-cli -- state status --target targets/zopfli
```
The staleness detector: facts vs tree, every unit's plan hash vs tree, every
verdict's input digests vs tree, and status/evidence contradictions.

**The observer (M2):**

```bash
cargo run -p harness-cli -- detect --target targets/zopfli
```
Runs the deterministic gotcha detectors (tree-sitter, 8 taxonomy categories:
macros, function pointers, unions/bitfields, setjmp/signals/threads, variadics,
mutable globals, allocator ownership) and writes content-keyed findings bound to
the facts they were computed from. Hazards the suite *cannot* see (pointer
arithmetic, type punning, aliasing) are carried as standing caveats; oracle- and
human-discovered hazards live in `annotations.jsonl` (M0's two real findings are
the founding entries).

```bash
cargo run -p harness-cli -- observe --target targets/zopfli
```
The LLM triage pass: findings are batched per unit and adjudicated
(confirm/dismiss/uncertain with rationale) under a hardened prompt contract —
nonce-delimited untrusted code slices, no source text in the trusted region,
harness-computed content hashes binding every verdict to exactly what was
serialized. Verdicts land in `triage.jsonl`; `observations.md` renders units
ranked by a deterministic risk score. Provider is configurable (`[llm]` in
harness.toml): `anthropic` (live, key from env), `replay` (recorded traces), or
`external` — the harness writes request files and any capable agent runtime
supplies the responses, which is also how triage runs without an API key.
Dismissals never delete: they keep full risk weight until a human upholds them
via `harness review <finding> --uphold-dismiss`, and human-mandatory categories
stay flagged until reviewed.

```bash
cargo run -p harness-cli -- migrate u001-katajainen --target targets/zopfli
```
**The executor (M3).** Builds a translation prompt (ABI contract, confirmed hazards,
nonce-delimited C source), asks the configured provider for exactly two files —
`src/logic.rs` (100% safe Rust) and `src/ffi.rs` (the C-ABI shim) — and drops them
into a **harness-owned** crate scaffold whose `lib.rs` makes the *compiler* confine
`unsafe` to the shim. The candidate then faces the full oracle; failures feed up to
three stateless repair turns carrying bounded evidence (rustc errors, differing
cases). Every attempt is recorded under `units/<id>/attempts/<id>/` with a
content-derived id, per-turn results and token usage, the candidate source, and a
`prompt_digest` — equal digests across attempts prove the same migration was posed
to different providers. Green candidates are promoted through a crash-safe
two-rename protocol and re-verified in place. `--provider replay` re-runs a recorded
attempt from its traces and *verifies* it against the ledger.

Model-written code is untrusted: every build and run happens under `sandbox-exec`
(no network, no reads of your home directory, writes confined to the build dir,
scrubbed environment, timeouts with process-group kill), and a **symbol-set check**
rejects candidates that export anything beyond the unit's symbols or smuggle in
pre-main constructors — so a candidate cannot forge a green by shadowing `printf` or
exiting before `main`. On platforms without a sandbox, `verify` and `migrate` refuse
unless you pass `--allow-unsandboxed`.

**Providers.** A target's `harness.toml` can only *name* a provider profile and a
model — endpoints and credentials live in user-level config
(`$RUHARNESS_PROVIDERS`, see [providers.example.toml](providers.example.toml)), so a
hostile target can't point your API key at its own server. Built in: `external`
(file hand-off to any agent runtime), `replay`, `anthropic`. Wire adapters:
Anthropic Messages and OpenAI-compatible Chat Completions (OpenAI, Ollama,
llama.cpp, vLLM, LM Studio, Groq, OpenRouter, …). Fully local, no API key:

```bash
export RUHARNESS_PROVIDERS=$PWD/providers.example.toml
cargo run -p harness-cli -- migrate u001-katajainen --target targets/zopfli --provider ollama-openai --model llama3.2-1b-32k
```

```bash
cargo run -p harness-cli -- sync-runtime --target targets/zopfli
```
Regenerates the managed block in the target's `AGENTS.md` (current state, next
units by risk, the exact commands) with a content hash; `--check` is CI-usable.
The ledger stays the single source of truth — the view is always derived.

```bash
cargo run -p harness-cli -- gen-driver u-lib --target targets/tractor/cases/Public-Tests/B01_organic/rev16_lib --model claude-sonnet-5
```
Asks a model for the unit's **differential driver** (a C `main` that calls every
exported function with deterministic inputs and prints everything), then validates it
against the ORIGINAL C only: strict build, `driver-shape` (it may define only `main`
and call only the unit and allowlisted libc), every symbol called, three identical
runs, `-O0` == `-O2`, ASan/UBSan, and **mutation adequacy** — deliberately broken
copies of the C must change its output (provably-equivalent mutants are discarded by
Trivial Compiler Equivalence). A green driver is promoted to
`migration/units/<id>/driver.c` with `driver-validation.json`; `migrate` then refuses a
unit whose generated driver is not freshly validated.

```bash
cargo run -p harness-cli -- bench status --suite targets/tractor
```
The benchmark suite: `bench vendor --from <checkout>` (pinned, checksummed copy of the
corpus), `bench verify-corpus`, `bench init` (every case becomes a harness target),
`bench status` (per-case pipeline progress), `bench score [--case …] [--write]`
(scores oracle-verified Rust on the corpus's held-out vectors with the corpus's own
runners, sandboxed; `--write` re-verifies everything first and records `scores.json`),
`bench boundary [--case …]` (design B calibration: the boundary check alone on every
verified unit, written nowhere — docs/ORACLE-HARDENING.md §B.7),
and `bench check [--replay]` (the regression suite: re-verify, re-validate, re-score,
compare per vector — exit 10 on a regression). `--replay` also re-judges every recorded
model trajectory from its stored evidence (zero tokens): prompt edits do not break it —
they show up as `prompt: drifted`, while any change in how the harness judges the recorded
replies fails the check (docs/REPLAY-DESIGN.md). See `targets/tractor/README.md`.

Exit codes (stable contract): `0` ok/green · `1` harness error · `2` usage ·
`10` oracle red (for `migrate`: red, blocked, truncated, or format). Machine consumers read the ledger files, not stdout.
With the global `--json` flag (`harness --json migrate …`) stdout is instead a
newline-delimited stream of `ruharness-events` (docs/SCHEMAS.md "CLI hardening") for
a consumer such as the review cockpit; human logs stay on stderr. Ctrl-C kills every
live sandboxed process group and the harness dies by the signal (no evidence is
journaled for a child it killed). Writing commands hold a writer lock on
`migration/.lock`; a second writer fails fast naming the holder.

## Repository layout

```
crates/
  harness-core/     # fact model, schemas, plan, verdicts, observer, risk, planner, traits
  harness-scan/     # C frontend (tree-sitter): facts, mutation sites, driver lint
  harness-detect/   # built-in hazard detectors (c-treesitter-v1 suite)
  harness-llm/      # provider profiles + adapters (anthropic, openai-compat,
                    #   replay/external), triage pass, and the shared trajectory
                    #   engine behind migrate + driver generation
  harness-oracle/   # c-abi-differential OracleStrategy: sandbox + run confinement,
                    #   symbol-set/capabilities/driver-shape gates, validate_driver,
                    #   held-out benchmark scorer
  harness-cli/      # the `harness` binary
docs/SCHEMAS.md     # normative ledger schemas, v1
targets/tractor/    # TRACTOR B01 library suite: suite.toml, corpus.lock, cases/ (one
                    #   harness target per case), heldout/ (vectors + corpus scorer,
                    #   never inside a target root), scores.json, handoff-tools/
targets/zopfli/     # vendored migration target (pinned)
  harness.toml      #   target config
  AGENTS.md         #   generated runtime view (managed block; `harness sync-runtime`)
  migration/        #   THE LEDGER: facts.jsonl, plan.toml, units/<id>/ (contract,
                    #   driver, Rust crate, content-bound verdicts), DECISIONS.md,
                    #   observer/ (findings, annotations, triage, reviews,
                    #   observations.md; traces/ gitignored)
DECISIONS.md        # engineering log: spikes, decisions, milestone handoffs
```

Unit crates under `targets/` are deliberately **not** workspace members — the
oracle builds them via `--manifest-path`, so a broken in-progress unit can never
brick the harness's own tooling on a fresh clone.

## Working on the harness

CI (GitHub Actions, macOS + Ubuntu) enforces: `cargo fmt --check`, `cargo clippy
--all-targets -- -D warnings`, `cargo test --workspace` (includes an end-to-end
pipeline test that migrates-and-verifies u001 in a temp copy), and `cargo deny`
(advisories, license allowlist, source policy). `Cargo.lock` is committed.

Dependency policy is tight (§11 of the project briefing): every addition is
justified in `DECISIONS.md`. Current tree: serde/serde_json, toml/toml_edit,
blake3, thiserror, tree-sitter (+C grammar), clap, anyhow.

## License

Harness crates: MIT OR Apache-2.0. `targets/zopfli/` is Google's zopfli, Apache-2.0
(see its `COPYING`); `katajainen_rs` is a derivative of it and stays Apache-2.0.
