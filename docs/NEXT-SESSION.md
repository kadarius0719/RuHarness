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
   item 2, or it stops at the leaves. Under way since 2026-10-27 (docs/SCALE-RUN-LZ4.md):
   `u-timefn` in Rust and green; three walls fixed (DECISIONS 2026-10-27 and 2026-10-30: the
   `literal-zero` mutant, one symbol per `#if`-branched name, `--retry` on Make the test
   program) and a fourth designed and built the same day (docs/HEADER-FUNCTIONS-DESIGN.md:
   helpers written in headers — `u-util` was never a piece); the walls still open are listed in
   DECISIONS 2026-10-30 (first entry) — `util.c`'s row, the per-platform `#if` design,
   test-program tries invisible on the card, the chat's default model. `u-lorem` is in Rust.
   A fifth wall fixed the same evening (DECISIONS 2026-10-30, third entry): the runtime's own
   output-limit message, read as a stranger's; the output ceiling is 128 000 now. The run log
   the person asked for on 2026-10-30 is designed, reviewed and built (2026-11-07,
   docs/RUN-LOG-DESIGN.md; DECISIONS of that date): `L` opens it, `migration/build/cockpit.log`
   keeps it, the status row says how long the chat has been thinking. Not yet seen on a real
   run: the person's next lz4 session is its first. A sixth wall was found and fixed by the
   gates the same day (DECISIONS 2026-11-07, second entry): after the Xcode update of
   2026-11-05 the map found no program on any project (the SDK's `SDKSettings.json` read as a
   file outside the project) — rebuild before mapping or accepting anything.
   Next on the run, top to bottom: `u-lz4`, `u-lz4hc`, `u-threadpool`, `u-xxhash` (expect the
   `-D` rename at the symbol-set check), `u-lz4frame`, then `u-bench` and `u-lz4io` — the first
   pieces with header helpers, the proof of the fourth fix.
   **2026-11-08:** the log's first real evening found the seventh wall (DECISIONS 2026-11-08):
   the chat refused u-lz4hc's test program because the plan, older than the October 30 scanner
   fix, listed static helpers as callable; a Refresh then kept the old signatures and blocked
   u-xxhash. Three fixes landed the same evening; the person's steps are Scan, Refresh the plan,
   then u-lz4. **First design item for this session:** `LZ4HC_hexOut`, a function the source
   defines only under a debug `#if`, is still in u-lz4hc's list and the judge requires every
   listed symbol to be called — decide between preprocessing with the map's compile flags and
   letting the judge require only what the compiled unit defines (DECISIONS 2026-11-08, "Left
   open"). Known for later: the scanner reads `XXH_PUBLIC_API XXH_errorcode XXH32_update(…)` as
   a function named `XXH_errorcode`, so u-xxhash lacks its two `_update` functions.
   **2026-11-11, the eighth wall, small:** after the Refresh, u-lz4 still read "waiting for
   your answer" because of a try from October 30 (attempt d-8378804af048, request bec63844,
   never answered) posed under the old contract. A waiting piece offers no Make the test program
   and the Next step says to answer the old file: the cockpit has no way out of a stale hand-off
   (the person's rule: a refusal must offer the way out). The person removed the try by hand this
   time. **Second design item:** a "Drop the waiting try" action on a piece whose in-progress
   try's prompt digest no longer matches today's prompt (or on any waiting try the person
   abandons), which removes its attempt record and its unanswered request; and the waiting state
   should come from the try's record, not from "newest request without a response", so a stray
   request cannot hold a piece waiting forever. An Opus task with a brief, after u-lz4's result.
   Also 2026-11-11: u-lz4 went through whole (test program accepted, Rust green on turn 2, waits
   for Accept), and the wheel burst that froze the cockpit is fixed (DECISIONS 2026-11-11). Start
   the cockpit with `--chat-model opus` from now on.
   **2026-11-19:** the wheel fix held in the person's hands and u-lz4 is accepted: 3 of 9, five
   checks green (the whole-program check not set up). Next is the `LZ4HC_hexOut` decision, then
   u-lz4hc.
   The decision is taken and built the same day (DECISIONS 2026-11-19): the plan leaves out a
   function the build does not compile (`not_built`), and flags a piece whose build names differ
   (`build_only`). The person's steps: rebuild, Refresh the plan (expect the lines for u-lz4hc,
   u-xxhash and u-lz4io), then u-lz4hc's test program. u-xxhash stays for the rename work.
   **2026-11-21:** the refresh did what it should in the person's hands (u-lz4hc offers its test
   program, the card says `LZ4HC_hexOut` was left out). The person found the piece card a wall
   of text; it now comes in titled groups (DECISIONS 2026-11-21). The file card and the function
   card can follow once the person has seen it. Still next: u-lz4hc's test program, then its Rust.
   **2026-11-25:** the outer loop is proven on a copy (DECISIONS 2026-11-25): the whole-program
   check and docs/lz4-features.toml's 15 scenarios pass with the three accepted pieces, after a
   ninth wall in `features map` was fixed. The person's steps: rebuild; turn the check on in
   t-lz4cli's harness.toml; verify u-lz4; make the two lz4 samples from NEWS; save the features
   file; map; verify the three pieces. Then u-lz4hc as before. The map's finding to design
   from: the program runs 9 of u-lz4's 49 functions, so the project's own library tests are the
   next source of real use.
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

## Ideas from rust-anything (read 2026-11-17), to take one at a time

The person found https://github.com/erkamyaman/rust-anything (MIT, one author, published
2026-11-16): record-and-replay checking with bridge generators for six languages; the agent
writes the Rust. Its C path ports only scalar and `const char *` functions, so it could move
almost nothing of lz4; it is not a replacement for the harness and is not installed here (its
`setup` rewrites Claude Code's settings file). What is worth taking, in the order to take it:

1. **A trap catalogue for the test program's inputs.** It feeds every function the same list of
   semantic traps by parameter type: non-ASCII and emoji text, empty strings, `-0`, NaN, large
   and tiny floats, integer overflow edges, negative division and modulo. For C add NULL, zero
   lengths, sizes at the type's edge. The driver prompt could carry the list, and the judge could
   check the driver reaches each trap its parameters allow, instead of trusting the model's
   imagination each time.
2. **Two gate rules for the Rust.** (a) No `unwrap`, `expect`, `panic!`, `unreachable!` or
   `todo!` in a function whose C returns an error code, unless marked `// INFALLIBLE: <why>`; a
   panic across `extern "C"` aborts, so this matters twice for us. (b) Every error path of the C
   (each distinct error return) must be reached by at least one recorded case, or the gate says
   which is not.
3. **Run twice, sort the noise, never accept it silently.** It runs the original twice and sorts
   the differences into three kinds: changed values (times, ids: wildcards), the same lines in
   another order (compared in any order, but the port must produce exactly those lines), a
   different number of lines (a stretch that matches anything). Each kind is listed for a human;
   normalizing goes in the case, never in the port. For when the lz4 run reaches `lz4io.c` and the
   bench program, whose output has timings.
4. **One number for the whole-program journey: the fallback rate.** In its whole-app mode a Rust
   front door owns the entry point and hands unported commands to the original; the share the
   original still answers only goes down, and the final lift refuses until it is zero. Our
   cockpit's header could show the share of the program answered in Rust, and Accept Rust on the
   `main` piece could be the lift, refusing while any piece falls back.
5. **A speed verdict, not just numbers.** `bench` times the original and the port, bridge
   included, and ends in a verdict: ship, keep the slow functions in C, or port a bigger unit.
   "Do not ship a port that makes callers slower without saying so." The Speed item of the
   roadmap should end that way.
6. **Grilling in rounds, with a recommended answer each.** Its whole-app skill interviews the
   person in rounds: every question whose prerequisites are settled, numbered, with the
   recommended answer first and worded so "yes" takes it; decisions become numbered ADRs that
   are superseded, never edited; a glossary maps the original's terms to the Rust names so every
   unit names the same thing the same way. The guided workflow's Configure and Features stages
   could ask this way, and a glossary would keep the lz4 pieces consistent.
7. **A per-unit kill switch until the end.** Each wired unit can be sent back to the original by
   an environment variable until the lift removes the switch. Worth a thought when pieces are
   linked into the program (BEYOND-LEAVES): a way to run the program with one piece back in C.
8. **A one-case rerun loop.** `replay --case <name>` reruns one failing case in seconds, the loop
   for fixing a mismatch before the full run. Check the judge and the cockpit's failing-check card
   offer the same: one scenario, by the name the report gives it.
9. **A "what it can't catch" page.** Short and honest: inputs nobody recorded, regex engines,
   floats, anything not in the output. Our docs should have one, in the fast-path set.
10. **Faithful first, idiomatic after.** Translate with the original's structure and error paths,
    make it idiomatic only after the cases pass, then run them again. A line for the migration
    prompt, if it is not there already.

Not taken: its rule that a crash of the original on an input makes any port behaviour acceptable
(we would rather exclude the input and say so), and its C bridge shape (ctypes from Python, one
fork per case), which cannot reach buffer and state-struct APIs.

**From the person's side of it (read 2026-11-17, second pass: what it makes easier to use).**
It has no screen of its own: the person's coding agent is the interface, the tool gives it
verbs and verdicts, and `setup` registers the MCP server and a skill with the agent in one line
(reversible, with a dry run and a backup of every settings file it touches). What to take:

- **The agent as the front door, the cockpit optional.** A `harness setup` that registers
  harness-mcp and a skill file with the person's own Claude Code, so "migrate this with the
  harness" works in any session, the cockpit being the screen for those who want one. Most of
  the pieces exist (harness-mcp, the chat brief); the skill and the registration do not.
- **The same numbered verbs everywhere.** Its help text, its MCP tool descriptions and its
  skill all say "1. Map, 2. Bridge, 3. Record, …". Check that the cockpit's header stages, the
  CLI subcommands, the MCP tools and the docs use one numbering and one set of names.
- **A plan written for a person, with "Start here".** Its plan is a Markdown page: a ranked
  table of the best first units (safe, and worth it), each with a readiness score, its tests,
  and its risks counted by kind with `file:line`; a `--json` twin carries the same facts to the
  agent. Our walls (static helpers listed as callable, macro-made functions, a debug-only `#if`,
  pointer-heavy APIs) are the kind of thing a per-piece risk list would have named before the
  run. The cockpit's piece card could show those risks; `harness plan` could write the page.
- **One place the model writes, and "never overwrites" said in every command.** The agent
  edits one file per unit (`port.rs`); every generator refuses to overwrite and says so in its
  description. State the same for ours: what each stage writes, what it never touches.
- **Verdicts in words at the end of every step.** Its gate lists each check by name with one
  line of detail; its bench ends in ship, keep or port bigger; its lift refuses and names every
  blocker. Our Accept and judge output should read the same way on the card.
- **Explicit wait points in the skill.** "Wait for the person's approval of the plan and of the
  first unit"; "a mismatch is fixed in the port, never in the recording; if the difference is
  intended, ask first". Our chat brief should say its wait points as plainly.
- **Cases a person can read and edit.** Its recorded cases are plain files meant to be opened
  and changed by hand. Check our drivers' scenarios can be read the same way from the cockpit.

**Second read, 2026-11-19 (it is at 2.1.0: about a hundred commits, outside contributors, a
Prettier port 91.9% in Rust).** Its C path is unchanged (scalars and `const char *` only). New
ideas, the first two the ones that matter most to us:

- **Coverage of the original.** Its `record` measures which lines of the original the recorded
  cases ran, and `record` and `gate` print the lines never reached. For us: build the C with
  clang's coverage flags when the test program runs, and put on the piece card "the scenarios
  run N% of lib/lz4.c; never reached: …". It turns "same outputs as C ✓" from a yes into a
  yes-over-how-much, and it is the honest number for the report. The toolchain is already here.
- **Scenarios from the project's own tests.** `record --from-tests` runs the original's test
  suite and captures every call it makes to the unit's functions (twice what was copied by hand
  on its example). lz4 ships `tests/` (fuzzer, frametest, fullbench): the harness could wrap a
  piece's exported functions (the `-D` rename we already use), run the project's tests, and
  hand the captured calls to the test program as real inputs.
- **Stall detection.** `diff` keeps a short history per change to the port and reports a
  stalled port after three changes that gain no matching case; the skill says stop and ask.
  Ours counts repairs; counting progress (checks or scenarios gained per turn) is the better
  stop rule, and it is a stop kind for Do it All.
- **A known-differences file.** Cases where the port is meant to differ, each with the exact
  result it gives and a reason; they pass only with exactly that result, and the gate fails on
  an entry that no longer differs. The honest home for a deliberate difference (a fixed bug, an
  input outside the C's contract), instead of excluding it silently.
- **A status page: verified against tested.** "Verified": run end to end on real code someone
  else wrote, named and dated. "Tested": fixtures only. Per stage and per command. Ours would
  say which stages lz4, liblzg and the tractor cases have proven and which only fixtures have.
- **A pitfalls page loaded by CLAUDE.md.** Each mistake already made, with the rule that
  prevents it, added in the same change. Shorter to recall than DECISIONS (ours: a pipeline
  hides an exit code; never the full suite while the cockpit is mid-turn; a plan older than the
  harness), and the page to hand every Opus brief.
- **A doctor.** Runtimes and versions, the agent's setup, a self-test, each problem with its
  fix; the bug form asks for its output. The sixth wall (clang 21 and the SDK's settings file)
  was a doctor-kind problem.
- **Every failure carries its next step, structured.** Each failed gate check names the command
  that fixes it; a failed MCP call returns code, message, retryable and next. Check harness-mcp
  hands the chat the way out as a field, not only as prose.
- **One option set for the CLI and the MCP tools, kept in step by a test; the tools page
  generated from the tool list.**
- **Platform-specific C as a named risk** (inline assembly, `volatile`, bitfields, packed
  layouts, compiler extensions, integers cast to addresses): "a recording on this machine says
  nothing about the target". For the per-piece risk list.
- **Smaller ones:** a size check (the port fails above three times the original's lines); a
  test that an unported crate passes no case (a panic must not match an error); every outside
  program in its own process group with stdin closed and a time limit, a runaway case dropped
  and listed with the rest kept; files a piece writes compared as output (for `lz4io.c`); the
  report's headline says how much runs in Rust, not only that nothing mismatches.

## From Anthropic's Messages API port (read 2026-11-25), to take one at a time

The post: https://www.anthropic.com/research/migrating-api-to-rust. They moved a live Python
service to Rust. Writing the Rust took a week; trusting it took months. Their tests passed at
98% while only a quarter of real requests matched, and when the model ported the tests it
weakened more than a hundred assertions. What worked: the old system as the judge on real
traffic, a difference record that says where (the first step to differ) and which bug (a
fingerprint), three loops (seconds in a sandbox; real traffic; lessons kept as rules), and
people reviewing at the seam between the loops, because a judge that rewards matching outputs
was gamed with a shortcut that matched and was not worth shipping.

What it confirms here: the judge is the product; the test program is pinned to the original C,
graded by mutants and bound by digests, so a model cannot weaken it afterwards; a changed C file
makes a verdict stale.

What it changes, in the order to take them (each is a decision for the person first):

1. **(Done on a copy 2026-11-25; the person's steps are in item 5 of the work.)** **A piece's green is the inner loop only; turn the outer loop on now.** lz4 has three pieces
   accepted on a test program a model wrote, with the whole-program check not set up and no
   features. That is the post's "98% of tests": set up the whole-program check and the features
   on lz4 before more pieces, from what the project already holds (its `tests/` folder and its
   fuzz drivers under `ossfuzz/`, which the map already lists). This is also the rust-anything
   second read's first two ideas (coverage of the original; scenarios from the project's own
   tests): three sources now say the same thing.
2. **Say where, not at which byte.** A red comparison reports "first diff at byte N". The test
   program prints labelled lines, so the judge can name the first labelled line that differs
   (the call, the function) to the model and on the card. For the whole program, when the mixed
   build differs, put pieces back to C one at a time to find which piece.
3. **A memory loop for the models that write the code.** Lessons go to DECISIONS, which the
   session reads, not the model that writes a piece. A steer note lives and dies with one try. A
   per-project rules file the person approves, read into every test-program and Rust prompt
   (and one for the harness, shipped), would stop a second piece hitting the first piece's wall.
4. **Review at the seam, in words a person can use.** Green then Accept asks a non-engineer to
   trust matching outputs. Before Accept: a check on inputs the writing model never saw
   (generated, or from the fuzz drivers), and a second model at high effort reading the Rust
   against the C for shortcuts (special-casing the test inputs, a branch left as a stub), its
   finding on the card in one line.
5. **One fingerprint per failure** (check, failure class, function, first labelled line): the
   try list can say "the same failure as try 2", and "Do it All" stops for a new kind only, not
   for each occurrence. Small now; the base of the stop policy later.
6. **The headline number is the match rate on real runs**, not pieces done: "the program with
   its Rust pieces matched the original on N of M runs", beside "3 of 9".
7. **Later, and bigger:** the post's piece-by-piece attempt lost its gain to crossings between
   the two languages. Here every piece keeps a C seam; do not read speed off a half-moved
   program, and plan a last stage that removes the seams once a whole program is Rust, judged by
   the outer loop.

Not taken: shadowing live traffic, privacy-shaped records and the fleet of agents with a
coordinator. They answer a service with millions of requests a day and a team; here the program
is a file on one machine and the reader is one person.

## "Do it All" (the person, 2026-11-17): the milestone after the lz4 run

The person's shape, kept as stated: three steps. **Load the program** (Map, Configure, Accept
program, Scan, Plan). **The harness and the model propose for review**: the plan, a risk list per
piece, the decisions only the person can make asked in rounds with a recommended answer each,
the rough cost, and the model's forecast of what comes out. **Execute and return the agreed
product**: every pending piece in plan order (test program, Rust, Accept), the link into one
program, and a report: what changed, what ran, what stayed and why, and the harness's and the
model's determination of what the transitioned product looks like, with its honest limits.

Design points agreed in the conversation:
- It is the cockpit pressing its own Enter with one review stop, not a new machine.
- It must stop well: at the first refusal, red judge or stale plan it writes the report as it
  stands and hands back to the cockpit, the piece selected and the way out on the card. The
  report is written at every stop, not only at the end.
- The report renders the ledger (attempts, traces, plan, decisions) plus one paragraph of the
  model's judgment on top. A run record names the run: start, the decisions taken at the
  review, each piece's outcome, where it stopped and why, the report. That record is what a
  person shares, and the cockpit on the same target is the way back to hands-on.
- A budget and a checkpoint per wave; the cost said at the review step before "go".
- Prove before planning: build it once one target has gone through by hand without a wall.

Order of building: (1) "Run the plan": every pending piece in order with the chat's model,
stopping at the first wall or at the end of a wave, with the report; (2) the review page and the
rounds of questions; (3) the run record and the shareable report.

Added the same day, on what a stopped run may do by itself:
- **What it may touch.** The Rust (repairs, steers, new tries: the loop itself) and the
  migration's own records (plan, ledger, a piece set aside). Never the harness: a run that
  patches its own judge to pass a wall is the one failure we must not have; a harness wall is a
  hand-back with the log and a bug report, fixed in a development session with the gates, and
  the run resumes as a new run naming the harness build. Never the C program: it is the truth
  the judge compares against; the only knob is configuration (flags, defines, what counts as
  the program), decided at the review step.
- **Stops have kinds, and a policy per kind set once at the review**, not a question at each
  stop (unattended, nobody answers): red judge → steer up to N, then set aside and go on with
  the wave; refusal (the contract is wrong) → wait; stale plan → refresh, go on only if no piece
  changed; harness error → wait, bug report written; budget reached → wait; end of a wave →
  report, then go on or wait as chosen. "Have another agent debug and keep going" is a policy,
  and only ever the Rust kind of fix. Anything outside the table stops.
- **Accept.** A chat try is never promoted by itself today. In this mode the agreement is at
  the review, so accepting green pieces during the run may be automatic when the person said
  so there; the final product still comes back for review; every try stays in the ledger.

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
