# Oracle verdict — u-simplestruct

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:f2f24bde177c2285ebc5370494882f8042ed6e067b580bd32c9eff1dd0e128a2`
- rust_crate: `blake3:cf3cc4b10098e9d911b9118b79e3d4a4985616b4a3b2faa6081a3e828fb4ca41`
- driver: `blake3:43f5eb7b58ca53d7fa574a293a445dce2346dbf0769e9634b4c3d5566556fc17`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 174 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
