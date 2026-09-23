# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:5258d31843490e86ba0488fe9b027bdc60a7cd8d7c4e7e6183b9ffa7766e0afa`
- rust_crate: `blake3:3792c5e9e4628ff754fa7ba81ba440c60580e77730e2464ce35d9b8470e1e09b`
- driver: `blake3:580168aa46519d60cad8de66d3f634c26e3cde5e33156a48508b052dc06cab09`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 14431 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
