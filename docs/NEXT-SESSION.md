# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **Build B of the friendly-wrapper cockpit: the mouse**. Everything is on
`main` and pushed. On 2026-09-25 Build A — the keyboard-complete navigator — was built,
reviewed (51 findings, 0 refuted), fixed, and the fixes checked twice more (§R3–§R5 of
docs/COCKPIT-WRAPPER-DESIGN.md: three fix passes); ~695 tests green; 68 rule-guarding
mutations killed. Before
doing anything else:

1. Read `DECISIONS.md` from "Cockpit wrapper, Build A: built, reviewed, fix pass verified
   twice" to the end, then `docs/COCKPIT-WRAPPER-DESIGN.md` §7 (the mouse), §11 (the terminal
   guard), §12, §13's Build B tests, §14's Build B order, and §R3–§R5 (they govern where they
   differ from the text above them: no blank chat strip; Accept from a unit opens the attempt;
   dialog focus never wraps; the quiet time also runs from the first full draw).
2. Read the code the mouse plugs into: `crates/harness-tui/src/termguard.rs` (every enable
   under the guard — the mouse modes are one more), `main.rs` (suspend/resume around the
   editor, the signal path, the panic hook, `park`), `view.rs` (`App::hits`: every clickable
   region is already recorded as it is drawn — rows, panes, menu items, dialog buttons, hint
   entries, activity buttons), `dialog.rs` (a click on a button acts only once armed),
   `app.rs` (keys → `Command`; the mouse is one more input to `on_input`). Note from §R5: a
   dialog acts only on a frame big enough to show it (`Dialog::usable`), and its button hit
   rects are recorded unclipped — clip them before clicks act on them.
3. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~695 tests,
   including the pty tests in `crates/harness-tui/tests/signals.rs`).
4. Baseline only if the CLI or the scanner changes (Build B should touch neither): the bench
   check needs the gitignored `targets/tractor/.scorer-vendor/` and `.bench/` (copy with `cp
   -cR` from another worktree) and a quiet machine; expect `198 reproduce (1 conformant, 197
   drifted), 2 expected divergence(s), 0 problem(s)` and `bench check: OK — no regression`.

Then build **Build B** in the design's order (§14), each step committed when green:

1. Mouse modes through the guard (our own `Command` writing `?1000h ?1006h`; crossterm's
   `DisableMouseCapture` on every exit path — quit, error, panic hook, signal path, `suspend`
   before the editor; re-enabled in `resume` unless dying), STARTING with their pty restore test
   (mouse-off sequences after a SIGHUP and around the editor; the VT interpreter in
   `signals.rs` reads the screen; assert the reset comes LAST).
2. Gestures from the hit record (§7): left click focuses and selects; double click = Enter
   (two presses on the same row within 400 ms, timed by the cockpit; a pair across a stalled
   loop iteration is discarded); the wheel scrolls an open menu or dialog, else the pane under
   the pointer; hint-bar entries and buttons do what their key does (Quit opens the quit
   dialog); a click outside a menu closes it, outside a dialog does nothing; a dialog button
   acts only once armed; a drag shows the "hold Option / Shift" hint; Help's "Mouse on/off";
   `--no-mouse`. Mouse events go through `on_input` (they restart a dialog's quiet time).
3. Review (3–4 lenses, independent verifiers), fix pass, VERIFY THE FIX PASS (Build A needed
   two rounds), mutation checks (the named rule: the mouse is off on every exit path; a click
   never runs an unarmed dialog), DECISIONS handoff, commit and push.

Separately suggested (their own tasks): the crate content hash skips files outside `src/`
(SAFE-5 of the design review); harness-detect's walk follows symlinks out of `source_dir` (now
a one-line switch to `walk::confined`); `verify`'s R6 gate; the harness-core ledger test that a
fork in the same test binary can fail (DECISIONS). After the wrapper: the chat pane (spike
first; it takes the right side from 150 columns — reserve it then) with harness-mcp's requester
label and a "migrate this" skill; then the feature-workflow view and the C-vs-Rust performance
baselines, then the briefing's M5 (external detector plugin + `EXTENDING.md`). Carry-forwards:
§16 escalation automation + `harness usage`; the `crash-timeout` classifier; a Linux sandbox;
the two deferred replay items; driver-attempt Accept; queueing on lock contention; an async
client for cooperative cancellation; per-function verdict dots; bounded reads in harness-core
(MCP-DESIGN §R TRUST-4); harness-mcp's revisit triggers.

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
  reviewed commit with `git show <sha>:<path>` so fixing can proceed meanwhile.
* Driving the cockpit headless: a Python `pty.fork()` driver with a small VT interpreter (the
  one used on 2026-09-25 rendered first-run Scan → Plan and a Re-check); no tmux here, no
  `timeout` (use `perl -e 'alarm N; exec @ARGV'`).
* A mutation whose test hangs or is killed (OOM) counts as killed — make such tests fail fast
  (a bounded wait on a channel) rather than hang. A read blocked on a FIFO cannot be released
  reliably: such a test needs a watchdog thread that calls `process::exit(1)` (see
  `a_fifo_in_the_crate_never_freezes_the_menu`). A script's timeout kills `cargo`, not the
  hung test binary under it — check `ps` for leftovers before rerunning.

Process that works (keep it):

* Run a time-boxed research spike in subagents, and verify its premise by running it end to end.
* Write the design, give it an adversarial design review from 3–4 lenses, verify every finding
  against the code, revise — then CHECK THE REVISION.
* Implement against the reviewed spec, then run an adversarial code review whose findings are
  verified against the code — then VERIFY THE FIX PASS the same way, and check each further
  pass (Build A: the first check found 12 new defects, one medium found by all three checkers;
  the check after that, 7 more, 3 medium).
* In the fix pass, add regression tests that fail without the fix, and mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test.
* Real end-to-end tests find what reviews miss.
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
