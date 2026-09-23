# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:2b4cf0ac6679d17c788618b9c6f953c350c361901c43c7acfd0caa37ae908ddf`
- rust_crate: `blake3:ccf7f01a577c566291dc1aa2bf1e95954fb762e6ddd10fe5a45480680eb9309c`
- driver: `blake3:a4555b02226816bddc9f2cbeae1b37a79d62a5b7cbfbe561894b5fada480ff6a`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 46006 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
