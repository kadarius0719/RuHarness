# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:969a223ddf64fc5dba75b64f4afb6de864c5f5c830aa99bcac8a39fc66aaefb2`
- rust_crate: `blake3:07cdd6b80be5893a73cf920a9893b18f902f6f3f429eae803098146cf9e85df0`
- driver: `blake3:96e4089bd47a6691a288ccdddbb29c3d62e6e02dbbc48f3dd5694e1061bfe774`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 3 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 41134 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
