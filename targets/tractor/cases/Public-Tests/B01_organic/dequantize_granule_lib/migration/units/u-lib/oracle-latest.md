# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:dda4400dd40b557deea6a32830e165fe7e997ac43a5fadf2dff066e7f69fd097`
- rust_crate: `blake3:83ee3e8f5f0d2fb5c4bb0177b863c7064a4efcc0d97a4f00f9c489dc3cdff19b`
- driver: `blake3:8543e1a7d7f8e0fce685078f49037519872a9126b17edbb41aaddfd5d23fa9c5`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 78223 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
