# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:72cdb45d25f9232467a7b5065d91cc95aa6addd77d4ac9126e294807f0118031`
- rust_crate: `blake3:b972941e556c5cf6174c168a03cb1e1efe1dbf14be5b779cef600ace07a67111`
- driver: `blake3:3fffaf3c08ad257c416905af0348a11a8d19e2a235011775a25c88604af4a6b1`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 7993 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
