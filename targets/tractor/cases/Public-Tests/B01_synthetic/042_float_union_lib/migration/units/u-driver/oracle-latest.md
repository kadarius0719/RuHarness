# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:0c73b8904121fcc561c41ada263115bbae1470c98c0324629263a47b7860b921`
- rust_crate: `blake3:7d6a8de9d0f3be0f4325d1536241fb01e2e0b33b6b27b28ec1ac657637119ce2`
- driver: `blake3:de2dfdf41417585b1829fd24a7fc7d51fca88634af0ac41bc8c822994cf11bb7`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 11029 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
