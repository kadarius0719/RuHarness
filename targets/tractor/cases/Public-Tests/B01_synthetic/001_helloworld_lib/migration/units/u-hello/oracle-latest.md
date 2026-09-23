# Oracle verdict — u-hello

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:ac0d6799ec4fe4b0deb85576627112ca20054e704194803dc3b3ddf92554b6ff`
- rust_crate: `blake3:ca5cb99423622458eac076732043d78bb215541299d9f8fa5af33d8f111bcd4b`
- driver: `blake3:54e7c0d8d75d337a2eeab5c283ca1948b1d0de09f57fe999495e5c94bbdc9939`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 130 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
