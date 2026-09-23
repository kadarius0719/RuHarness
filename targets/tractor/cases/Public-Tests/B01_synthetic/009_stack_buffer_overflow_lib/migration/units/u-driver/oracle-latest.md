# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:91d71462bf3ee5a9e7a18aea74b98596c564aa93a1c67d0dcb20fb8c4bba9123`
- rust_crate: `blake3:57d9617ca2fe16818b434ca48f95ac061e8f7a37e6df08e725f86c1e6ae390af`
- driver: `blake3:1b038285c376a3b37cf81191488bb9f1bb614096764feb84ebfedc830d1e241f`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 3661 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
