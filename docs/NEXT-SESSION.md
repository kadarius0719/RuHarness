# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — the **harness-mcp** session. Everything is on `main` and pushed (this
session ended at the commits "§R2 fix pass …" and "harness-tui: the terminal front end …").
Before doing anything else:

1. Read `DECISIONS.md` from "2026-09-24 — §R2 fix pass" to the end, then `docs/MCP-DESIGN.md`
   in full (DESIGN, REVIEWED — §R holds the 20 confirmed findings of its design review, and the
   text above it is the post-review design to implement), then `docs/TUI-DESIGN.md` §4 and §R3
   (the cockpit whose library the server reuses) and `docs/CLI-HARDENING.md`.
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~544 tests),
   `cargo run -q -p harness-cli -- bench status --suite targets/tractor`.
3. The scorer needs the gitignored `targets/tractor/.scorer-vendor/` (copy with `cp -cR` from
   another worktree, or `targets/tractor/README.md`'s one-time `cargo vendor`); copy
   `targets/tractor/.bench/` too.
4. Baseline (about 35 minutes, zero tokens) ON A QUIET MACHINE — driver re-validation is timing
   sensitive and flaked once under a load average of 33:
   `cargo run -q -p harness-cli -- bench check --suite targets/tractor --replay --jobs 6`.
   Expect `198 reproduce (1 conformant, 197 drifted), 2 expected divergence(s), 0 problem(s)`
   and `bench check: OK — no regression`.

Then, in order:

* **harness-mcp** per `docs/MCP-DESIGN.md` (reviewed): new crate `crates/harness-mcp`
  (harness-core, harness-tui `default-features = false`, serde_json, signal-hook — zero new
  crates), hand-rolled JSON-RPC 2.0 over stdio, MCP 2025-06-18; tools `harness_status`,
  `harness_unit`, `harness_steer` (steer attempts only), `harness_retry` (the record's run
  shape; refuses unseeded `external`), `harness_promote`; server flags `--target`,
  `--target-root`, `--harness`, `--provider`, `--allow-unsandboxed`; progress notifications,
  shutdown path, `busy` refusals, untrusted values wrapped in `structuredContent`, size caps.
  Reuse `harness_tui::{model, events, spawn}` (`spawn::interrupt_and_wait` is the shutdown
  path). The tests of §5 (a scripted stdin session; the zopfli end-to-end through the
  `external` hand-off; cancellation during the spinning-driver promote — see
  `crates/harness-tui/tests/signals.rs` for the pgrep/`drv_c` pattern). Then an adversarial
  code review → fix pass (+ a verification of the fix pass: last session every fix pass
  produced new findings) → README `.mcp.json` section → commit and push.
* After that: the feature-workflow view and the C-vs-Rust performance baselines, then the
  briefing's M5 (external detector plugin + `EXTENDING.md`). Carry-forwards: §16 escalation
  automation + `harness usage`; the `crash-timeout` classifier; a Linux sandbox; the two deferred
  replay items; `verify` lacks the R6 gate; driver-attempt Accept; queueing on lock contention;
  an async client for cooperative cancellation; per-function verdict dots; bounded reads in
  harness-core (MCP-DESIGN §R TRUST-4).

Environment (re-check; don't assume):

* There are no cloud API keys, so model calls go through the `external` hand-off.
* Answer hand-offs with plain Agent subagents, never Workflow agents.
* Set `--model` to the model that actually answers.
* Audit every batch with `targets/tractor/handoff-tools` before importing it.
* Never download a model without asking.
* The harness-tui pty tests (`crates/harness-tui/tests/signals.rs`) need the `harness` binary
  next to `harness-tui` (`cargo test --workspace` builds it).

Process that works (keep it):

* Run a time-boxed research spike in subagents, and verify its premise by running it end to end.
* Write the design, then give it an adversarial design review from 3–4 lenses.
* Implement against the reviewed spec, then run an adversarial code review whose findings are
  verified against the code — then VERIFY THE FIX PASS the same way (it found 18 new issues
  last time).
* In the fix pass, add regression tests that fail without the fix (mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test).
* Real end-to-end tests find what reviews miss (the pty test found `Terminal::clear`'s cursor
  query on its first run).
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
