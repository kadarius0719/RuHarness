# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:4af12fe55ef2f033bf4ccf591fada7372eceb7ac3406d69babcc9eb326d749cd`
- rust_crate: `blake3:7ba5512dc222eddaaebdf89d72c56371210314183f97d11f69aca9a1f8251207`
- driver: `blake3:97f5f0428ba19c0b343629846c45c4a7d3445c5a3f4dc42b84d516d847b631f4`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 33161 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
