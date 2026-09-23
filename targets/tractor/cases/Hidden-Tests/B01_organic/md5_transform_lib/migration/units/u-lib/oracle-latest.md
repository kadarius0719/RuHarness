# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:5ff8bee91a8cb8c9b9fbcc798fdfcd71851a49ada8516a85f54b26d5f122da81`
- rust_crate: `blake3:d3038dc1306b944e79cf0c5bc2cca5fdead9ab1fa04c8707454df4c1957c452f`
- driver: `blake3:e4cec58597cfa2d579f732b669d44135f751ad6b1461ec18577249bc86c0d984`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 2971 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
