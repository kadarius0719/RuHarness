# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:bb4b96fd3063fd61976f71869ec25d9fd10f611dbc6decb75c13fef8965cddcd`
- rust_crate: `blake3:d27165c03011c1ab76c80986893081b5b84fd0ecc06e3165d35ba5dae77411ee`
- driver: `blake3:b725af06690431796f7b2c771efb303ac8da9e9fec4467b19fdf49410c1c7e13`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1896 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
