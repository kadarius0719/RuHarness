# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e6cf3003aba1959402a240870c6c262eea3064628f32d3ce3dda40c001476212`
- rust_crate: `blake3:b19394d335a82083a7baceaf6d7f06fb989d05e000492b8a970fac4cdeeb4f25`
- driver: `blake3:03b8589bd97e50de71f7d6d18de09846823920012b4f264078f851328dd48b2f`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 4130 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
