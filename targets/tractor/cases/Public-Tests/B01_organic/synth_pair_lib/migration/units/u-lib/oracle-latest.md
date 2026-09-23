# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:809e15c263fc5b99483836183ebcf5851c95879d3c34078ed9f3c02379117966`
- rust_crate: `blake3:a0c6b2ddd87f7435cb81e5420a1c844b91adb264aac614e9ad59962512b6b741`
- driver: `blake3:92efd2fd36ad6d2160c357e94eb15ad128921b9b7dc8b3359d1a7db6aec8ba0b`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 1 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 13503 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
