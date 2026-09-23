# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:e30cff08f04a082bfade029e681570b9764a32502f268f79a0fcef20b2443fa1`
- rust_crate: `blake3:39223feed96d13df273be8a18d9bcb3a35793301f982ddf120dc5b987be27341`
- driver: `blake3:1ffcc6297be08a92f54d84493c881ee2470f900a035d8fc06425573d0807beb6`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 24788 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
