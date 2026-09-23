# Oracle verdict — u-staticloop

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e2d8bf1f4a16129f6b4304180471bc2cdc48fe432814a0919b85d8bb915c4752`
- rust_crate: `blake3:eec0d6019d4c8784122ebfce7ccabfe0b1053642a18f9a06b2fc93ec1c7e606c`
- driver: `blake3:2621533105634ed47ce3233514b21aca3b0940a8caa7910675770904eab34238`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1233 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
