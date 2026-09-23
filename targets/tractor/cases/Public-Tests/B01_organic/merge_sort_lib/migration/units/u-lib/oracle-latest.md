# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:b40e7c9ea622f12bac0c314901e6c7a1a30721b8c296454f5a188d2d4a604273`
- rust_crate: `blake3:35b04c1fe4fb4dbf23438d415a43f4639137cf583d0f077d2a22efe1d2b91441`
- driver: `blake3:1a82b88d0b4e6ae4378d6b695f0a8381b840f8d0b8cda0815213ad388438fb78`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 28306 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
