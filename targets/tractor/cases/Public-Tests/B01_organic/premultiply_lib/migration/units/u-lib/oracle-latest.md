# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:0168011a816987c96803c875ed98bc42cc4ab0ecb95cf12eba9a691d9ae14d2f`
- rust_crate: `blake3:a0ebe0180867e368f5280f4cdf661e789bb3b3eab08b4c3d475cd750e92c72f7`
- driver: `blake3:8122d2d3ce72d3bf67de5b60112be9b30e7257e53fbef4e1f34614bac17242f7`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6363 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
