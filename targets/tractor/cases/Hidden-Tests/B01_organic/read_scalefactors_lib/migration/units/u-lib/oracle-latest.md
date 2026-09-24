# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:fd0540b200009a06059b084eacaeec0d1fc6278ce0274d3ff5a119836da8deb0`
- rust_crate: `blake3:f98321590a55e90ae5462ee98a5d8a05aaf35a26a73455e99290bd2ab4444d26`
- driver: `blake3:b2544df11d7df6bd265ac4be3f30210309cee3432d8d78fa8ca87cb61f1e1263`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off; boundary: sancov+guard-pages rt=fe651697; observable: stdout+stderr

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 11503 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
- **boundary**: PASS — Rust stays inside the C's footprint: 31 call(s), 124 guarded object(s) (6 untouched by the C, 86 partially touched, 0 widened), 0 argument(s) unshadowed; tail and head layouts clean; note: in call 3 the C reads the driver's stack through a pointer field (unchecked)
