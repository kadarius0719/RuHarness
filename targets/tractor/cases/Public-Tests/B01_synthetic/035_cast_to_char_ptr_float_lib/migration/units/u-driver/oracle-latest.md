# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:2536963d5448e892578817ca9352b3c476ecaeb87f5b66b4bc724bc5c264871f`
- rust_crate: `blake3:748433f6f38c2980682d04b2fb878a4432498e8009422382a0c8bfa4a20b8ca3`
- driver: `blake3:7c89b75c7b72c6df282f0291463b557396a367c061f22f8805721adc0e84ef10`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 2799 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
