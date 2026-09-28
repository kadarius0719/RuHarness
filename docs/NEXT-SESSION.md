# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **the chat pane's Build D: the pane itself**. Everything is on `main` and
pushed. The chat pane's §15 spike (Claude Code headless, verified end to end), its design
(docs/CHAT-PANE-DESIGN.md: 87 → 71 → 18 findings, §R–§R3) and its Build C (the requester
label — the harness side of chat: `requester: chat`, `traces/chat/`, the CLI's
`--answer=- --answer-bytes=N --answer-key=K`, harness-mcp's labelled acts and
`harness_request`) are done: Build C reviewed (38 findings, §R4) and fixed in three passes
(§R5, §R6); 752 tests green; 44 mutations killed; the bench unchanged. Before doing anything
else:

1. Read `DECISIONS.md` from "Chat pane: §15 spike" to the end, then docs/CHAT-PANE-DESIGN.md
   whole — §0–§11 are the spec; §R–§R6 govern where they changed it (above all: an answer
   travels on the CLI's stdin, never in a file — `Running::spawn_with_input`, framed by
   `--answer-bytes`). The Build B rules still hold (docs/COCKPIT-WRAPPER-DESIGN.md §R6–§R8: a
   click answers a dialog only once it is armed, shown armed and a second old).
2. Read the code Build D plugs into: `crates/harness-tui/src/{app,view,main,spawn,dialog}.rs`
   (focus, the armed dialog, the loop, the signal path, the child slot), `crates/harness-mcp`
   (`--cockpit` mode is Build D's first step), and the spike's recordings and drivers (the
   design's §1 names them).
3. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~752 tests,
   including the pty tests in `crates/harness-tui/tests/signals.rs`).
4. Baseline only if the CLI or the scanner changes: the bench check needs the gitignored
   `targets/tractor/.scorer-vendor/` and `.bench/` (copy with `cp -cR` from another worktree)
   and a quiet machine; expect `198 reproduce (1 conformant, 197 drifted), 2 expected
   divergence(s), 0 problem(s)` and `bench check: OK — no regression` (~29 min).

Then, in the design's order (§10), each step committed when green:

1. harness-mcp `--cockpit` (no harness binary at all; the fence and `valid_model` move into
   harness-tui's library), then new recordings of Claude Code with the exact argv.
2. The `chat` module: the runtime child (resolve, environment, its own process group, the
   stale-dir sweep — own 0700 dirs only, "gone" only from `kill -0` under `LC_ALL=C`, §R5),
   the stream parser and state, the end routine; a shell-script fake `claude` replaying the
   recordings; pty tests.
3. `app`: focus, input, the chat's requests mapped onto the cockpit's own acts (the armed
   dialog, the argv shown), outcomes answered as `deny` + the outcome, the hand-off table
   and Continue (the answer on stdin), the continuation permission, the typing guard, the
   chat-dialog rules (a no-letters flag in dialog.rs).
4. `view`: layout (three columns from 156; below, the chat only while focused), the tab strip,
   the transcript, request and waiting lines, hits, Help; the brief
   (`crates/harness-tui/src/chat_brief.md`, by `--append-system-prompt`); README.
5. The live test by hand, from a plain terminal and from inside a Claude Code session.

Then: an adversarial code review from 3 lenses, findings verified against the code; fix
pass; VERIFY THE FIX PASS (Build C: the check found a low-medium and the reason to drop answer
files altogether) and check each further pass; mutation checks of the named rules;
DECISIONS handoff; commit, push.

Separately suggested (their own tasks): confine the oracle build sandbox's temp dirs (a
per-build temp dir — found in Build C's check, §R5 S-NEW-1); deflake the oracle's
process-group timeout test; the crate content hash skips files outside `src/`;
harness-detect's walk follows symlinks out of `source_dir` (`walk::confined`); `verify`'s R6
gate; the harness-core ledger test that a fork in the same test binary can fail (keep forks
out of harness-core's lib tests). After the chat pane: the feature-workflow view and the
C-vs-Rust performance baselines, then the briefing's M5. Carry-forwards as in DECISIONS
(Build C's accepted items: `harness_request` re-reads up to 16 MiB per call; the hard-link
fallback, the bench `(chat)` tags and a chat `.r2` replay untested).

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
  checkers each time; Build C: 38, then a low-medium and a design change, then three lows).
* A reviewer's "only an unsandboxed process could race this" must be checked against the
  sandbox profile (Build C: the build profile may write all of the temp dirs).
* In the fix pass, add regression tests that fail without the fix, and mutation-check the
  rule-guarding ones with a script that reverts each fix and runs its test.
* Real end-to-end tests find what reviews miss (Build B: a pty drive found the triple click
  before the review did).
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
