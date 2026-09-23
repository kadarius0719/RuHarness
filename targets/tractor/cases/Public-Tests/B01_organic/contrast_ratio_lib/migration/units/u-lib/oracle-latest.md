# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e27a6eab0b4cacf7fb4c7a69660575d57fc404ca2877e52090e0ce8a3bda29b8`
- rust_crate: `blake3:599a882153489358ee7912ae6fd9e5b8c07149f561aad88d35fdadf2798d2225`
- driver: `blake3:f831d315efd6812e604414f2a3aa94aaba1154766f6b14774c23c10819f9ac3a`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 3926 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
