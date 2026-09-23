# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:0dc448e0a4ed2254b6cc1873921d4bfe0ceef064f422158c11d0618900956f31`
- rust_crate: `blake3:821a1f9233632e5343ac51b553b53b74ce3c977000af8d7a44d7ef1402a6780e`
- driver: `blake3:f9a961134a1e76dd54208d6ce19faa2863326177af69dee170c996843f6f61e5`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 19521 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
