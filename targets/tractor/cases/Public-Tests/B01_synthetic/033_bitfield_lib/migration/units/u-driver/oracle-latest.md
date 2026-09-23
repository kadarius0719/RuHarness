# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:ffa761a2e9374ea7a97043e9993511977a848e2d2aca62da5d7640c15d8afbed`
- rust_crate: `blake3:5fbe65678ffa2c8cd7dbb3b9aebf6d023997e2f66e68f66712b01e9a194bbe5d`
- driver: `blake3:332a00af745d78bd56488d3367b9612749713d80767cb98c166ce8470e8ea058`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1983 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
