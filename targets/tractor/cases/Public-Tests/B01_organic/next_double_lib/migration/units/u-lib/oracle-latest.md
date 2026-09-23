# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:af811e4a635658f903138bc67285d5695c2ea6f0df6d3331dfac5e76142689ca`
- rust_crate: `blake3:1b7d0c87fb578040d53ae9335dcfc40a2d4df0465eac4fdf867a501ecc858f08`
- driver: `blake3:3d7e5219a67fa5f1973aedfb3ae6deba970f86adc598042b580241893dae2d3a`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 23544 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
