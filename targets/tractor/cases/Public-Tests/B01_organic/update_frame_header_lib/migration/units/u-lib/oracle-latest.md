# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:7abf119d4889f54da3e7188b130ccd3fe98ff0e0cc62495e23c358a8ca96ee00`
- rust_crate: `blake3:5743c6badf35318542dc4fa3e2acbe04f28236444dcbe3dbb5917fd05f507fd8`
- driver: `blake3:44da103f6a0e2c9ee53fa311c79e0db5e8726ffa589b34c477c85304b33987ee`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 23981 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
