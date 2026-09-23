# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:24a2236e27d7cf1545b69bc05890961cfac73eaca9a6c04a48abd774c4feb951`
- rust_crate: `blake3:23d4c13b71bd3312ef40b76dfbf4e53322a078a14f9e379e9b5ee09cf1da6457`
- driver: `blake3:b435f241e405afebeee58205e530f3fdfb7919a08a90a13a9c7f44a6dd29bd62`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1254 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
