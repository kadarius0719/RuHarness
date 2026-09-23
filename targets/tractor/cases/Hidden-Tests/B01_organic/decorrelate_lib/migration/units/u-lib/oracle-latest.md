# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:45cded970ec052877a505df3808073e84bb17d165396c30a81ddc735259e02e5`
- rust_crate: `blake3:21713b8a9fce42dc6fc691be764218a73306588a310c9ee1770ca966a6bae1d1`
- driver: `blake3:78a88b21349b25fcb00e9be8610649050ad72b48273aa0d90866669bff0e8bac`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 5751 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
