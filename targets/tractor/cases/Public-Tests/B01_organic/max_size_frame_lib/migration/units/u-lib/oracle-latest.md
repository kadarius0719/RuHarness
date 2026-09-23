# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:d1f6547829462c7f7a7c721501c6c379be20ed9fdc16d91488eed325310e86b1`
- rust_crate: `blake3:603f1803b7ec275ba567c95df20a7756b72b0f51edb23b04a791bf088fb7376e`
- driver: `blake3:7f561fdeac7f48c70eaaa73b96954b15d56014a0864d2935a42fc2c5df08aec0`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 76924 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
