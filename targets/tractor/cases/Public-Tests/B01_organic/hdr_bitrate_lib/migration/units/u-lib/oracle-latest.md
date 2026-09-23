# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:a65d63b7716304a8f794f55037efc8201171633227d298b3b1d6c51ad5abc805`
- rust_crate: `blake3:dd0d57b3e7d9c51e8a63e2a4cc563c44f412e745d0dca1f41905706b7f993156`
- driver: `blake3:12b848e532a6aa353d19caf9d41e6e5e3e779454c5fcdd01d8492547fb235e3c`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 10390 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
