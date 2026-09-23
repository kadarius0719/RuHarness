# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:53edbd5bde9d872b4e89e1bf357d07f064843d6a7d3eee0fc895e91a3fa57c4a`
- rust_crate: `blake3:223f2b937df2330d9ce449c5b9f2a86d619dbb4a393eabf31db26b18e2c0b5a4`
- driver: `blake3:92c3f7c6bfcfe338c63b5318c8f0f4a0655f269cc6c7d424ad893241f0eed330`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 26795 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
