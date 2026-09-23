# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:83601a730fca78ea3369dbd6daeb3b764e278c9a347e541259e0c16c370cf89a`
- rust_crate: `blake3:a3b0adf7c94371698ff6c593ecf1386a8bb2bc86108a62181cec523b5cee9ed2`
- driver: `blake3:3100b2ac24adce72523f41f87e2069aee373453a146efd2313924207458e201b`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 10 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 8155 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
