# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:ea2af5d4d038c98577e38bf11ab056e365493596f89efb2d1a97b58b80e19ddd`
- rust_crate: `blake3:5e186f270e4b9326dc2ce4dd02bbd97f80bc94cd56f2e79374cd00deda379d4f`
- driver: `blake3:3805a109503337ca38624654c528d523d43c5e5fd993563adce8c3c9586191aa`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 1264 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
