# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:1df4525ff013e7142cf284042ecadb885df14011f7e371b644ceff91e78e6699`
- rust_crate: `blake3:6739a3f70268912c10a81ec7c10e92c021fa0d4ec5a0345d2c993995973b3e54`
- driver: `blake3:ddf05a81fce879a8ce14e59946f7d11895b9dc6f049abbab34314639bfaeac58`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 41492 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
