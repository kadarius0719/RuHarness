# Oracle verdict — u-driver

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:dc36f3d88973b97a052c6300a17c205c07241b4caf166c240d1d4c2fce6f83c1`
- rust_crate: `blake3:1064dc349c4ac45b1410587bddf83ac5fbe85d79152b0b993be75fa6280e9d18`
- driver: `blake3:a2b02f877952d7e65d690457dd55aa58083b8c22e94233ae3019e3bc355a2cd3`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 5147 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
