# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:afcdb741591f96d0f31db8e2189efcbefa13d0c44f83a47fd36a18799b063a7f`
- rust_crate: `blake3:4ef050bd988cc32ae6749fb71108addb2f726cffdd0b9ee3df2e970ebdd7d5cb`
- driver: `blake3:2254a0e31708edae72c11803a74d2fab154b5674a17360850a5635148f92a076`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 4 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 518 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
