# Unit u001-katajainen — interface contract

**Status:** verified (oracle GREEN 2026-09-17, see `oracle-latest.md`)
**Source:** `src/zopfli/katajainen.c` (+ `katajainen.h`)
**Rust crate:** `migration/units/u001-katajainen/katajainen_rs` (staticlib)

## Preserved C ABI

```c
int ZopfliLengthLimitedCodeLengths(
    const size_t* frequencies, int n, int maxbits, unsigned* bitlengths);
```

- `frequencies`: array of `n` symbol counts (read-only).
- `bitlengths`: output array of `n` `unsigned` (u32), fully written (zeroed first).
- Returns 0 on success; 1 if `maxbits` cannot represent the used symbols or any
  frequency ≥ 2^55.
- No global state, no I/O, no allocation visible to the caller.
- **Domain:** `1 ≤ maxbits ≤ 15` (C baseline corrupts memory beyond 15 — hazard #4);
  behavior-identical guarantee holds for frequencies < 2^22 (beyond that the C
  baseline's qsort comparator is inconsistent, i.e. C-side UB — hazard #1). All real
  zopfli call sites use maxbits ∈ {7, 15}.

## Semantics to preserve exactly

Boundary package-merge (Katajainen–Moffat–Turpin), DEFLATE-tailored: unused symbols get
bitlength 0; a single used symbol gets bitlength 1; two symbols get +1 each. Leaves
sort by packed `(weight << 9) | symbol` key in a true total order (see
migration/DECISIONS.md hazard #1 for why the C comparator's truncation is *not*
preserved).

## Interior-unsafe policy

`unsafe` only in the `ffi` shim module (raw-pointer → slice at the boundary). Interior
logic is safe Rust: pointer-chained nodes from the C version become index-based arena
(`Vec<Node>` + `Option<usize>` tails).

## Done-criteria (oracle)

`cargo run -p harness-m0 -- oracle` fully green:
1. differential driver stdout byte-identical (C vs Rust link),
2. whole-program zopfli compressed output byte-identical (all-C vs mixed link),
3. ASan/UBSan-clean C-side driver run.
