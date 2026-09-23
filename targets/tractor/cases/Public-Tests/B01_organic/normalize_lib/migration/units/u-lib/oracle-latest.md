# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:44a69b9e06439864a5fde1cc4a26152b5050c641eeb31d83d8899a10d5bf4920`
- rust_crate: `blake3:f0f531e1c033388c5d5ad049c0b17303f0b43d25f9c64dbd1e56f4da727f4d5d`
- driver: `blake3:3823b20ba4c7cdae55d0519abba0d4e896818525cbde2d73601b741072547046`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 15914 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
