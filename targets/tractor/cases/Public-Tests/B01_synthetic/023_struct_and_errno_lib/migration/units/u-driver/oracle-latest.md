# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:eaf8225dac1e1c827215912ddfeaaab36b3b787674ee8c1afa40241222daf392`
- rust_crate: `blake3:5fa779cbdbd3b83e241f86197c7ad494214e481576720fb89284d718a1e8733f`
- driver: `blake3:c629818c8a47382dfa69822e889008d5968ccfd6ac35637fe7d6dfe169c1f0e3`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6160 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
