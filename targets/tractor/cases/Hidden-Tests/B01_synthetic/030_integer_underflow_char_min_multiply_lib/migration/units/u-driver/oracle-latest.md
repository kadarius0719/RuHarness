# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:2b94231df914e202333a0039dd54cc93835dd33a98a525cd0d012843da2a7425`
- rust_crate: `blake3:7da82dd81e6e5ee3126b207f2e979da216ff35a3e1d1fba2b5a5bc89c137695c`
- driver: `blake3:f08a90c051ae55c690d707ba43c3657ecc0712d5100686ce20d09ed80b41d693`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 14433 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
