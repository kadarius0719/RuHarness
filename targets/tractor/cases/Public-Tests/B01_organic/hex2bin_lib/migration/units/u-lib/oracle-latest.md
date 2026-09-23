# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:b1e12f3e641128c8ae093aed6c80f965e838ea05378f76bcda0c98400abe7ca1`
- rust_crate: `blake3:0a7d7659b91ff9df2a0b638c315d8394cb76aaae522ea245285ff3ae6aac184b`
- driver: `blake3:c30cc34778ff1203870250ff11d036c424a4d7792c236b9d622c636b1614f010`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 7011 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
