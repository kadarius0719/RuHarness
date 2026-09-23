# Oracle verdict — u-pow

Verdict: **RED**

Inputs tested:
- unit_source: `blake3:b2be8029115482b20756a87753c49a6fb3ef7583fc17aee678e6fb51b053a400`
- rust_crate: `blake3:9a320a3d627fa6304f94eb51d483647e6daeea8ac75e81ab0e8e67f2def3e14b`
- driver: `blake3:fa901952b3a5b05e4c15dab3744696d49e976ed158a5d453a4d3bcac2d4161f8`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: FAIL — stdout identical (10707 bytes); stderr differs (lens 4122 vs 0, first diff at byte 0)
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
