# Next session — kickoff (from 2026-10-08)

Resume RuHarness on `main` (everything below merged and pushed). Models and effort as in CLAUDE.md:
the main session may run on Fable, every hand-off on Opus; the session stops and asks for an effort
change at the switch points below.

**Where things stand (2026-10-07 evening, session ee312a, Fable):**
- The speed work's **fix passes 3, 4 and 5 are merged**: 1 312 tests green, fmt and clippy clean.
  Reports: docs/reviews/2026-10-07-perf-fix3-plan.md (the triage), -fix3-check.md (ten checkers,
  86 mutants, fix pass 4's triage), -fix4-check.md (three checkers, 48 mutants, fix pass 5's triage).
  DECISIONS' last entry says what changed in plain words and what was decided (no check round after
  the small fix pass 5; unreadable facts stay a whole-target refusal in the cockpit and harness-mcp;
  harness-mcp copies the 999-unit words; the own-build rule stays by location).
- **`bench check --replay` RUN after the merges** (the person copied the gitignored fixtures;
  the classifier refuses that `cp -cR` for the agent): 198 reproduce (1 conformant, 197 drifted),
  2 expected divergences, 0 problems, `bench check: OK — no regression`, 27 min 44 s. The fixtures
  now sit in this worktree (`targets/tractor/.scorer-vendor`, `.bench`).
- **The project-map design is at revision 2.1** (docs/PROJECT-MAP-DESIGN.md; the person answered
  every proposal as recommended under the rule "simplicity means usability": CLI first, the cockpit
  runs the same acts). The trail: the four-lens review and its triage
  (docs/reviews/2026-10-07-project-map-design-review.md), revision 1's check and triage (…-08-project-
  map-rev1-check.md), revision 2's re-check (…-rev2-check.md), revision 2.1's edits (§9 of the
  design). A final Opus reader over 2.1 was running at the time of writing; then the **build starts
  in §5's order** — (a) the walk and symbols over one folder, (c) the layout and the file-list
  target, (b) the whole-root map, (d) the model step, (e) accept and the cockpit's acts, (f) docs,
  (g) the features step — each committed green with §4's tests, then the usual review, fix passes,
  mutation checks. Building is medium-effort work (briefing §17); the design holds the judgement.
- **Part 11 of the testing guide on liblzg: DONE** (Steps 11.3–11.6 and the checkpoint, headless
  cockpit drive; the practice branch `practice-lzg` holds the committed rows, local only). The guide
  now uses a 900-copy input (the `fast` workload was too short to time with 300) and the real
  counts and times. Two small candidates seen, not fixed: the Measure dialogs' estimates read high
  (about 3 min for a run that took 87 s; 83 s for the program-as-it-stands run, about half the
  work); the help's Speed section leaves out "Measure the program as it stands".

**The work, in order:**
1. **Step (a) of the map's §5 is BEING BUILT** (launched 2026-10-08 from commit 3f54389, revision
   2.2): four Opus builders at medium effort in their own worktrees — merge each branch when its
   report says it is green, resolve conflicts by file ownership, run the full gates, then step (c):
   - `worktree-agent-ab93152a9e0742945` — (a1) MERGED: the walk's four additions (harness-core walk.rs,
     the callers) and the `--json` escape fix (harness-cli report.rs, SCHEMAS);
   - `worktree-agent-a367e9fa49de8404a` — (a2) adoption of a ledger made elsewhere (harness-core
     module + `TargetContext::load`, `--adopt` on every CLI command and `bench`, the cockpit's
     dialog, harness-mcp's refusal, the test helper and `$RUHARNESS_ADOPTED`, committed tokens for
     zopfli and the tractor suite, SCHEMAS' fixed-names table and writer row);
   - `worktree-agent-ac521dd6b4231da16` — (a3) MERGED (14cdbe1): the `objsyms` extension (kinds, weakness, commons;
     the `$`-suffix and identifier helpers);
   - `worktree-agent-ae9dd210f1e7a03c0` — (a4) the map sandbox profile renderer, the targetless
     runner extracted from harness-oracle bench.rs, the work folder and the children's `PATH` and
     `RUSTUP_TOOLCHAIN`/`RUSTUP_AUTO_INSTALL`, the unit-crate file check and `build = false` in the
     harness manifest (harness-oracle lib.rs/exec.rs/sandbox.rs, harness-llm migrate.rs).
   Still to build in (a) after these: the per-file facts and include folders over one folder and
   the `project map` command's first form (printing per-file facts; closures come in (b)). Then
   the usual review of (a)+(c) together, fix passes, mutation checks.
2. The briefing's M5 after the map's first accepted tool (DECISIONS 2026-10-08).

**Environment:** as the previous kickoff says (below). Test downloads only in
`~/code/ruharness-test-downloads/`; nothing installed; the binaries from a worktree's
`target/debug` on PATH. The pid/launcher tests are timing-sensitive under load: rerun one alone
before calling it a failure (known flakes under load: `chat::tests::an_exited_leaders_pipes_are_read_first`,
`golden_dialog_before_and_after_arming`, chat_e2e's `a_deaf_or_stopped_runtime_is_ended_on_quit`).

---

# Next session — kickoff (Fable, from 2026-10-07)

Resume RuHarness on `main` (everything below is merged and pushed). The main session runs on
**Fable**; **everything it hands off runs on Opus** (the person's rule, also in CLAUDE.md): Agent
subagents with `model: "opus"`; every Workflow `agent()` call with `{model: 'opus'}` — the default
inherits the main session's model, so set it on each call; hand-off answers by plain Agent
subagents with `model: "opus"` and `--model claude-opus-5-5`. Ask the person before giving any
handed-off work Fable. Fable's own job: triage, design judgement, the shape of each workflow.

**Effort** (the person, 2026-10-07). A session cannot change its own effort — only the person can,
in the app — so the session STOPS and asks at each switch point, in one plain line ("Please set my
effort to high now: the design review's findings are back and the judging starts"), and waits:
- **Start on medium**: triage of the check round, fix pass 3 and its check, Part 11, merges.
- **Ask for high** when the map design review's findings have come back — before triaging them and
  writing the revision (step 3 below).
- **Ask for medium again** once the revision is written and its check has been launched.
- Never ask for xhigh or max without saying why the work needs it.
Handed-off agents copy the main session's effort unless it is set on each call: Workflow
`agent()` calls get `effort: 'medium'` for mechanical stages (fixers, mutation runners) and
`effort: 'high'` for checkers, verifiers and judges; Agent subagents answering hand-offs get
`effort: "medium"`.

Read first: DECISIONS.md's last three entries (2026-10-07), then docs/NEXT-WEEK-PLAN.md "Status
2026-10-07". Check `get_usage` before each large workflow and say roughly what it costs. Plain words,
no review codes.

**Where things stand (2026-10-07, session 03b25fd0):**
- Speed work fix pass 2: all twelve leftovers fixed, merged, 1 251 tests green (DECISIONS); with the
  mutation checks' new tests merged, 1 264 tests green, fmt and clippy clean.
- Its check round FINISHED (workflow `wf_6812de33-51b`, 17 Opus agents, ~3.1M tokens): the
  report is **docs/reviews/2026-10-07-perf-fix2-check.md**. Checkers: 0 high, 6 medium, 22 low, not
  yet triaged — the mediums: without facts `perf show` lets a program-as-it-stands row read current
  (its units unchecked; both CLI checkers), while the cockpit and harness-mcp say every held unit is
  "left out now" (the two disagree); the new 999 cap on `left_out` can stop a real run on a plan
  over 999 units; the caps do not bound the parse itself (the fixer said so too); the Measure
  dialog can drop the launcher-cache line when the cockpit and the `harness` it runs are different
  builds. Mutation checks: 253 mutants — 185 killed, 52 survived and got a test (merged: the six
  "Merge the mutation checks' new tests" commits), 14 equivalent with reasons, 2 open (a ratio-end
  rounding the current code gets wrong on the same inputs; the dialog's 0.25 s wait, untestable as
  the probe is wired).
- The liblzg walkthrough: Parts 0–10 done and the guide corrected; Part 11 (speed) half done.
- The project-map spike: done (docs/PROJECT-MAP-INVESTIGATION.md "The spike on real downloads"); the
  person answered the design's open questions 1–4 and the new flags question (design §7, DECISIONS).

**The work, in order:**
1. **Fix pass 3** from the check round: triage its findings (keep what both checkers or the
   evidence confirm; say plainly what is dropped and why; the mutation tests are already merged),
   decide the two open survivors, fix the confirmed findings in worktrees (Opus fixers, a test each that fails without the
   fix), then a scoped check of that pass (Opus checkers). Candidates the fixers themselves noted:
   docs/reviews/2026-10-07-perf-fix2-check.md "Seen by the fixers, not in scope" and "Seen by the main
   session" (a cockpit test that leaves a looping child behind when it fails — eight were found
   running for a week and stopped).
2. **Part 11 of the testing guide on liblzg**, on a quiet machine: the practice worktree
   `.claude/worktrees/practice-lzg` (branch `practice-lzg`, local only, never pushed; it holds
   `targets/lzg` with both units verified, features, `migration/perf/workloads.toml` saved and
   `bench/big.txt` made — Steps 11.1–11.2 done). Merge main into it first (Part 10's "After
   updating RuHarness"), `cargo build -p harness-cli -p harness-tui -p harness-mcp`, then
   `PATH="$PWD/target/debug:$PATH"` and Steps 11.3–11.6 (11.5 drives the cockpit: use
   devtools/cockpit-drive). Fix the guide where reality differs.
3. **The project-map design review** (docs/PROJECT-MAP-DESIGN.md draft 0 + §7's answers + the
   spike): 3–4 lenses, Opus reviewers, every finding verified against the spike's evidence and the
   code by Opus verifiers; Fable triages and revises; then CHECK THE REVISION. The spike's "What
   the spike changes" list is the first input. No building before the revision is checked.
4. Then: weigh the briefing's M5 against building the map.
5. At the next merge to main: `bench check --replay` (not run for fix pass 2; it needs the
   gitignored `targets/tractor/.scorer-vendor/` and `.bench/` copied with `cp -cR` from
   `.claude/worktrees/rust-migration-harness-7d1c42`, and a quiet machine; expect 198 reproduce,
   2 expected divergences, 0 problems).

**Environment (re-check; don't assume):**
- Test downloads live only in `~/code/ruharness-test-downloads/` (liblzg, lz4, lzg-practice): never
  installed, never on PATH, compiled only inside that folder or the harness's sandboxed builds; ask
  before any new download. An `lz4` in /opt/homebrew/bin predates this and is not ours.
- The harness binaries are not installed (`~/.cargo/bin` has none): put a worktree's
  `target/debug` first on PATH instead of the guide's `cargo install`.
- The cockpit's chat uses the person's default Claude Code model (seen: claude-opus-4-8, then
  claude-opus-5-5); the guide never passes `--chat-model`.
- devtools/README.md: the headless cockpit driver (dialogs ignore keys for a moment — wait for their
  own `ready:` line) and the project-map closure script.
- Live chat tests were not re-run (no change to the chat's protocol, environment or end routine).

---

# Next session — kickoff (for the week of 2026-10-07)

Paste the Agent Briefing first, then:

Resume RuHarness on branch `claude/rust-migration-harness-f7665f` (worktree
`.claude/worktrees/rust-migration-harness-7d1c42`; also on GitHub). Main is at 56cd090. The
C-vs-Rust speed work is built, reviewed (82 findings) and fixed (fix pass 1 merged, all tests
green) but not yet re-checked, mutation-checked or merged. UPDATE 2026-10-03: the speed work is MERGED TO MAIN (9af3fcf); the 9 high
fixes were re-checked (all clear) and verify's sandbox gap closed. Follow docs/NEXT-WEEK-PLAN.md
in its revised order: fix pass 2's leftovers (items 2–13), the mutation checks, then the liblzg
walkthrough of docs/TESTING-GUIDE.md end to end (ask before downloading), then one real-project
spike for the map, then — and only then — the design review of docs/PROJECT-MAP-DESIGN.md (ask
the person its §7 questions first).
Check `get_usage` before each large workflow and say what it will cost. Plain words, no review
codes.

---

# Next session — kickoff (2026-09-30)

Paste the Agent Briefing first, then:

Resume RuHarness. Read `DECISIONS.md`'s last entries and `docs/FEATURES-PROGRESS.md` "Open" —
its list is this session's work, in order: collect the code review of the compiler-guided
probe build (and the §8 premise re-run), fix what it confirms with tests, check each fix pass,
mutation-check the named rules; then `bench check --replay`, the live chat tests, and the
C-vs-Rust performance baselines design. Report to the person in plain words — no review codes
(M1, N4, …). A from-zero testing guide for the person is at docs/TESTING-GUIDE.md (liblzg).

---

# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness — **finish the feature-workflow view, then the C-vs-Rust performance
baselines**. Everything is on `main` and pushed. The features track is built, reviewed (35
findings, 33 confirmed) and fixed in four passes, each checked; 87 mutants (86 killed, 1
equivalent); 952 tests. It paused before the check of the fourth pass. Before doing anything
else:

1. Read `DECISIONS.md`'s last entry (the feature-workflow view) and docs/FEATURES-PROGRESS.md —
   its "Open" list is this session's first work, in order: two low fixes (a top-level `.c`
   linked out of `source_dir`; a FIFO named `x.c`), a scoped check of the fourth pass (and a
   fix pass for what it finds, mutation-checked), `bench check --replay`, the live chat tests.
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~952 tests; the
   `harness-core` ledger test `a_reader_never_makes_a_writer_fail` can fail under load and
   passes alone). The pty and e2e tests need the `harness` binary built from current sources
   (`cargo test --workspace` builds it).
3. The live chat tests run the real `claude` on haiku under the person's sign-in:
   `RUHARNESS_LIVE_CHAT=1 cargo test -p harness-tui --test chat_live -- --ignored
   --test-threads=1`, once inside Claude Code and once with `RUHARNESS_LIVE_CHAT_HOST=plain`.
   Ask the person to sign in to `claude` first if the chat says the session expired.
4. The bench check needs the gitignored `targets/tractor/.scorer-vendor/` and `.bench/` (copy
   with `cp -cR` from another worktree) and a quiet machine; expect `198 reproduce (1
   conformant, 197 drifted), 2 expected divergence(s), 0 problem(s)` and `bench check: OK — no
   regression` (~29 min).

Then the C-vs-Rust performance baselines (roadmap), as every track: a time-boxed §15 research
spike in subagents, the design, an adversarial design review from 3–4 lenses with findings
verified against the code, revised — then CHECK THE REVISION; build in steps, each committed
when green; an adversarial code review, fix pass, VERIFY THE FIX PASS and each further pass;
mutation checks of the named rules; DECISIONS handoff; commit, push. After it: the briefing's
M5.

Separately suggested (their own tasks): confine the oracle build sandbox's temp dirs (a
per-build temp dir — Build C's check, §R5 S-NEW-1); deflake the oracle's process-group
timeout test; the crate content hash skips files outside `src/`; harness-detect's walk follows
symlinks out of `source_dir` (`walk::confined`); `verify`'s R6 gate; the harness-core ledger
test that a fork in the same test binary can fail (`a_reader_never_makes_a_writer_fail`,
seen again under load during the features track). Carry-forwards as in DECISIONS (the chat
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
* A checker that runs a fix against a real corpus finds what unit tests miss (the features
  track: the probe rule re-run over 294 C files — sqlite, oniguruma, tree-sitter — showed 31
  ordinary functions lost; a folder with a space turned a whole check off). Each of the four
  fix passes' checks found real issues in the pass before it.
* Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
