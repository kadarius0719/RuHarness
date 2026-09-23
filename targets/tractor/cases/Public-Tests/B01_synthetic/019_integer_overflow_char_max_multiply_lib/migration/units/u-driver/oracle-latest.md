# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:8a180813d88663fef64a89a2835860abab43c81e56cf1c287f81584343757202`
- rust_crate: `blake3:6587b7e1a42d07c1a108ff2cf34b6f72ec21a2e50669c056ae700d40dc00b5c4`
- driver: `blake3:df38a143859af062dabb1b9499733c34cc773c1880c52078644510e3eacc64a1`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 5 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 585 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
