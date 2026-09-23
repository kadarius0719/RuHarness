# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:7074e93361f13786ed83ed09b6cda2a3969db4622e19a389b5e7ea68dd46d8db`
- rust_crate: `blake3:5f2435cb1f82c373c44c369ff6fe35547565a23c29660b0b8923413c9fb97278`
- driver: `blake3:45e9ab362befb21e2ad4ef1ad1b528eb7015aba509be3502140fdb5b96ebf3e9`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 52510 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
