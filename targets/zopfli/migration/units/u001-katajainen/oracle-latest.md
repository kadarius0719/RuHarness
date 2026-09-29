# Oracle verdict — u001-katajainen

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:24231df2ed359c49223558a9f977ce9d6d370e6ad1d9baf6d34cf47751a81f4c`
- rust_crate: `blake3:b91f77e8501bfb3ec59ca6d644f1c26b4c158bd9f4f1fcbd8fb95136b8b7d67d`
- driver: `blake3:019139974804439b50881cb5be2173812764eec4d92f13110564d7514fe68d38`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off; observable: stdout+stderr
- features: `blake3:e613abcb485e3fb146c07b68374707d1f58f3b4feb36cb1baf768a5311417265`
- program: `blake3:d191be5c4b9a2380dc6a075da3c8dccc92216562d1103bc82c7dc6d8a9e75d64`

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 183832 bytes identical
- **whole-program:sample_text.txt**: PASS — 205 bytes identical
- **whole-program:sample_rand.bin**: PASS — 16407 bytes identical
- **whole-program:sample_empty**: PASS — 20 bytes identical
- **sanitizers**: PASS — asan+ubsan clean
- **feature:gzip/text**: PASS — exit 0; stdout 205 bytes identical; stderr empty
- **feature:gzip/rand**: PASS — exit 0; stdout 16407 bytes identical; stderr empty
- **feature:zlib/text**: PASS — exit 0; stdout 193 bytes identical; stderr empty
- **feature:deflate/text**: PASS — exit 0; stdout 187 bytes identical; stderr empty
- **feature:verbose/text**: PASS — exit 0; stdout 205 bytes identical; stderr 425 bytes identical
- **feature:quick/text**: PASS — exit 0; stdout 205 bytes identical; stderr empty
- **feature:help/flag**: PASS — exit 0; stdout empty; stderr 492 bytes identical
- **feature:no-file/missing**: PASS — exit 0; stdout empty; stderr 29 bytes identical
