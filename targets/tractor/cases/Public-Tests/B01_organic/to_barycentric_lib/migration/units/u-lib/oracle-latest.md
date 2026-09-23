# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:0501b40ca32ae370fec5e17815a2eb9b6a4f30eb127551a99db34b02577bc2e1`
- rust_crate: `blake3:e3501c4f8f81b628df58922e8c398259e7392dc1bdd19f6f4253a41bd0570aee`
- driver: `blake3:7de7356f8366fd5c7db9c7d29fe5e080a4c8dae9533283646febf4fb56963249`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 15842 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
