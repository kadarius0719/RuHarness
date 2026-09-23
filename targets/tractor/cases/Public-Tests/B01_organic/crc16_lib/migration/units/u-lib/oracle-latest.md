# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:440378219cabec0587bb8b83da3af6ff01e5a078101cbdfa81a5a3693ce34d90`
- rust_crate: `blake3:bdf6af3d1046eee7b341ea64a41173c201aa9bd2714106209b25aff5fc58db0b`
- driver: `blake3:ffa06d6c03a072e3ff23bfde71fbe9ffd34e881524a713c1df178ec3dd31521f`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 5602 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
