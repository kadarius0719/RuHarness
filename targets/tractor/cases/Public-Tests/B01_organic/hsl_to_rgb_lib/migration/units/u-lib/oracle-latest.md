# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e04fc46efddadf1ad08f8b50f658746b2c9bc59ea3cc139ced0d45a843b14c06`
- rust_crate: `blake3:f553ec92beedd9ce89cf0226494ad9fd22cbad6337edc7ec817473d60da93f97`
- driver: `blake3:009f3e4e8953be2a59237aee7a0259dbfc317a8be32e42491b75530f3922a5da`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6512 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
