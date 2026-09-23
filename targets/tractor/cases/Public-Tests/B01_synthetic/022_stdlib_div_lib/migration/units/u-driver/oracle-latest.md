# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:c4aab8953e94a974cbef17ffe74d35a0569639348ff76b3a37cc407df06a09ba`
- rust_crate: `blake3:4194a94a9dfd38c1c7fb6f886dd82c0e316132c1ebca407df9a196ee2899dcfc`
- driver: `blake3:107f8fda4c86115af16ddcd40246e6994d196f120908cfb243c659411f4cafb8`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 817 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
