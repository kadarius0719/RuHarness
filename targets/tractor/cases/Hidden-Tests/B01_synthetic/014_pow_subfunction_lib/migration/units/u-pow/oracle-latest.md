# Oracle verdict — u-pow

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:b2be8029115482b20756a87753c49a6fb3ef7583fc17aee678e6fb51b053a400`
- rust_crate: `blake3:7b0058880f2fe6f02beb87dacf357fab6261548c811519ffa69377ff40d646fd`
- driver: `blake3:fa901952b3a5b05e4c15dab3744696d49e976ed158a5d453a4d3bcac2d4161f8`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off; observable: stdout+stderr

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 10707 bytes identical (stderr: 4122 bytes identical)
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
