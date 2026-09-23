# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:0a2eff480d034efc6a2ef4ad585128d8a358b5769f3436abdc9d3677c35086e3`
- rust_crate: `blake3:6b459880f07375908d6d2c4f5deb6f732798b1708c554a083282ac06449dac2b`
- driver: `blake3:395ebf76d731c004c914e3c44944e5f205c57b7f52bb17fd9e484b96d7bc8bf3`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 896 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
