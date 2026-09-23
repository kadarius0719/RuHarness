# Oracle verdict — u-lib

Verdict: **GREEN**

Inputs tested:
- unit_source: `blake3:d166c15157a8e64d0ce607727cf17c650a1d27e5098fcaa6bcf29b44fb329ed5`
- rust_crate: `blake3:1343155432ea77cceab29f19bab661d7c63e0765344718f554e6dd7272ebb306`
- driver: `blake3:c76cf61b28d565e69d703bb383b03b3ac9e289aa036a2346ad0e5eb158775db7`
- toolchain: rustc 1.94.1 (e408947bf 2026-03-25); Apple clang version 21.0.0 (clang-2100.1.1.101); sandbox: sandbox-exec; cflags: -ffp-contract=off

Checks:
- **symbol-set**: PASS — 2 exported symbol(s) match the unit's symbols exactly
- **capabilities**: PASS — no capability beyond the C unit's (allowed: none); no asm
- **driver-shape**: PASS — driver object defines only main, references only the unit and allowlisted libc; source lint clean
- **differential-driver**: PASS — 6450 bytes identical
- **whole-program**: PASS — not configured for this target
- **sanitizers**: PASS — asan+ubsan clean
