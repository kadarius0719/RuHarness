# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:1f6657a6c54eecf8b18fa9df856f5e235bc8171415468c50fc2271acc330e1f6`
- rust_crate: `blake3:7d87edc3d8fe0c776a8611d4cdc17bdef04aeed8364ec11bea097307c8706beb`
- driver: `blake3:a0030a4760e6c5abcb91077ac6ac3aec70c4c79427b256178b64b35cd890c364`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 13699 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
