# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e2a175dcf54a85d3a3d2b66110e0380711a1a9d821f01f09c5f2410203afedd2`
- rust_crate: `blake3:c0fe376f2cc857c13bdf38ff1eb663f49e6914e2a33d84470f8eea5a220e4bf1`
- driver: `blake3:b7d1eb707e854c2fb63e252d141f232b7a5709f7ba0b6e661ae1e27aaccce46c`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 9779 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
