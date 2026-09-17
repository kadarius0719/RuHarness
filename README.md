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
| **M2** — observer | Gotcha detectors + LLM triage; runtime-view sync | next |
| **M3** — provider adapter #2 | Same migration through a second LLM provider | — |
| **M4** — benchmark | DARPA TRACTOR public corpus scores as regression suite | — |
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

Exit codes (stable contract): `0` ok/green · `1` harness error · `2` usage ·
`10` oracle red. Machine consumers read the ledger files, not stdout.

## Repository layout

```
crates/
  harness-core/     # fact model, schemas, plan, verdicts, planner, traits (forbid unsafe, deny missing_docs)
  harness-scan/     # C frontend (tree-sitter) implementing LanguageFrontend
  harness-oracle/   # c-abi-differential OracleStrategy
  harness-cli/      # the `harness` binary
docs/SCHEMAS.md     # normative ledger schemas, v1
targets/zopfli/     # vendored migration target (pinned)
  harness.toml      #   target config
  migration/        #   THE LEDGER: facts.jsonl, plan.toml, units/<id>/ (contract,
                    #   driver, Rust crate, content-bound verdicts), DECISIONS.md
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
