# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:08d258b9bb12aabe90e6707f026a51aa743543de320df4da3eff1a6147b75354`
- rust_crate: `blake3:77dd43036dc18c809f1d05b95027304e44756333939d89cf7b957ba810404b8b`
- driver: `blake3:f5c725543818667f9ec1eb6585c319707cceec63c89088d1c8fbed3ad2403a13`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 14908 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
