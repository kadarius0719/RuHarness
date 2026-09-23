# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:a0fabf948029c1adc317931d487f254b0326fb48430990b1634290dfce7f0009`
- rust_crate: `blake3:fbf2aa63de5a933d519cd9e35640d7bd3503da89d9e0bdf4fb7ff1bfc8ac8143`
- driver: `blake3:79c7b6c3d51eff51c5759b235c85b357b6f13729e6cba36d3e5a881eb6e8b7dd`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 3460 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
