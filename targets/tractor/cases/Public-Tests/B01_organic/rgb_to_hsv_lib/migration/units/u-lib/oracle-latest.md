# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:1a6105c3c27b839bad526cae8ffdcb1f77e1bf51d4459ecb345a7dc42aa08252`
- rust_crate: `blake3:c62ff0ac33124b5b7cdb403391ebeb55c43196c439fcb82450ca952ae5c337c1`
- driver: `blake3:ba3ecc33338d0f17f190211817f809940196398d74db1c16464416f9ab7667fc`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 23818 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
