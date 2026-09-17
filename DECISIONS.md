# Harness Engineering Decisions

Running log, newest last. Each entry: decision, rationale, alternatives rejected, revisit-when trigger.

## 2026-09-17 — M0 kickoff: research spike (§15)

Time-boxed landscape check (run in a cheap-tier subagent, sources verified live):

- **google/zopfli**: archived by Google 2025-10-14, read-only, Apache-2.0, last release 1.0.3
  (2019). *Implication:* frozen upstream is a feature for us — a stable, reproducible target.
  https://github.com/google/zopfli
- **tree-sitter**: 0.27.0 (Aug 2026), active. Grammars expose `LANGUAGE: LanguageFn`;
  `set_language(&Language)` takes a reference; callers need `.into()`. tree-sitter-c 0.24.x.
- **c2rust (Immunant)**: actively maintained (v0.21, Oct 2025). Reference point for
  rule-based translation, not a dependency.
- **DARPA TRACTOR public corpus**: available —
  https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus/ (Battery 01; P00 Perlin
  noise, P01 SPHINCS; new battery every 6 months). Target for M4.
- **SOTA systems to re-check at M2+**: Syzygy (dual code+test translation, ICSE LLM4Code
  2025), EvoC2Rust (skeleton-guided, ICSE 2026 SEIP), ORBIT, His2Trans.

**Revisit when:** M2 observer design starts (re-check translation-system literature), M4
benchmark setup (re-check TRACTOR battery status).

## 2026-09-17 — M0 target: Zopfli, unit: katajainen.c

- **Target library:** google/zopfli, vendored at commit
  `ccf9f0588d4a4509cb1040310ec122243e670ee6` (2024-04-11, upstream HEAD at archive time)
  under `targets/zopfli/`. The vendored copy's `.git` was removed so this repo can track
  the migration ledger and Rust units inside the target tree; provenance is the pinned
  hash above (re-fetchable; verify by re-cloning and diffing).
- **Leaf unit:** `katajainen.c` — single public symbol `ZopfliLengthLimitedCodeLengths`,
  callees are only libc (`malloc`/`free`/`qsort`) and same-file statics. Narrowest real
  interface in the codebase; pure-function semantics (no globals, no I/O).
- **Oracle (exact command):** `cargo run -p harness-m0 -- oracle` from the repo root. It:
  1. builds a differential driver (`driver.c`) twice — linked against original
     `katajainen.c` and against the Rust staticlib — runs both over ~500 deterministic
     generated cases, and byte-diffs stdout;
  2. builds the full `zopfli` binary twice (all-C vs. mixed C/Rust with `katajainen.c`
     replaced) and byte-compares compressed output over sample inputs;
  3. additionally runs the C-side driver under ASan+UBSan.
- **M0 scan:** `cargo run -p harness-m0 -- scan` parses `src/zopfli/*.c` with
  tree-sitter, builds a function-level call graph, and ranks leaf units. Known gap
  (recorded deliberately): calls made through function pointers (e.g. the qsort
  comparator) are invisible to naive call extraction — this is exactly gotcha-taxonomy
  material for M2 detectors.

## 2026-09-17 — M0 scope reductions (deliberate, to be lifted at M1+)

- **Sandboxing (§12.2):** M0 runs builds/tests as plain subprocesses (explicit argv, no
  shell, paths confined to the checkout). Containerized/network-disabled sandboxing is
  deferred to M1. Revisit when: first LLM-generated code runs without a human reading it
  first.
- **No `facts.db` yet:** M0 state is ad-hoc files under `targets/zopfli/migration/`.
  M1 promotes to the neutral schema after the §14.1 storage spike.
- **Single-crate-per-purpose workspace shape (§10.1) deferred:** M0 is `harness-m0`
  (one binary) + the unit crate. M1 splits into `harness-core`/`-scan`/etc.

## 2026-09-17 — Dependencies added (§11.1 justifications)

- `tree-sitter` 0.27 + `tree-sitter-c` 0.24: approved baseline (§11.2) parsing path;
  chosen over libclang for M0 because it needs no `compile_commands.json` and has no
  system-library setup cost. Revisit at M1: libclang gives real type/ABI facts that
  tree-sitter cannot; the scanner frontend trait must not assume either.
- No other non-std dependencies in M0. `clap` deliberately skipped (two subcommands,
  plain `std::env::args` suffices at this size).

## 2026-09-17 — M0 complete; state at session end (§17 handoff)

**M0 is done.** `cargo run -p harness-m0 -- oracle` is GREEN (5/5 checks): 500-case
differential driver byte-identical, whole-program zopfli (mixed C/Rust link)
byte-identical gzip on 3 samples, ASan+UBSan clean. Unit u001-katajainen is
`verified`; zero human-written Rust. Quality gates pass: fmt, clippy -D warnings,
cargo test.

Two real findings the oracle produced (details in targets/zopfli/migration/DECISIONS.md):
1. Implicit contract `maxbits ≤ 15` — C baseline SIGBUSes beyond it (`counts[16]`).
2. C qsort comparator is not a total order for frequencies ≥ 2^22 (UB per C11
   7.22.5p4); Rust port uses the true total order, differential domain constrained.

**Next actions (M1):** state-backend research spike (§14.1) → freeze `facts.db` +
`plan.yaml` schemas → promote m0/ into the §10.1 workspace shape
(harness-core/-scan/-detect/-llm/-oracle/-cli) → multi-unit ordering. Also carried
forward from M0: function-pointer calls invisible to the scanner (M2 detector
material); CI for the §10.2/§11.3 gates not yet set up.
