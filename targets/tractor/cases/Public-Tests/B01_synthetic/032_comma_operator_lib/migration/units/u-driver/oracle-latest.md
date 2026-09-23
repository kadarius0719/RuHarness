# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:388da6c7aa0e9e592b681c4196cc77f6b5e43349c4583de9d531fc8c36b72d4f`
- rust_crate: `blake3:a716d54588b490d58d3762cee09b1bc43e68c0f668c99120615525034fa54aee`
- driver: `blake3:f640cc4aa51e3e0e3574ae2f504d37373fee019c3cd88464645070595168b9cc`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 9275 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
