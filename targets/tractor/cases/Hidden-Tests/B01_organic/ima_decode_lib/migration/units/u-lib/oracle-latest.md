# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:430cd45acf0d65d376f095e8d497a9a7e6745d0c6ef4d657eb4d510270dfdc07`
- rust_crate: `blake3:6070c45f7f4e6c64e1a4928f06883ef267668a44f677416fce9ff08a99c117d0`
- driver: `blake3:beb99110d1c1b4def31784e618d953766b2aeacdc1be033387232f2146e9b482`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6821 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
