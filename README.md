# RuHarness

A provider-agnostic harness that migrates codebases to safe, idiomatic Rust
incrementally and verifiably: LLM agents for judgment, deterministic tooling for
measurement, and a differential oracle as the only definition of done.

**Phase 1 (current): C → Rust**, behind the original C ABI, in dependency order,
with the project building and passing its tests as a mixed C/Rust link at every
commit. See `DECISIONS.md` for the engineering log.

## Status: M0 — end-to-end thread ✅

One real C library (vendored [zopfli](https://github.com/google/zopfli), pinned in
`DECISIONS.md`), one leaf unit (`katajainen.c` → safe Rust with an `extern "C"`
shim), one binary that scans, and one oracle that verifies:

```bash
cargo run -p harness-m0 -- scan     # call graph + leaf-unit ranking (tree-sitter)
cargo run -p harness-m0 -- oracle   # differential + whole-program + sanitizer checks
```

The oracle builds the differential driver twice (linked against C, then against the
Rust staticlib) and byte-diffs their outputs over ~500 deterministic cases; builds
the full `zopfli` binary all-C and mixed-C/Rust and byte-compares compressed
output; and runs the C baseline under ASan/UBSan.

Migration state lives on disk in `targets/zopfli/migration/` (the ledger): decisions,
unit contracts, and oracle results. Any agent must be able to resume cold from it.

## Layout

- `m0/` — the harness in embryo (promoted to a multi-crate workspace at M1)
- `targets/zopfli/` — vendored migration target
- `targets/zopfli/migration/` — the ledger: `DECISIONS.md`, `units/<id>/`
- `DECISIONS.md` — harness engineering log (research spikes, dependency
  justifications, milestone handoffs)
