# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — the review cockpit (`harness-tui`) session. Everything is on `main` and
pushed (last session ended at the commit "review acts: migrate --steer/--from, harness
override, provenance in core; harness-tui read model"). Before doing anything else:

1. Read `DECISIONS.md` from "2026-09-24 — TUI track: §15 research spike" to the end, then
   `docs/TUI-DESIGN.md` in full (§R is authoritative; **§R2 lists the OPEN findings of the
   code review of step 1–2 — the first thing to fix**), `docs/CLI-HARDENING.md` (the CLI
   contract the cockpit drives), and `docs/SCHEMAS.md` sections "CLI hardening" and
   "Review acts: steer attempts and human attempts".
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~480 tests;
   `--workspace` includes `harness-tui`, which the root's `default-members` leaves out of a
   plain `cargo build`), `cargo run -q -p harness-cli -- bench status --suite targets/tractor`.
3. The scorer needs the gitignored `targets/tractor/.scorer-vendor/`. If this worktree
   lacks it, copy it from another worktree with `cp -cR` (APFS copy-on-write) or re-run
   the one-time `cargo vendor` step in `targets/tractor/README.md`. Optionally copy
   `targets/tractor/.bench/` too, to skip the scorer rebuild.
4. Baseline check (about 40 minutes, zero tokens):
   `cargo run -q -p harness-cli -- bench check --suite targets/tractor --replay --jobs 6`.
   Expect `198 reproduce (1 conformant, 197 drifted), 2 expected divergence(s), 0
   problem(s)` and `bench check: OK — no regression`.

Then, in order:

* **Fix pass for docs/TUI-DESIGN.md §R2** (the verified findings of the adversarial code
  review of `migrate --steer/--from`, `harness override`, the core `provenance` rule and
  the `promotion_interrupted` report). Each fix with a regression test that fails without
  it (mutation-check the rule-guarding ones); update SCHEMAS.md/TUI-DESIGN.md where the
  contract moves; mark §R2 resolved; commit and push.
* **The terminal front end** (docs/TUI-DESIGN.md §3, §4, §6, §7; §8 step 4). The library
  half is done and tested — `harness_tui::{model, pairs, display}`: `Snapshot::load`,
  `UnitView` (report, ordered attempts, `ProvenanceView`, crate dir, verdict),
  `Snapshot::pairs` (C span with the facts-freshness guard; shim + logic callee found by
  tree-sitter-rust in any file), the display filter (tabs to 8-column stops, controls and
  bidi characters to `?`, 4 KiB cut on a char boundary). To build, behind the existing
  `tui` feature (dependencies already declared and vetted): `events` (typed NDJSON of the
  `ruharness-events` stream, unknown `k` kept), `spawn` (child with `process_group(0)`,
  stdin null, stdout + stderr reader threads, reload only after reader EOF + `wait()`,
  `/bin/kill -INT` only while `try_wait` is `None`), `highlight`, `view` (rail, function
  pairs with filler lines, verdict strip, run panel, overlays; narrow fallback < 110
  columns; `--layout split|stacked`), `app` (key handling returning commands; every act
  shows its argv and asks y/n; `R` resume re-spawns the stored argv; `e` runs
  `sh -c '$EDITOR "$@"' --` on a temp copy and hashes before/after), the TUI's own signal
  path (SIGINT/SIGTERM/SIGHUP → INT the child, wait ≤ 1 s, restore, die by the signal),
  and `main` (`--target`, `--harness`, `--allow-unsandboxed`, `--layout`). API notes
  verified last session: ratatui 0.30 with `default-features = false, features =
  ["crossterm_0_29"]` re-exports crossterm as `ratatui::crossterm` (no direct crossterm
  dependency); `ratatui::init()` installs a panic hook that restores the terminal;
  `ratatui::backend::TestBackend` for the view tests; `tree_sitter_rust::HIGHLIGHTS_QUERY`,
  `tree_sitter_c::HIGHLIGHT_QUERY`; in raw mode Ctrl-C arrives as a key event, not
  SIGINT. Then adversarial code review → fix pass → commit and push; README section.
* **harness-mcp** (hand-rolled JSON-RPC 2.0 over stdio, zero new crates, per the §15 spike):
  reuse `harness_tui::model` with `default-features = false`; tools spawn `harness --json`.
* After that: the feature-workflow view and the C-vs-Rust performance baselines, then the
  briefing's M5 (external detector plugin + `EXTENDING.md`). Carry-forwards: §16
  escalation automation + `harness usage`; the `crash-timeout` classifier; a Linux
  sandbox; the two deferred replay items; and from the CLI-hardening and TUI reviews:
  `verify` lacks the R6 gate, driver-attempt Accept, queueing on lock contention, an
  async client for cooperative cancellation, per-function verdict dots.

Environment (re-check; don't assume):
- There are no cloud API keys, so model calls go through the `external` hand-off.
- Answer hand-offs with plain Agent subagents, never Workflow agents.
- Set `--model` to the model that actually answers.
- Audit every batch with `targets/tractor/handoff-tools` before importing it.
- Never download a model without asking.

Process that works (keep it):
- Run a time-boxed research spike in subagents, and verify its premise by running it end
  to end.
- Write the design, then give it an adversarial design review from 3–4 lenses.
- Implement against the reviewed spec, then run an adversarial code review whose findings
  are verified against the code.
- In the fix pass, add regression tests that fail without the fix (mutation-check the
  rule-guarding ones).
- Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
