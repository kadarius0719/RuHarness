# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:5930a61f26908ddfc08555fff4919b019aee258ce18c43cc06e38da64d76db13`
- rust_crate: `blake3:88b519c3d5d6d0fbab0e344069a764fb9bb48199387a3b6892a245c519a58c0f`
- driver: `blake3:befa5bed470ae367eecf7842d9f3b606c704a527445e57f91c1f6b34038d5586`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 15020 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
