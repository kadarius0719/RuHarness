# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:aae2062b3389912724ae68ddc6c59c49fc428fffbe65161a82cd28e7f375d799`
- rust_crate: `blake3:93a733a6092bf113fbcf7af124c263f77c593ea10127e065db4420e9514ad849`
- driver: `blake3:6bea167d27d43d91a9a18996bfac5ab80a0ebe3c44b0b0c585fe04f1611dcf7c`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 2092 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
