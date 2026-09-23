# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:9474c1b95801a5d931ccb25bf9c6cf76931c6df64c47174b17105dce86728221`
- rust_crate: `blake3:e86b5bc1a0946bc438b15f9dd182fd3dbdf1f66bf8c9eb4489c2c0e50aa61758`
- driver: `blake3:bf6ee47ef54f77d52c65981d14cc2a51493dff93eea8bf2b3dc6a0f14a388b04`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 12021 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
