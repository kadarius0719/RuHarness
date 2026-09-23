# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:c4ec5a1d32f4b5a13f4c6b8660c680225e8b8aecd5084fdb1a1b767dfce481bc`
- rust_crate: `blake3:1d0aad9ebd93287ee233f8075abfeb94134155576e6730fde53b4cde68c63588`
- driver: `blake3:169748fb94afdeda11a5d609c7427a16f113689ecc32f8375fdb863493a9146e`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 17134 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
