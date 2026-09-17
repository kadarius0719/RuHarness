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
- **State lives on disk, not in conversation.** All pipeline state is checkpointed
  as files in the target repo (the *ledger*). Any agent, from any provider, must be
  able to resume any stage cold by reading it.
- **Deterministic tools measure; agents interpret.** Parsing, graph construction,
  test execution, and diffing are plain code. LLMs rank, explain, plan, and write
  code — nothing a script could do instead.
- **Provider-agnostic by construction.** No pipeline logic couples to any one
  vendor's SDK.
- **Migrate in dependency order, behind an FFI seam.** Leaf units with narrow
  interfaces first, strangler-fig style; the C ABI boundary is the safety net.
- **Unsafe Rust only at the FFI shim.** Interior logic of migrated units is safe
  Rust.

The full architecture is five components — ledger (state), scanner (deterministic
analysis), observer (hazard triage), planner (ordered migration units), and
executor + oracle (the translate/verify loop). The engineering log with every
decision, research spike, and milestone handoff is [DECISIONS.md](DECISIONS.md).

## Status

| Milestone | Scope | Status |
|---|---|---|
| **M0** — end-to-end thread | One C library, one leaf unit, one binary: scan → translate → link → differential test | ✅ done |
| **M1** — ledger + fact schema | `facts.db`, `plan.yaml`, multi-unit ordering, multi-crate workspace | next |
| **M2** — observer | Gotcha detectors (macros, unions, pointer tricks, UB reliance, …) + LLM triage | — |
| **M3** — provider adapter #2 | Same migration through a second LLM provider, zero core changes | — |
| **M4** — benchmark | DARPA TRACTOR public corpus, scores as regression suite | — |
| **M5** — extension proof | External detector plugin + `EXTENDING.md` | — |
| **M6+** — Phase 2 spike | Second language frontend, golden-test oracle | — |

**M0 concretely:** the vendored [zopfli](https://github.com/google/zopfli)
compression library (pinned commit, provenance in `DECISIONS.md`) with its
`katajainen.c` unit — length-limited Huffman code lengths, one public symbol —
migrated to safe Rust (`katajainen_rs`) behind the identical
`extern "C"` ABI, with zero human-written Rust, verified by the oracle.

The oracle already earned its keep at M0 by catching two things code review alone
would likely miss: the C baseline memory-corrupts (SIGBUS) for `maxbits > 15`, an
implicit contract nowhere in its header; and its qsort comparator stops being a
total order for frequencies ≥ 2²² — undefined behavior in the *original C*. Both
are recorded as hazards in the migration ledger.

## Usage

Prerequisites: stable Rust (pinned via `rust-toolchain.toml`) and a C compiler
(`cc`; clang with ASan/UBSan support — developed and tested on macOS).

Scan the target: parse all C sources with tree-sitter, build the function-level
call graph, and rank public *leaf units* (functions whose transitive callees are
only same-file statics and libc — the cheapest safe migration candidates):

```bash
cargo run -p harness-m0 -- scan
```

Run the oracle — the definition of done for the migrated unit:

```bash
cargo run -p harness-m0 -- oracle
```

It performs five checks and prints a PASS/FAIL line for each:

1. **Differential driver** — `migration/units/u001-katajainen/driver.c` is compiled
   twice, linked against the original `katajainen.c` and against the Rust
   staticlib, then run over ~500 deterministic generated cases (fixed-seed PRNG:
   edge cases, error paths, sparse/dense/large distributions). Stdout must be
   byte-identical.
2. **Whole-program (×3 samples)** — the full `zopfli` binary is built all-C and
   mixed (C minus `katajainen.c`, plus the Rust staticlib); gzip output over text,
   random-binary, and empty samples must be byte-identical (zopfli's gzip MTIME is
   hardcoded to 0, so output is deterministic).
3. **Sanitizers** — the C-side driver is rebuilt with ASan+UBSan and must run
   clean.

Exit code is 0 only on a fully green verdict, so the command is CI-usable as-is.
Both commands together: `cargo run -p harness-m0 -- all`. The harness's own tests:
`cargo test --workspace`.

Everything is deterministic and offline: fixed seeds, pinned target sources, no
network. Build artifacts land in `targets/zopfli/migration/build/` (gitignored);
the machine-checked evidence of the last oracle run is committed at
`targets/zopfli/migration/units/u001-katajainen/oracle-latest.md`.

## Repository layout

```
m0/                          # the harness in embryo: scan + oracle binary
                             # (promoted to harness-core/-scan/-oracle/… crates at M1)
targets/zopfli/              # vendored migration target (pinned; .git stripped)
  migration/                 # THE LEDGER — all migration state, on disk
    DECISIONS.md             #   unit choices, behavioral hazards, oracle definition
    units/u001-katajainen/
      contract.md            #   preserved ABI, semantics, domain, done-criteria, status
      driver.c               #   differential test driver (shared by both links)
      oracle-latest.md       #   latest oracle verdict + evidence
      katajainen_rs/         #   the migrated unit: safe Rust core, unsafe only in
                             #   the extern "C" shim
DECISIONS.md                 # harness engineering log + per-session handoff state
```

The ledger is the source of truth. To pick up work cold — human or agent — read
root `DECISIONS.md` (the latest handoff section says what's done and what's next),
then the target's `migration/` directory. Chat history is never authoritative.

## Working on the harness

Quality gates expected to pass before committing (CI to be added at M1):

```bash
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace
```

Dependency policy is deliberately tight (widely-used, permissively-licensed,
minimal transitive footprint; every addition justified in `DECISIONS.md`). Current
non-std dependencies: `tree-sitter` + `tree-sitter-c`, total.

## License

Harness crates: MIT OR Apache-2.0. `targets/zopfli/` is Google's zopfli, Apache-2.0
(see its `COPYING`); `katajainen_rs` is a derivative of it and stays Apache-2.0.
