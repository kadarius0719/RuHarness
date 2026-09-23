# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:9690d398d8b84db2e0534da2adcda4873ff4688085ac118267648c77d64a22ba`
- rust_crate: `blake3:88ebd228ab1c7f728cd4811e42d21783fd9b5c691f2b34ff4e1b530265a9532a`
- driver: `blake3:09f784a28e146861d5971142e1c26d9984192f07f8fef88dac646da488609ef3`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 14641 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
