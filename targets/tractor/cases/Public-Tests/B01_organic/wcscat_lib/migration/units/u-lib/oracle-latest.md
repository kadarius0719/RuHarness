# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:3a095e7f1434bdbb5e16f179474d4ac575200c151c79b213ddb0311018b211a5`
- rust_crate: `blake3:5b703440699e8870f88ebc73209d9b89ca7418e94ee4142656ad49489cca4647`
- driver: `blake3:aaff4d40e31a21efc99d4c94962b765ab54e574f4b7caa5d6550f67891f243ee`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 12749 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
