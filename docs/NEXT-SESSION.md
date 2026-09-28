# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **the feature-workflow view** (user direction, recorded 2026-09-23: user-
facing behaviours mapped to the code that implements them, so a migration preserves what a
user perceives, not only per-function correctness). Everything is on `main` and pushed. The
cockpit is done: the wrapper (Builds A and B: keys, mouse) and the chat pane (the §15 spike,
the design — docs/CHAT-PANE-DESIGN.md, §R–§R3 — Build C, the requester label, and Build D,
the pane: harness-mcp `--cockpit`, the `chat` module, the app's and the view's chat side, the
brief, the live test from both environments; reviewed from three lenses, 45 findings, §R7;
four fix passes, the first three checked, §R8–§R10; 148 mutations killed). Before doing
anything else:

1. Read `DECISIONS.md`'s last two entries (the chat pane) and the roadmap notes it points to
   (the M4 section's "roadmap notes", "Direction change (user)"), then
   docs/TUI-DESIGN.md §9 and docs/COCKPIT-WRAPPER-DESIGN.md §0–§2 (what the cockpit is for).
2. Read what a feature-workflow view would stand on: harness-detect (the detectors), the
   plan and its units (`harness-core::plan`), the scanner's call graph, the oracle's drivers
   and differential tests (`gen-driver`, `observe`), and the cockpit's tree and View
   (`crates/harness-tui/src/{tree,view,files}.rs`).
3. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~840 tests,
   including the pty tests in `crates/harness-tui/tests/{signals,chat_e2e}.rs`; the live chat
   tests are ignored unless `RUHARNESS_LIVE_CHAT=1`).
4. Baseline only if the CLI or the scanner changes: the bench check needs the gitignored
   `targets/tractor/.scorer-vendor/` and `.bench/` (copy with `cp -cR` from another worktree)
   and a quiet machine; expect `198 reproduce (1 conformant, 197 drifted), 2 expected
   divergence(s), 0 problem(s)` and `bench check: OK — no regression` (~29 min).

Then, as for every track: a time-boxed §15 research spike in subagents (what "a feature" is
here — entry points and the call paths under them, the drivers' scenarios, a person's own
names for behaviours — and how others map behaviour to code; verify the premise end to end on
`targets/zopfli` and `targets/tractor`); the design, an adversarial design review from 3–4
lenses, findings verified against the code, revised — then CHECK THE REVISION; build in
steps, each committed when green; an adversarial code review, fix pass, VERIFY THE FIX PASS
and each further pass; mutation checks of the named rules; DECISIONS handoff; commit, push.
After it: the C-vs-Rust performance baselines, then the briefing's M5.

Separately suggested (their own tasks): confine the oracle build sandbox's temp dirs (a
per-build temp dir — Build C's check, §R5 S-NEW-1); deflake the oracle's process-group
timeout test; the crate content hash skips files outside `src/`; harness-detect's walk follows
symlinks out of `source_dir` (`walk::confined`); `verify`'s R6 gate; the harness-core ledger
test that a fork in the same test binary can fail. Carry-forwards as in DECISIONS (the chat
pane's accepted items: `harness_request` re-reads up to 16 MiB per call; untested by design —
the hard-link fallback, the bench `(chat)` tags, a chat `.r2` replay, SAF-9's `dying` checks,
a KILL that cannot be sent; the chat protocol verified with Claude Code 2.1.274 only).

Environment (re-check; don't assume):

* There are no cloud API keys, so model calls go through the `external` hand-off.
* Answer hand-offs with plain Agent subagents, never Workflow agents.
* Set `--model` to the model that actually answers.
* Audit every batch with `targets/tractor/handoff-tools` before importing it.
* Never download a model without asking.
* The pty and e2e tests (`crates/harness-tui/tests/signals.rs`, `crates/harness-mcp/tests/e2e.rs`)
  need the `harness` binary next to theirs, built from the current sources (`cargo test
  --workspace` builds it; the MCP e2e refuses a stale one).
* The live chat tests (`crates/harness-tui/tests/chat_live.rs`) run the real `claude` on haiku
  under the person's sign-in (plan usage): `RUHARNESS_LIVE_CHAT=1 cargo test -p harness-tui
  --test chat_live -- --ignored --test-threads=1`, once as is (inside Claude Code) and once
  with `RUHARNESS_LIVE_CHAT_HOST=plain`. Run them after any change to the chat's protocol,
  environment or end routine.
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
  checkers each time; Build C: 38, then a low-medium and a design change, then three lows).
* A reviewer's "only an unsandboxed process could race this" must be checked against the
  sandbox profile (Build C: the build profile may write all of the temp dirs).
* In the fix pass, add regression tests that fail without the fix, and mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test.
* Real end-to-end tests find what reviews miss (Build B: a pty drive found the triple click
  before the review did; Build D: the live runs found the model writing its answer into the
  chat, and that a message typed mid-turn is queued, not folded).
* Tests of a terminal UI read a rendered screen (a VT interpreter), never the raw byte
  stream: ratatui skips cells that already show the letter.
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
