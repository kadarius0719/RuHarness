# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — the **cockpit-as-a-friendly-wrapper** session (then harness-mcp). Everything is on `main` and pushed (this
session ended at the commits "§R2 fix pass …" and "harness-tui: the terminal front end …").
Before doing anything else:

1. Read `DECISIONS.md` from "2026-09-24 — §R2 fix pass" to the end, then `docs/TUI-DESIGN.md`
   §9 (the new direction), §3–§4 and §R3, then `docs/MCP-DESIGN.md` in full (DESIGN, REVIEWED —
   §R holds its 20 resolved findings) and `docs/CLI-HARDENING.md`.
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

Then, in order — **the direction changed at the end of the last session** (DECISIONS.md
"Direction change (user)"; docs/TUI-DESIGN.md §9): the cockpit becomes a user-friendly
wrapper — arrow-key/mouse file tree, deterministic actions on `Enter`, and a Copilot-style
chat pane inside it for model work.

* **The wrapper UX design** (TUI-DESIGN §9, "Suggested order" 1): a §15 check of how
  approachable TUIs do it (file trees, menus, mouse, on-screen hints — e.g. ratatui apps such
  as chess-tui for arrow/Enter/Esc/`?` navigation), then a design doc, an adversarial design
  review, the build on today's engine (`harness_tui::{model, events, spawn}`, the acts and
  their safety rules stay), a code review, a verification of the fixes.
* **harness-mcp** per `docs/MCP-DESIGN.md` (reviewed) — revisit it first: the chat pane is
  now its main client, and "migrate this" from chat needs the requester label (§7) so a
  chat-requested migration is recorded as such and never scored as unassisted.
* **The chat pane** after a spike on embedding an existing agent runtime headless (Claude
  Code's streaming mode first: flags, auth, permission prompts inside a pane, resume), plus a
  "migrate this" project skill.
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
