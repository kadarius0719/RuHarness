# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:3a05cc3447763895a4845bb05d8f15773b2f1aae5fa7315d4a91d078f79b5b04`
- rust_crate: `blake3:8c6e8b11a3248bcba23d3470632fc8a762e9608237d95b9dce9646e2c4207f0f`
- driver: `blake3:8b634c1505dce9f093814543679e40b889f186b2327d456b3fe7c2a20693c2b6`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1139 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
