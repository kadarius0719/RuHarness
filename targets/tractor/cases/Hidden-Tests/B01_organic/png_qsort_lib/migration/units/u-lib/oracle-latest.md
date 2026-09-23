# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:096b0f6c0f6d3fdbdabcd59337ed510451120b03b3f0a0b6990221b6b0eab553`
- rust_crate: `blake3:c7d82c8927bab2cf6607df930c3fd5325c9f23a31c52c34ade579d0c563e1ebe`
- driver: `blake3:1ee8bd763b1b635eabe0aef510810c2e01ef32992082f01cd86260282b4b4457`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 12542 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
