# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:006d3cc1e09b2cc333c8475b4cd0f844796e174116551ecab9a55a520dcb11a3`
- rust_crate: `blake3:5a25f90a60efb859fed19a51a2b8eac90863c2a44a5037757f29ac0b28d04b32`
- driver: `blake3:a2b02f877952d7e65d690457dd55aa58083b8c22e94233ae3019e3bc355a2cd3`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 2771 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
