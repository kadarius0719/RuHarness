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
| **M4** — benchmark | DARPA TRACTOR public corpus scores as regression suite | next |
| **M5** — extension proof | External detector plugin + `EXTENDING.md` | — |
| **M6+** — Phase 2 spike | Second language frontend, golden-test oracle | — |

The working target is a vendored [zopfli](https://github.com/google/zopfli) (pinned
commit in `DECISIONS.md`). Its plan currently holds 11 units in dependency order;
`u001-katajainen` (length-limited Huffman codes) is migrated to safe Rust behind the
identical C ABI and oracle-verified, with zero human-written Rust.

## Usage

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

Exit codes (stable contract): `0` ok/green · `1` harness error · `2` usage ·
`10` oracle red (for `migrate`: red, blocked, truncated, or format). Machine consumers read the ledger files, not stdout.

## Repository layout

```
crates/
  harness-core/     # fact model, schemas, plan, verdicts, observer, risk, planner, traits
  harness-scan/     # C frontend (tree-sitter) implementing LanguageFrontend
  harness-detect/   # built-in hazard detectors (c-treesitter-v1 suite)
  harness-llm/      # provider profiles + adapters (anthropic, openai-compat,
                    #   replay/external), triage pass, the migrate executor
  harness-oracle/   # c-abi-differential OracleStrategy: sandbox, symbol-set check
  harness-cli/      # the `harness` binary
docs/SCHEMAS.md     # normative ledger schemas, v1
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
