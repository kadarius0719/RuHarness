# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:59f1f65ceba55d94abd05df0796d5fe47650ebbfdc1cc0a074fa78b86590c060`
- rust_crate: `blake3:154b54fe9c70212c4993188e6c4a916522a53f04b1450b38fadd6fc4c6e038f7`
- driver: `blake3:3793806fc86bf650cc0ca53064c6e4edc931df2ff7d4ef1e813be404a03f659a`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 10777 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
