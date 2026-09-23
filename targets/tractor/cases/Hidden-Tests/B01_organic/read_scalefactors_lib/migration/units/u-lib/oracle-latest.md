# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:fd0540b200009a06059b084eacaeec0d1fc6278ce0274d3ff5a119836da8deb0`
- rust_crate: `blake3:d3dcc4921246a88469dc34d5570b1dbde30702c65f0a483dd1b66979a4cd135a`
- driver: `blake3:b2544df11d7df6bd265ac4be3f30210309cee3432d8d78fa8ca87cb61f1e1263`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 11503 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
