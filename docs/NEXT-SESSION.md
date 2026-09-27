# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **the chat pane of the friendly-wrapper cockpit: spike first**. Everything
is on `main` and pushed. The wrapper's two builds are done: Build A (the keyboard-complete
navigator, 2026-09-25) and Build B (the mouse, 2026-09-27: built, reviewed — 48 findings, 0
refuted — and fixed three times, §R6–§R8 of docs/COCKPIT-WRAPPER-DESIGN.md); 731 tests green;
75 mutations of Build B's rules and fixes killed. Before doing anything else:

1. Read `DECISIONS.md` from "Cockpit wrapper, Build B (the mouse)" to the end (and the Build A
   entry before it if the cockpit is new to you), then docs/TUI-DESIGN.md §9 (the direction:
   a chat pane like Copilot inside the cockpit, running an existing agent runtime headless
   with harness-mcp as its tools), docs/COCKPIT-WRAPPER-DESIGN.md §10 (what the chat pane
   must decide: how chat's acts reach the activity panel and the cockpit's confirmation, how
   a chat-requested act is labelled, where chat sits in the focus order) and §R6–§R8 (the
   mouse's rules the chat pane must keep: a click answers a dialog only once it is armed,
   shown armed and a second old; a click outside a dialog does nothing), docs/MCP-DESIGN.md §7
   (the requester label) and §4 (what chat may record).
2. Read the code the chat pane plugs into: `crates/harness-tui/src/view.rs` `draw` (the right
   side is NOT reserved from 150 columns today — §R3 USE-15: the View takes the width until
   the chat exists; reserve it when the chat lands), `app.rs` (`Focus` is Files | View;
   `on_event` is the loop's step; every act goes through an armed `Dialog`), `main.rs` (the
   event loop, the terminal guard, the signal path — a headless agent child must be
   interrupted and reaped on every way out like the spawned `harness`), and `crates/harness-mcp`
   (its tools, `harness_answer`, the pending blind hand-off refusal).
3. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~731 tests,
   including the pty tests in `crates/harness-tui/tests/signals.rs`).
4. Baseline only if the CLI or the scanner changes: the bench check needs the gitignored
   `targets/tractor/.scorer-vendor/` and `.bench/` (copy with `cp -cR` from another worktree)
   and a quiet machine; expect `198 reproduce (1 conformant, 197 drifted), 2 expected
   divergence(s), 0 problem(s)` and `bench check: OK — no regression`.

Then, in order, each step committed when green:

1. **A §15 spike** (time-boxed, subagents, sources cited, verify its premise end to end): the
   current headless/streaming mode of the agent runtime to embed (Claude Code first: its
   non-interactive flags, stream-JSON output, permission handling inside a pane, session
   resume, attaching harness-mcp, auth without an API key), and how comparable TUIs render a
   chat stream in ratatui. Deliverable: DECISIONS entry with options, the chosen default,
   rejected alternatives, and "revisit when".
2. **The chat pane design** (docs/CHAT-PANE-DESIGN.md or a section of the wrapper design):
   the pane, focus order, keys and mouse (the Build B rules hold), how a chat act is shown and
   confirmed (the same armed dialog, the argv shown), the requester label in the ledger
   (MCP-DESIGN §7), the "migrate this" skill. Adversarial design review from 3–4 lenses,
   verify every finding, revise — then CHECK THE REVISION.
3. Build it against the reviewed design; review, fix pass, VERIFY THE FIX PASS and check
   each further pass (Build B: the check of the fix pass again found a medium all three
   checkers agreed on); mutation checks of the named rules; DECISIONS handoff; commit, push.

Separately suggested (their own tasks): the crate content hash skips files outside `src/`
(SAFE-5 of the design review); harness-detect's walk follows symlinks out of `source_dir` (a
one-line switch to `walk::confined`); `verify`'s R6 gate; the harness-core ledger test that a
fork in the same test binary can fail (DECISIONS). After the chat pane: the feature-workflow
view and the C-vs-Rust performance baselines, then the briefing's M5 (external detector
plugin + `EXTENDING.md`). Carry-forwards: §16 escalation automation + `harness usage`; the
`crash-timeout` classifier; a Linux sandbox; the two deferred replay items; driver-attempt
Accept; queueing on lock contention; an async client for cooperative cancellation;
per-function verdict dots; bounded reads in harness-core (MCP-DESIGN §R TRUST-4);
harness-mcp's revisit triggers; Build B's accepted items (DECISIONS: hits are the last
frame's; the notice row and border prompts are not clickable; TSTP).

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
* Review agents must not delete scratchpad files they did not create; verifiers read the
  reviewed commit with `git show <sha>:<path>` (or a `git archive` copy with its own
  CARGO_TARGET_DIR) so fixing can proceed meanwhile.
* Driving the cockpit headless: a Python `pty.fork()` driver with a small VT interpreter; SGR
  mouse reports are `\e[<0;COL;ROWM` (press) and `…m` (release), 1-based; the wheel is button
  64/65. The interpreter does not model the alternate screen: after the cockpit exits it shows
  the last frame — read the mode sequences (`?1049l`, `?1000l`) instead. No tmux here, no
  `timeout` (use `perl -e 'alarm N; exec @ARGV'`).
* Mutation checks: a script that replaces one string per mutant, runs the guarding test,
  restores the file (keep the tree committed first; check `git status` after). A mutant that
  survives because a fix doubled a guard is equivalent — mutate both copies together, or
  write the test that tells the guards apart. A test on a synthetic future clock can pass for
  the wrong reason (a key clears notices older than its `now`).
* A mutation whose test hangs or is killed (OOM) counts as killed — make such tests fail fast
  (a bounded wait on a channel) rather than hang. A read blocked on a FIFO cannot be released
  reliably: such a test needs a watchdog thread that calls `process::exit(1)` (see
  `a_fifo_in_the_crate_never_freezes_the_menu`). A script's timeout kills `cargo`, not the
  hung test binary under it — check `ps` for leftovers before rerunning.
* A compound shell command keeps going after a failing `cargo test`: check the test result
  before `git commit` in the same line (or use `set -e`).

Process that works (keep it):

* Run a time-boxed research spike in subagents, and verify its premise by running it end to end.
* Write the design, give it an adversarial design review from 3–4 lenses, verify every finding
  against the code, revise — then CHECK THE REVISION.
* Implement against the reviewed spec, then run an adversarial code review whose findings are
  verified against the code — then VERIFY THE FIX PASS the same way, and check each further
  pass (Build A: 12 then 7 new defects; Build B: ~15 then 9, one medium found by all three
  checkers each time).
* In the fix pass, add regression tests that fail without the fix, and mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test.
* Real end-to-end tests find what reviews miss (Build B: a pty drive found the triple click
  before the review did).
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
