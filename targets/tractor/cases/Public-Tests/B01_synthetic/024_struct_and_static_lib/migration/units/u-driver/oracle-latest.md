# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:cc0ed22e3bf771e84d79fe296b90c883330bb6256f643568324db66ca6f5eb25`
- rust_crate: `blake3:e05aca5f375f3ff351db83c6682e024b35bfe86f2c299371e0f5a3bc8dadca28`
- driver: `blake3:0add6670f3e97459a787403c156f84c58aea9edc9579c3c2c8171ac11869dd7d`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 10179 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
