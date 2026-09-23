# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:98c2e57ae66f31ba394c341fad084a197fbf5d96b558997320c032de9b87561b`
- rust_crate: `blake3:ddc144717c1bd8c5355be3a0a263f010cdbc6403dccdaadbff896c7ada184db2`
- driver: `blake3:c88747a16e863dcc36faa11fe9cf9f5fd883e6661567fabfda9cc2f60b1d486e`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 8620 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
