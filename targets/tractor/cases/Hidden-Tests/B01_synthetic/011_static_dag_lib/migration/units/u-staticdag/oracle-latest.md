# Oracle verdict — u-staticdag

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:29b96a0cb2833054597344abcc35a423344ba6195e98d99ef59da6d93cee23db`
- rust_crate: `blake3:984ae0a5a808aa81a7083eb774c08078ce3164888416abdcaf8e933907233725`
- driver: `blake3:64d27470691d784a14c5172b79fb07f73c3258ab3a0fc04917caa77959a40982`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 11099 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
