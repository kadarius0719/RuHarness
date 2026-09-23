# Scorer dependency patches (harness-authored, hash-locked in ../../corpus.lock)

Applied through `[patch.crates-io]` in `../Cargo.toml`. Patches touch ONLY
third-party dependencies of the corpus scorer — never `tools/cando2`, the
runners, or any test vector.

## process-fun-core 0.1.2 — `pipe2` is Linux-only

`create_pipes` calls `nix::unistd::pipe2(O_CLOEXEC)`, which does not exist on
macOS, so the corpus scorer (`cando2`) cannot build there at all. On
non-Linux targets the patch uses `pipe()` followed by `fcntl(F_SETFD,
FD_CLOEXEC)` on both ends. Linux behavior is unchanged (the original line is
kept under `cfg(linux/android)`). cando2 itself imports only re-exported
signal constants from `process_fun`; scoring semantics are unaffected.
Found 2026-09-23 while building the scorer (M4); see DECISIONS.md.
