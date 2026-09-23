# TRACTOR B01 library suite (`tractor-b01-lib`)

A benchmark suite for RuHarness built from DARPA TRACTOR's public test corpus
(`github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus`, MIT, "Distribution A"),
pinned to tag **v2**, commit `37960ee08a7c644ee66e29855eca33903f702e67`.
Design and rationale: `docs/M4-DESIGN.md` (§R is authoritative), `DECISIONS.md` (M4).

## Layout

| Path | What | Owner |
|---|---|---|
| `suite.toml` | upstream pin, batteries, the DERIVED case list (`library`/`symbol`/`runner` per case), exclusions | `harness bench vendor` |
| `corpus.lock` | blake3 of every vendored file (+ the harness-authored scorer files) | `harness bench vendor` |
| `cases/<upstream path>/` | one harness **target** per case: upstream `test_case/` + `harness.toml` + `migration/` ledger | upstream / `bench init` / the pipeline |
| `heldout/<upstream path>/{test_vectors,runner}` | held-out scoring material — never inside a target root, never in a prompt | upstream |
| `heldout/tools/cando2/` | the corpus's own scorer library | upstream |
| `heldout/Cargo.toml`, `heldout/Cargo.lock` | harness-authored scorer workspace + dependency pin (hash-locked) | harness |
| `heldout/patches/` | one portability patch to a scorer *dependency* (see `PATCHES.md`) | harness |
| `scores.json` | the committed scores — the regression baseline | `harness bench score --write` |
| `.scorer-vendor/`, `.bench/` | vendored crates, scorer builds, snapshots (gitignored) | — |

Suite = the 100 Battery-01 **library** cases: 80 `Public-Tests` (split `public`)
and 20 `Hidden-Tests` released at v1 (split `hidden`). Executables, B02, P00–P02
are out of scope for M4 (see DECISIONS.md).

## Reproducing from a cold checkout

```bash
harness bench verify-corpus --suite targets/tractor
```
One-time, networked (the ONLY networked step): vendor the scorer's crates.io
dependencies. Cargo later verifies every vendored file against the committed
`heldout/Cargo.lock` checksums.
```bash
cd targets/tractor/heldout && CARGO_HOME=$(mktemp -d) cargo vendor --locked ../.scorer-vendor
```
Then (offline, sandboxed):
```bash
harness bench status --suite targets/tractor
```
```bash
harness bench score --suite targets/tractor
```
```bash
harness bench check --suite targets/tractor
```

## Re-vendoring (only when changing the pin)

```bash
git clone --filter=blob:none --no-checkout https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus.git /tmp/tractor
```
```bash
git -C /tmp/tractor checkout --detach <commit>
```
Update `[upstream]` in `suite.toml`, then:
```bash
harness bench vendor --suite targets/tractor --from /tmp/tractor
```
`bench vendor` refuses a checkout whose detached `HEAD` is not the pinned
commit and never overwrites a vendored file with different bytes.

## What a score means (and does not)

- Scored with the corpus's own `cando2` runners, per vector, on **macOS arm64**
  (C baseline `cc -shared -fPIC -O0 -ffp-contract=off`). Not the official
  Nix/Docker/Falco harness, not Linux.
- Headline: **per-case strict pass** (every non-UB vector passes) over
  scorable cases, per split. NOT comparable to the First TRACTOR Evaluation
  Report's 150-test rates (which include executables, hidden tests, Linux).
- The vectors are public since Feb 2026 (before the answering models' training
  cutoff): a **public-vector score**, not a contamination-free held-out measure.
- **Unmarked-UB vectors are excused, with disclosure.** A vector the C passes but
  the Rust fails is re-run against the C built with `-fbounds-safety` + ASan; if the
  C itself is proven memory-unsafe on it, the vector is `unmarked-ub` — excluded like
  `has_ub`, counted in `vectors_unmarked_ub`, and listed per vector in `scores.json`
  (`c_sanitized`). A vector the plain C fails is never excused. See
  `docs/ORACLE-HARDENING.md` §A.
