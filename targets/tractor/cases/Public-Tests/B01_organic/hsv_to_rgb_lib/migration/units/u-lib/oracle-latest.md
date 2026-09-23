# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e8196393a2c0d9da87f083c83b710943473519d74c8b2f9c5950e4f4953f0eea`
- rust_crate: `blake3:87645571baa9c65a41cbc06c7efa8dac749a2bc9898e1c5ff37d745a1b63ef84`
- driver: `blake3:982f9cfb5f140415e0bf3f27a1bb1cf32d08db1970791f806b35e1c4198ba505`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6741 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
