# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:69d743a124bea7f297be4218d7452a4fc1246eed227c18eec49ce79502e23147`
- rust_crate: `blake3:4df35766c0afd33b4e267f0d2cc0fc71f8768e391e01a351a42950cc19e5dbba`
- driver: `blake3:d16750e5730107a89f763def49d47ff0bc3804f769b5d0f56b9e44b2dc0e8a48`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 10606 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
