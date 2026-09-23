# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:aab2f851333b6b05d7c740b46a7c4c59d3a5696c3e0d28b28320cb7f6fd0a12e`
- rust_crate: `blake3:262533e58a1b37d7c85fe89254c523df4cccbb170646c119790d44d9ceecc685`
- driver: `blake3:aa9d125fd8630b7dc39dcfee968e8829d1571d701851a757b1b53271312aafcd`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 4762 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
