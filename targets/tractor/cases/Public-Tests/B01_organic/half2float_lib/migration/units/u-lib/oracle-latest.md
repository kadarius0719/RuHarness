# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:64f21638049a47641d06409cfbc3040797f145c8cb44a2da65ca641413361267`
- rust_crate: `blake3:f779537bc3b17c4fce23cc7edde2fc8d652a5db4ea8740d0fdf8c98bae2afe87`
- driver: `blake3:9214ea0523e2c2829db14b38eae799965da4554483224704860195a094180635`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 75172 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
