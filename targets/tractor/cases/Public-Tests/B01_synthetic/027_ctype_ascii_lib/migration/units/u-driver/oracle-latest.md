# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:f7eff6e0843bd30e56ceec34a848a4601be2afc28a87bec544397fcc291ced39`
- rust_crate: `blake3:2afef4bedd93937d504108404ca35e4289969d5690242543df40413fe5d34b6a`
- driver: `blake3:34f03cea6f3f01e4a1a053e18bf3265ec3d2afd5a2c1c5bb12c87b0c6288822a`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 52428 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
