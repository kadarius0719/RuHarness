# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:81c70ab3c9fec60cdf9d8bed2684df2e62a3394522660580c9df0de1d63d944e`
- rust_crate: `blake3:87a799a5028fab4696b2b81a70e52523a57ab318112b648771176ed31337b642`
- driver: `blake3:ed1edcc8bef87b60ea982c445b988d8359963035ea287c5a62e6d757e4707c3c`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 2548 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
