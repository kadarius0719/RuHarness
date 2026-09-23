# Oracle verdict — u001-katajainen

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:24231df2ed359c49223558a9f977ce9d6d370e6ad1d9baf6d34cf47751a81f4c`
- rust_crate: `blake3:b91f77e8501bfb3ec59ca6d644f1c26b4c158bd9f4f1fcbd8fb95136b8b7d67d`
- driver: `blake3:019139974804439b50881cb5be2173812764eec4d92f13110564d7514fe68d38`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 183832 bytes identical
- **whole-program:sample_text.txt**: PASS — 205 bytes identical
- **whole-program:sample_rand.bin**: PASS — 16407 bytes identical
- **whole-program:sample_empty**: PASS — 20 bytes identical
- **sanitizers**: PASS — asan+ubsan clean
