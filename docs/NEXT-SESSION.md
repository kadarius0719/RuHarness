# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **Build A of the friendly-wrapper cockpit**. Everything is on `main` and
pushed. On 2026-09-24 the two parallel sessions (harness-tui's front end; harness-mcp on top
of it) were reconciled (611 tests and the pty tests green; docs aligned), and the wrapper was
DESIGNED and reviewed — not built: docs/COCKPIT-WRAPPER-DESIGN.md (46 review findings, 43
confirmed; the check of the revision found 13 more; all resolved in §R/§R2). Before doing
anything else:

1. Read `DECISIONS.md` from "Reconciliation of the parallel sessions" to the end (the spike,
   then the design and its review), then `docs/COCKPIT-WRAPPER-DESIGN.md` in full, then
   `docs/TUI-DESIGN.md` §2–§4 (the engine and its safety rules, which all hold) and §R3, and
   `docs/MCP-DESIGN.md` §0, §3, §4 (the provider list; the preflight being moved).
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~611 tests,
   including the pty tests in `crates/harness-tui/tests/signals.rs`).
3. The scorer needs the gitignored `targets/tractor/.scorer-vendor/` (copy with `cp -cR` from
   another worktree, or `targets/tractor/README.md`'s one-time `cargo vendor`); copy
   `targets/tractor/.bench/` too.
4. Baseline (about 30 minutes, zero tokens) ON A QUIET MACHINE — driver re-validation is timing
   sensitive: `cargo run -q -p harness-cli -- bench check --suite targets/tractor --replay --jobs 6`.
   Expect `198 reproduce (1 conformant, 197 drifted), 2 expected divergence(s), 0 problem(s)` and
   `bench check: OK — no regression`.

Then build **Build A** in the design's order (§14), each step committed when green:

0. **The bugs the review found in the shipped cockpit**, each fixed after a regression test that
   fails without it: `Q` and the quit prompt through arming (SAFE-11); the non-blocking terminal
   guard + the kept edit recorded before `resume` (SAFE-10, CHK-5); the Retry refusals (unseeded
   `external`, half-seeded) and the `--provider` list, Modify passing `--provider` (SAFE-3,
   SAFE-12, CHK-1, CHK-13); harness-mcp's preflight moved into the harness-tui library and run by
   the cockpit before every load, loads on a worker thread (SAFE-6, CHK-10).
1. Dialogs, the arming latch and buttons (clock injected into `app`), the menus' argv table —
   tested and mutation-checked.
2. harness-core `walk::confined` + the scanner switched (facts byte-identical on zopfli,
   read_scalefactors and a symlink copy), `status::live_holder` public, the two additive model
   fields; `harness_tui::files` and its tests.
3. The navigator (`Selection`, tree, View, activity rows + narrator, notices, hint bar, help,
   empty states, hit-record API), goldens, the signals.rs rewrite, the keyboard pty e2e.
4. README + TUI-DESIGN §3; adversarial code review; fix pass; VERIFY THE FIX PASS (and further
   passes as needed — the last milestones needed three); mutation checks; DECISIONS handoff;
   commit and push. Build B (mouse) is the session after.

Separately suggested (their own tasks, not part of Build A): the crate content hash skips files
outside `src/` (a `build.rs` builds unseen — SAFE-5); harness-detect's walk follows symlinks out
of `source_dir`. After the wrapper: the chat pane (spike first) with harness-mcp's requester
label and a "migrate this" skill; then the feature-workflow view and the C-vs-Rust performance
baselines, then the briefing's M5 (external detector plugin + `EXTENDING.md`). Carry-forwards:
§16 escalation automation + `harness usage`; the `crash-timeout` classifier; a Linux sandbox; the
two deferred replay items; `verify` lacks the R6 gate (decide separately); driver-attempt Accept;
queueing on lock contention; an async client for cooperative cancellation; per-function verdict
dots; bounded reads in harness-core (MCP-DESIGN §R TRUST-4); harness-mcp's revisit triggers.

Environment (re-check; don't assume):

* There are no cloud API keys, so model calls go through the `external` hand-off.
* Answer hand-offs with plain Agent subagents, never Workflow agents.
* Set `--model` to the model that actually answers.
* Audit every batch with `targets/tractor/handoff-tools` before importing it.
* Never download a model without asking.
* The pty and e2e tests (`crates/harness-tui/tests/signals.rs`, `crates/harness-mcp/tests/e2e.rs`)
  need the `harness` binary next to theirs, built from the current sources (`cargo test
  --workspace` builds it; the MCP e2e refuses a stale one).
* Subagents cannot write report files here: they return findings as text, and the main
  session writes them into its scratchpad (one subdirectory per reviewer) for the verifiers.
* Review agents must not delete scratchpad files they did not create.
* Opening the cockpit headless: drive it under `expect` in a 120×40 pty and render the captured
  bytes with a small VT interpreter (no tmux on this machine).

Process that works (keep it):

* Run a time-boxed research spike in subagents, and verify its premise by running it end to end.
* Write the design, give it an adversarial design review from 3–4 lenses, verify every finding
  against the code, revise — then CHECK THE REVISION (the wrapper's check found 13 more, 3 high).
* Implement against the reviewed spec, then run an adversarial code review whose findings are
  verified against the code — then VERIFY THE FIX PASS the same way.
* In the fix pass, add regression tests that fail without the fix, and mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test.
* Real end-to-end tests find what reviews miss.
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
