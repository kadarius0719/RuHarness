# Migration Decisions — targets/zopfli

Target: google/zopfli @ `ccf9f0588d4a4509cb1040310ec122243e670ee6` (Apache-2.0, archived
upstream 2025-10). Phase 1 / M0: single leaf unit, end-to-end thread.

## 2026-09-17 — Unit u001-katajainen selected

`src/zopfli/katajainen.c` → Rust crate `migration/units/u001-katajainen/katajainen_rs`.

Why this unit first: one public symbol (`ZopfliLengthLimitedCodeLengths`), only libc
callees, no globals/IO/threads, deterministic output — the cheapest possible oracle.

## 2026-09-17 — Behavioral hazards found during translation (pre-oracle)

1. **Comparator truncation (C `LeafComparator`) — resolved as C-side UB, not
   behavior to preserve.** The C comparator returns `(int)(a->weight - b->weight)` on
   `size_t` weights; once (shifted) weights differ by ≥ 2^31 the truncated sign flips
   and the comparator stops being a total order — which makes the `qsort` call
   undefined behavior per C11 7.22.5p4. Empirically confirmed: a first Rust port that
   replicated the truncated sign was *rejected by Rust's own sort* ("comparison
   function does not correctly implement a total order" panic) on large-weight inputs.
   Decision: the Rust port uses the true total order (`sort_unstable_by_key` on the
   packed weight; keys are unique so stability is irrelevant), which is
   result-identical everywhere the C comparator is consistent — all frequencies
   < 2^22 — and well-defined beyond it, where C's output is unspecified garbage
   ordering (still memory-safe, still Kraft-valid, just not reproducible). The
   differential driver constrains random frequencies to < 2^22 accordingly; the
   ≥ 2^55 error-path cases are kept (that check precedes the sort). Note for the
   observer/M2: zopfli's 20MB master blocks can in principle push real frequencies
   past 2^22, i.e. upstream zopfli can hit this UB on adversarial inputs — worth an
   upstream-facing note, and exactly the kind of finding the §6 taxonomy
   ("reliance on undefined behavior") exists to catch.
2. **Uninitialized node weight in `BoundaryPMFinal`:** the final chain node's `weight`
   is never initialized in C (and never read). Rust sets it to 0.
3. **`ExtractBitLengths` underflow guard:** if the chain ever had length 16
   (`end == 0`), C would index `counts[-1]` (UB). Unreachable for `maxbits ≤ 15`; Rust
   would panic instead of corrupting — acceptable hardening, no behavior change on
   reachable inputs.
4. **Implicit precondition `maxbits ≤ 15` (found by the oracle, not by reading):** the
   first differential run passed `maxbits=20`; the *C baseline* died with SIGBUS —
   `counts[16]` in `ExtractBitLengths` underflows once a chain exceeds 16 nodes. The
   unit's contract is therefore `1 ≤ maxbits ≤ 15` (all zopfli call sites use 7 or 15);
   the driver now stays inside it. The Rust port panics (bounds check) rather than
   corrupting memory outside the contract — strictly better failure mode, identical
   behavior inside the contract.

## 2026-09-17 — Oracle definition for u001

Run from repo root: `cargo run -p harness-cli -- verify u001-katajainen --target targets/zopfli`
(originally `cargo run -p harness-m0 -- oracle` at M0; promoted at M1 with identical checks)
- Differential driver: `migration/units/u001-katajainen/driver.c`, deterministic
  xorshift64 inputs (~500 cases: n ∈ {0..300}, maxbits ∈ {1,3,7,15,20}, sparse/dense/
  large-value/error-path frequency patterns), full bitlength arrays printed; C-linked
  vs Rust-linked stdout must be byte-identical.
- Whole-program: `zopfli` binary all-C vs mixed (katajainen.c dropped, Rust staticlib
  linked); compressed outputs over sample files must be byte-identical.
- Sanitizers: C driver additionally built/run with `-fsanitize=address,undefined`.

Done-criteria: all three green. Status lives in `units/u001-katajainen/contract.md`.
