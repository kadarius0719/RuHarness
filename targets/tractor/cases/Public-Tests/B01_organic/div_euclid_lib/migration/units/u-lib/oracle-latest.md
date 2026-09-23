# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:b79b1d15dd5d854e756f9f04b030c54a869ad00dde24a13b7097851fe037723e`
- rust_crate: `blake3:d8d3c6c84cac5a3e561b051e315050a3bed57ec9dc45fb1ba36490a13f9212c0`
- driver: `blake3:9713a94904c4fa245aefd0d5f5e551df889d7034224e1ff512263866caef85f8`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 12017 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
