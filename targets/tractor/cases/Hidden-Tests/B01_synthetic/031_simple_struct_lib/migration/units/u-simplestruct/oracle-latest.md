# Oracle verdict — u-simplestruct

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:462d602e8eaa46003e031974d0998168c93e7b12471d73d30d358e261bca5fd9`
- rust_crate: `blake3:599c5dfbd3986ff2cd7b4181a6579218e0059895ae7c8667bad78815bd81afb8`
- driver: `blake3:7da0f0a21795d45d25525408d82389d75cc45d11125a352a76aa1ada931aea5d`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6179 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
