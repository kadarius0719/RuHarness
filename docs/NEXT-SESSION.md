# Next session — kickoff (2026-10-21)

This is the current plan; no other file holds one. Older kickoffs and build trails were cut on 2026-10-18 and again on 2026-10-21 (git history
keeps them; every result in them is in DECISIONS.md, whose index at the top finds each).

## Where things stand (2026-10-21)

The whole journey runs inside the cockpit on a real program: on liblzg the person mapped the
project, accepted its three programs, scanned and planned one, had the chat make a test program
and the Rust for two pieces, accepted them, and had the chat propose a features file — every step
from the screen, with the judge green at each. The project map (2026-10-08), the documentation
for a non-technical reader (2026-10-09 to 2026-10-18) and eleven cockpit passes (2026-10-18 to
2026-10-21, below) are on `main`; 1 820 tests; `bench check --replay` green on 2026-10-21 after the
judge's beyond-leaves pass (no regression). The leaf cap is gone (work item 2, done 2026-10-21): a piece that calls other
pieces moves after them, tested with them as the program stands; only the piece holding `main`
stays C, which the end-state design will settle. The person's working rhythm since 2026-10-20: they walk the cockpit and
send screenshots; the session triages each into a fix made at once, an item for the pass in
flight, or a design.

## The person's rules that matter for resuming

- Prove before planning: no new design until something has been run. The person's own run decides
  what comes next.
- Simplicity means usability: easy and intuitive, the command line first, the cockpit runs the same
  acts.
- Ask before any download; test downloads live only in `~/code/ruharness-test-downloads/`.
- Models and effort as in CLAUDE.md. Effort: medium for running and building; ask for high before
  judging what the person's run found or designing anything new, and for medium again after.
- Plain words in every report; commit to `main` and push at each milestone.

## Built since 2026-10-18 (all on main; the DECISIONS entry of each date says what and why)

- **2026-10-18, the cockpit made easy for a newcomer** (four passes; docs/reviews/2026-10-18-
  cockpit-usability-critique.md): one vocabulary, a Next step on every screen, the Project view
  replacing the text menu, the cockpit writing the build configuration itself.
- **2026-10-19, colours** (`crates/harness-tui/src/theme.rs`): fourteen roles, four themes, a
  settings file, rule tests over every golden; a hand-off pause reads "waiting", never "failed".
- **2026-10-19, model work through the chat** with a sign-in walk-through; an old `harness-mcp`
  beside a newer cockpit says "rebuild" with the line to paste.
- **2026-10-19, the Project view, revision 2** (COCKPIT-WRAPPER-DESIGN §9.3): one list, one
  card, one line; the map's reply carries a `why`.
- **2026-10-20, the guided path** (§9.4): the journey line of eight stages on every screen, stage
  words instead of planned/tried, **Make the test program — ask in chat**, refusals in words,
  dialogs on a panel over a backdrop, panes as wide as their rows (`files_width`); a newcomer
  check and a seventeen-item fix pass the same day (pieces that stay C, a waiting try, the
  chat-off way spelled as commands, guide section 12D).
- **2026-10-20, explain anything** (§9.5): the hint line under the panes, the `i` card on every
  row and in every dialog, dialogs of three lines, narrow menus, the chat's lines folded.
- **2026-10-20, a model proposes the features and the workloads** (§9.6): Propose items on the
  Features and Speed rows; harness-mcp writes a checked proposal file only; Write opens it as
  the draft; the Features and Speed views show the proposal before the editor opens.
- **2026-10-20, a readable default** (§9.7): the theme chosen by asking the terminal its
  background (`crates/harness-tui/src/background.rs`), title bands, the section colour on
  headings only; `--theme terminal` keeps the earlier look.
- **2026-10-21, file inputs and the chat's fix** (FEATURES-DESIGN §14): a scenario's input may
  be a small file in the project; `input-unusable` as a reason; **Fix the features file — ask in
  chat**; the chat's brief knows which scenarios cannot be checks.
- **2026-10-21, the cockpit after the whole-journey check** (eighteen snags; DECISIONS of that
  date): the plan's summary yields to the hint line at the first key; a unit says what the
  features ran when only a scenario that cannot run keeps the map incomplete; one count about
  moving (the pieces that can move are the denominator); a test program waiting for its answer
  is a waiting piece; the tool's journey stays on its earliest unfinished stage; Open tool keeps
  the theme's why and `files_width`; the Project view ranks a tool with work ready first;
  dialogs of three lines for the build configuration and the adoption question; the `i` card
  ends with the pane's Next line; greyed reasons never cut; on screen every word is **piece**
  and **try** (the commands and files keep unit and attempt); a feature row never cuts its state
  word. Walked on a liblzg copy at 120x40 and 100x30.
- **2026-10-21, beyond the leaves, pass B** (BEYOND-LEAVES-DESIGN §5–§6; DECISIONS of that
  date): the cockpit says `holds main` only for the piece holding `main`, counts every other
  piece among those that can move, names the first in the plan's execution order, says on a
  caller's card what it is checked with, names the callee that changed, and names a callee
  linked as Rust whose own check is out of date; `state status` names the main holder on its row,
  ends with "the program as it stands: 3 of 4 pieces in Rust (u-lzg holds main)" and says
  `out of date: a piece it calls changed` with no contradiction marker; the features' program
  digest folds in the other accepted pieces' Rust; the guide's Part 7 moves `u-encode` on a side
  branch, its screens run with this build; TUTORIAL "The order pieces move in", the judge,
  drivers and cockpit topics, FIRST-HOUR §3's last line, SCHEMAS.

Still open from these, for the person: the live chat tests once (`crates/harness-tui/tests/
chat_live.rs`, both hosts; the brief and the tool list changed many times); a live "Fix the
features file" on t-benchmark and a decode scenario with a real `.lzg` file on t-unlzg. For a
session: the tool's screen still has prose where the Project view has a card (the Back row's
pane, the unit views' "Heading:" lines) — the next cockpit design under §9.3's three rules; a
newly started cockpit does not remember that a hand-off is waiting; a terminal that answers the
background query late (slow ssh) could have its reply read as keys (no DA query follows it).

## Done (2026-10-21): beyond the leaves, pass A

The judge tests a piece that calls other pieces (docs/BEYOND-LEAVES-DESIGN.md §1–§4; DECISIONS
2026-10-21, "Beyond the leaves, pass A"): every build links the callees as the program stands, a
failed link is a red check, the verdict and the driver record carry `dependencies` (a callee's
accepted Rust makes its callers `STALE: dependencies`), migrate refuses only the piece holding
`main`, the prompt names the callees' prototypes and the `extern` block may declare them,
capabilities has the import rule, the whole program links every accepted piece, and symbol-set
ignores LLVM's promoted constants. On a copy of the spike's liblzg, `verify u-encode` is green with
no spike edits. The live walk on a fresh copy moved u-checksum, u-encode and u-version in order
(DECISIONS 2026-10-21, "proven on a fresh copy"), and pass B (above) closed its four gaps.

## The work, in order

1. **The person's own cold-start run.** docs/FIRST-HOUR.md as written (liblzg), then lz4 through
   the README's "move your own C program" door alone, past one unit, ideally to several promoted
   units, with every stumble written down. Step (g) of the map (record the first accepted tool's
   features before its first unit moves) happens inside it. Fix what the run shows first.
2. **Past the leaf cap: pieces that call other pieces — done 2026-10-21** (docs/BEYOND-LEAVES-
   DESIGN.md; pass A the judge, the live walk on a fresh liblzg copy, pass B the cockpit, the
   command line and the docs). **Open: the end-state design** — the piece holding `main` (today
   "holds main — stays C in this version") and the combined crate (one staticlib per program,
   with LTO: 381 KB against 1.42 MB in the spike). Design it with item 3.
3. **The last stage: "the program works".** Build each program with its Rust in place, run its
   own test suite and the person's features on that build, and say so on the journey line ("the
   program works with 2 of 4 pieces in Rust"). The whole-program check, the features and the
   speed workloads exist; what is missing is this stage as a step of the journey, in the cockpit
   and on the command line, and the mixed build as a thing the person can run (success criterion
   1 of the brief). Design with item 2; build after it.
4. **`harness answer` and a live provider on the command line.** `harness answer <unit> <file>`
   wraps a plain file into the hand-off's envelope (the idea in DECISIONS 2026-10-15; decide its
   shape with the person first). A live provider: the `anthropic` and OpenAI-compatible adapters
   exist, but no cloud endpoint has been used (DECISIONS 2026-09-19) and there are no keys here;
   the person decides whether and how.
5. **The scale run on lz4**: many units of one lz4 program moved, promoted and kept green, to see
   what breaks at size (time, disk, the small open items below that are about speed) — after
   item 2, or it stops at the leaves.
6. **Then the briefing's next milestone (M5, the extension proof)**, or what the scale run shows,
   as the person decides.

## Two ideas the person raised on 2026-10-20, to design after the explain-anything pass

- **Beyond "behaves the same": improve the Rust.** The person's next question after a green
  port: can this be better — idiomatic, no unsafe, faster, readable? The judge is the safety
  net that makes this safe to ask: an **Improve — ask in chat** act on a done piece is a steered
  attempt (harness_steer exists: a new try seeded from the green one with a note, judged like
  any other), with a brief section on what "better" means and the order to try it (safety
  first, then clarity, then speed), Speed comparing the two tries, and the chat saying in plain
  words what it changed and why — the learning tool the person wants. A ninth stage of the
  journey, after Accept Rust: `Improve` (optional, never required). Design it with the
  "program works" stage; it needs nothing new in the judge.

## Small open items (rechecked against the code on 2026-10-18)

- The ask prompt still asks for fields "in this order" (crates/harness-llm/src/projectask.rs);
  nothing checks the order. Keep it, or drop it and re-record the trace keys.
- Ask's answer line says "in <program> it did not link"; it could add "when the map was made" like
  its neighbour.
- `project accept` link-checks every program again (as long as a map); the link check compiles its
  own objects instead of reusing the map's.
- The map's sandbox profile does not yet limit its reads to an allow-list (PROJECT-MAP-DESIGN §8).
- `state status` builds an include resolver per unit without caching reads (slow on tools with
  many units).
- Attempts, driver validation, `perf`, `bench`, `promote` and `gen-driver` still hash the facts'
  include closure, not the resolver's (safe direction: an attempt shows unbound).
- About 70 recursive tree walks remain in harness-scan (lint, interface, mutate).
- The driver-validation record carries no configuration entry.
- The committed B01 `read_scalefactors_lib` verdict holds the old boundary digest (status does not
  compare it).
- A test in crates/harness-tui/src/view.rs pins a made-up "all 8 checks passed" idle line.

## Once per checkout

The adoption tokens are not committed, so a fresh checkout or worktree asks once:
`harness state status --target targets/zopfli --adopt`. The bench adopts its suite by itself
(`bench check … --adopt`).

## Environment

- The harness binaries are not installed: put a worktree's `target/debug` first on PATH.
- `bench check --replay` needs the gitignored `targets/tractor/.scorer-vendor/` and `.bench/`
  (the person copies them with `cp -cR`; the agent is refused) and a quiet machine; expect
  198 reproduce, 2 expected divergences, 0 problems (about 28 minutes).
- How sessions work here (hand-offs, reviews, mutation checks, driving the cockpit headless,
  the live chat tests): docs/AGENT-BRIEFING.md §18.
- Known flaky tests under load; rerun one alone before calling it a failure:
  `chat::tests::an_exited_leaders_pipes_are_read_first`, `golden_dialog_before_and_after_arming`
  and `the_ready_line_invites_a_click_only_when_one_answers` (harness-tui), chat_e2e's
  `a_deaf_or_stopped_runtime_is_ended_on_quit` and `a_request_is_reviewed_run_and_answered`,
  harness-core's `a_reader_never_makes_a_writer_fail`.
- About 75 merged `worktree-agent-*` branches remain, each checked out in a worktree under
  `.claude/worktrees/` that the desktop app made; the app's own worktree cleanup removes them
  (a plain `git branch -d` refuses a checked-out branch).
