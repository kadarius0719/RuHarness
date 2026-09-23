# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:fbb7ae3b47bfa561bbbe2b7a7856da190b5c60431136ee4e5278ef00024521cf`
- rust_crate: `blake3:5c5e34152dfbcd8d8da7925b07ba0405662ba5d7606fda943add790e802b7e20`
- driver: `blake3:987f058c1dc70a151e75aaf6b04584d0ecbab85b0417fb525030bff3d79638be`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 68657 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
