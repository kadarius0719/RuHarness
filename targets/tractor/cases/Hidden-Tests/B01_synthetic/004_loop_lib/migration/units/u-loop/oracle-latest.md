# Oracle verdict — u-loop

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:96addbc1740e7ed93f710119baf98f41255ff3c8968972e12231f96f783d7519`
- rust_crate: `blake3:80f12d089cbe172e054c071eed3ac8fae25d3f1062443ea839185fb3fb653b8a`
- driver: `blake3:856f6472940c2cd708492651b4822cc5e003b445ee0844b7b4f6b7d479baba23`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 8777 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
