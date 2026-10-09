# Review of README.md and docs/TUTORIAL.md for a non-technical reader — 2026-10-09

The same lens as the testing guide's review (docs/reviews/2026-10-09-testing-guide-review.md): a careful person who has never programmed. Two Opus reviewers at high effort, one per document, ran every command they could on fresh copies. Their reports follow verbatim; the decisions are at the end.

---

# README review: read as a non-technical newcomer

Read: README.md (688 lines, commit 0d79abe), every line. Compared with docs/TESTING-GUIDE.md
(lines 1-120, Part 0 Steps 0.19-0.23, Part 12 Steps 12.1-12.6, Known quirks) and docs/TUTORIAL.md.
Run on copies under the scratchpad, with `RUHARNESS_ADOPTED` pointed at a scratch file and the
binaries built from this worktree: Quick start steps 2-5, the no-model hand-off, and "Start from
your own C project" steps 1-6 plus "when the project changes" on a copy of liblzg. Checked
`harness --help`, `harness project --help`, `harness project accept --help`, `harness migrate --help`,
`harness-tui --help`, `harness-mcp --help`, scores.json and the dependency list.

## Stops a non-technical reader

1. **The first screen says nothing a newcomer can use (lines 1-12).** In twelve lines it uses
   "harness", "codebases", "idiomatic", "incrementally", "verifiably", "LLM agents", "deterministic
   tooling", "differential oracle", "C ABI", "differentially tested", "mixed C/Rust link",
   "commit", "dynamic languages", "types and ownership", "planner". It never says that C and Rust
   are programming languages, who it is for, that it runs on a Mac (Linux only unconfined, line
   195; Windows never mentioned), what it does to your files, or what to do first. The first
   plain sentence is at line 75; the first "start here" is at line 72 (the tutorial); the testing
   guide first appears at line 207, as unlinked text inside the own-project section.
2. **Quick start fails on its first real command (lines 132-139).** Run as written on a fresh
   copy, `harness state status --target targets/zopfli` prints
   `error: …/targets/zopfli: this folder already holds migration results made elsewhere (11 units,
   1 verified): to trust them here, add --adopt once` and exits 1; step 3 fails the same way. The
   word `--adopt` appears nowhere in the README. The guide does it right in Step 0.21. With
   `--adopt` added to step 2, steps 2-5 work.
3. **Quick start assumes what a newcomer lacks (lines 124-130).** "You need Rust and a C compiler"
   with no way to get Rust; "From the repo root" with no way to get the repository (the guide's
   Step 0.8 has the `git clone` line); "5 minutes" when a first build alone takes several minutes
   (the guide says 30-60 minutes for Part 0). There is no "never done this? start at the testing
   guide's Part 0" line.
4. **Step 4 asks for an edit with no tool to make it (lines 142-146).** "Open … find … change the
   1 to 2": a newcomer does not know how to open a `.rs` file, or with what. The guide does this
   with `nano` in Part 5 (Steps 5.5-5.6), one key press at a time.
5. **"Start from your own C project" step 2 says "Write migration/map/config.toml" with no way to
   write it (lines 216-226).** The guide's Step 12.3 gives a paste-able `cat > … <<'EOF'` box and a
   `cat` to check it. Without one, the newcomer stops here.
6. **Step 6's block stops three times with "error:" and does not say so (lines 244-250).** Run as
   written: `gen-driver` ends `error: awaiting response: …response.json` (exit 1); `migrate` then
   refuses (`its generated driver's validation is missing`); `verify` refuses (`has no
   [unit.oracle] configured`). Pasting the block whole gives three errors in a row. Nothing says a
   hand-off is expected, how to answer it, or that `--model` should name who answers (left out,
   `gen-driver` records the answer as `claude-sonnet-5`'s, as its own message says). The guide
   spends Steps 12.7-12.13 on exactly this.
7. **The AI part points a newcomer down the hardest road (lines 157-177).** It offers Ollama (an
   install, a model download, a hand-made Modelfile, two terminals, `export`) or a bare hand-off.
   The guide's route for a newcomer is the cockpit chat on a Claude subscription, no API key; the
   README mentions that only at line 490, deep in the cockpit reference.

## Slows them

8. **Words used before they are explained** (line of first use; "never" = not explained at all):
   oracle 5 (explained 91); differential 5 (never); ledger 20 (102); provider 27 (never, in plain
   words); unit 46 (91); blind spot 59 (60); stderr 62 (never); undefined behaviour 63 (never);
   sandbox 118 (in brackets, good); API key 119 (never; the guide's line 5 explains it);
   compiler 124 (never); driver 145 as `differential-driver` and 247 as `gen-driver` (explained only
   at 377, in the command reference); flag 179 (never, though a table is headed by it);
   `harness.toml` 183 (never); attempt 188 (never); promote 189 (never); exit code 197 (never);
   tool, map, held choice 203-214 (explained in place, good); `main()` 213 (in brackets, good);
   linking 214 (never); `-I`/`-D`/`-O` 228 (named, not explained; the guide's "picture, in words"
   does it); commit 10 and `git diff` 241 (never); cockpit 422 (a heading, then "terminal UI");
   MCP 520 (never).
9. **"Expect eight `PASS` lines" (line 137; the tutorial repeats it).** Today's run prints 16
   `[PASS]` lines (8 checks and 8 feature scenarios) then `GREEN`. The guide lists this as Known
   quirk 1; fixing the README means deleting that quirk as well.
10. **"Read it with `git diff`" (line 241).** The tool's file is new, so `git diff` prints nothing
    (checked: 0 lines; `git status` shows `?? migration/`); in a project that is not a git
    repository it fails outright. The guide uses `cat`. The harness's own `accept` message says the
    same thing ("review it with `git diff`") and is worth fixing too.
11. **The hand-off instructions cannot be followed as written (lines 176-177).** The command names
    the `….response.json` it waits for, not a `….request.json`; the answer must be wrapped in the
    envelope `{"text": …, "input_tokens": 0, …}` that the harness prints. The README gives
    neither. It also does not say this leaves an in-progress attempt behind that
    `git checkout targets/zopfli` does not remove (an untracked `attempt.json`, checked).
12. **No step says what you should see or what it means**, except step 3's wrong count. Step 2
    (status) and step 5 (plan, observations) give no expected lines; step 5's output ends
    "next, write the first unit's differential driver: `harness gen-driver u-cache …`", which
    invites a newcomer into a hand-off the README never explains.
13. **Two ways of running the same program, unexplained.** Quick start uses the installed
    `harness`; the command reference (lines 268-401) uses `cargo run -p harness-cli -- …`; the
    cockpit example uses `cargo run -p harness-tui --`. A newcomer cannot tell they are the same.
14. **The flags table (lines 179-198) is reference material sitting inside Quick start.** Rows such
    as `--requester=chat … --answer-bytes=N` (line 192) and `harness override` (a command, not a
    flag, line 193) are for developers, yet a newcomer reading top to bottom meets them before
    their own project.
15. **Install steps differ from the guide.** README: `cargo install --path crates/harness-cli`
    (one program, no `--locked`). Guide Step 0.19: all three programs, with `--locked`. Anyone who
    followed the README has no cockpit and no chat connector; lines 480 and 568-571 mention
    reinstalling them only in passing ("perf's launcher", a term used nowhere else in the README).

## Polish

16. Lines 218-219 say "three lines are enough" over a four-line box; the guide says "four short
    lines, a heading line and three settings" (Step 12.3); `harness project --help` says "three
    lines". Pick one wording everywhere.
17. Lines 207-208 name the guide and the tutorial as plain text; make them links, with the Part
    and step numbers (Part 12; 12A is Steps 12.1-12.5).
18. The Status section (lines 34-68) leads with milestone codes M0-M6 and benchmark vocabulary
    (strict pass, non-UB vectors, scorable, training cutoff). It belongs below the newcomer path.
19. The Features section (lines 412-420) says "the person's features" (design-document wording)
    and "one of the three samples" (true of zopfli only). Point to guide Part 8 and the tutorial.
20. Repository layout (lines 636-666) omits `devtools/`, `providers.example.toml` and every doc but
    SCHEMAS.md; the dependency list (lines 680-683) omits `libc`; line 679 cites "§11 of the
    project briefing" without naming or linking docs/AGENT-BRIEFING.md.
21. Line 515: `--harness-mcp PATH (default: the one next to the cockpit)`; `harness-tui --help` adds
    "else on PATH".
22. The exit codes appear twice (lines 197-198 and 403-404). The guide's table (Step 0.22) adds
    `130` (stopped with Ctrl-C) and "1 also means waiting for an answer"; the README has neither.

## Untrue or stale against the program and its records

23. **Benchmark table, hidden row (lines 58, 61-66).** README: 15/17 (88.2%), 1 blind spot, 83/87,
    and "one (an FFI-boundary bug in the Rust) remains open". `targets/tractor/scores.json`
    (commit a55cb65, 2026-09-24): strict pass 16/17 (94.1%), 16 verified, blind spots 0, 86/87.
    DECISIONS.md 2026-09-24: "the blind spot is closed". The public row (70/77, 908/950) is right.
24. **No `--adopt` anywhere** (see 2). It also breaks every command-reference example on
    `targets/zopfli` and the tractor cases (lines 269-388; the case at line 375 refuses the same
    way, checked), changes the cockpit example (line 425: the cockpit asks its own adoption
    question first), and the `.mcp.json` examples (lines 573-597): harness-mcp refuses a target
    made elsewhere until the person adopts it (crates/harness-mcp/src/server.rs:1348).
25. "eight `PASS` lines" (line 137): 16 today (see 9).
26. "read it with `git diff`" (line 241): shows nothing for a newly written file (see 10).
27. "the matching ….response.json next to the ….request.json it names" (line 176): it names the
    response file (see 11).
28. The cockpit example (line 425) opens a benchmark case tree with the chat on by default, while
    line 599 says not to serve `targets/tractor/cases` to a chat agent. Use `targets/zopfli`, as
    the tutorial does.
29. The command reference has no entry for `perf` (guide Part 11), `project` (only in the
    own-project section), `promote`, `override` or `review`, all listed by `harness --help`.
30. Checked and correct: `harness project --help` prints the same six-step order as lines
    212-250; `--keep d1=d1.2` and `--run-name` exist as described; the `[SKIP] … not run` wording
    (lines 255-256) matches the oracle and guide Step 12.14; the "project changed" behaviour
    (lines 258-261) matches today's run (`accepted tool t-lzg: its own files changed since it was
    accepted …; scan it`, and `state status --tool t-lzg` starts with the same sentence); the
    `.mcp.json` argument shapes match harness-mcp's usage line; the planted bug of step 4 turns
    `differential-driver` to `[FAIL]`, the verdict `RED` (exit 10) and the unit `in-progress`, and
    `git checkout targets/zopfli` restores it (`Updated 4 paths from the index`).

## Where the README duplicates other documents, and who should own the text

- Quick start (lines 122-155) is copied almost word for word into TUTORIAL.md "Using the command
  line" (lines 153-187), stale in both places (no `--adopt`, eight PASS lines). The README owns
  it; the tutorial links to it.
- "How it works, in plain English" (lines 70-120) overlaps TUTORIAL "The big idea", "How a
  migration works", "How the judge decides". Keep ten lines in the README; the tutorial owns the
  long version.
- Cockpit and chat pane (lines 422-516): the tutorial owns keys, menus and the chat; guide Parts 4
  and 9 own the walk. The README keeps how to open it, its flags, and one link.
- "Start from your own C project" (lines 200-261): guide Part 12 owns the walk, tutorial "Mapping
  a whole C project" owns the words. The README keeps a short, correct six-step outline that hands
  over to Part 12, plus the config example.
- harness-mcp (lines 518-634): developer and safety reference; docs/MCP-DESIGN.md exists. Keep
  install, the two `.mcp.json` blocks, the deny list and the warning; move the protocol prose.
- Command reference prose (lines 263-410) repeats SCHEMAS.md and the design docs in places; fine as
  reference if marked so and kept to one paragraph per command.

## Order and length

Today's order serves a developer reviewing the design: principles, milestones, scores, then the
newcomer path at line 122, then about 400 lines of reference. A newcomer needs: what it is, is it
for me, what it touches, start here, a taste, their own project, then "where to look things up".
About 250 lines carry newcomer value. What could go or move: the milestone and score detail (to a
status section near the end, or targets/tractor/README.md); the flags table (into the command
reference); the cockpit's key-by-key prose (lines 442-465) and the chat pane (lines 483-516) to the
tutorial; the mcp protocol prose (lines 546-560, 599-613) to MCP-DESIGN.md; the Features section
to one line pointing at guide Part 8. The README could be 300-350 lines without losing a command.

## Proposed shape (every real command and output kept)

1. **RuHarness** (4-6 lines, everyone). In plain words: moves a program written in C to Rust one
   piece at a time, and proves each piece behaves the same by running both and comparing. Who it
   is for. Runs on a Mac (Linux only unconfined). What it touches: a `migration/` folder inside
   the project; your C files are never changed.
2. **Start here** (8-10 lines, newcomers). Never programmed? Follow docs/TESTING-GUIDE.md from Part
   0 (link; Part 0 takes 30-60 min, the whole guide 4-5 h). Want the ideas first? docs/TUTORIAL.md.
   Tools already set up? Quick start below. Your own C project? The section after it.
3. **How it works, in ten lines** (newcomers). The problem, the idea, harness / judge (oracle) /
   ledger, the flow diagram, why it is safe; words defined as they appear, in the guide's words.
4. **Quick start: the built-in example** (newcomers who have the tools). An experiment: what you
   need (Part 0 done, or Rust, Apple's tools and the clone line), then one command per step with
   You should see / What it means / If not: three `cargo install --locked` lines; `state status …
   --adopt` with its two adopt lines; `verify` (16 PASS and GREEN; `echo "exit=$?"` gives 0); plant
   the bug with `nano`, RED and exit 10; `git checkout targets/zopfli`; `plan`, then observations.md.
5. **Your own C project** (newcomers, then everyone). The six steps as today, with the `cat >` box
   for config.toml, `cat` instead of `git diff`, `--model` naming who answers, one line per step on
   what you see; hands over to guide Part 12 for answering the hand-offs.
6. **Using AI** (newcomers). The cockpit chat on a Claude subscription first (no API key; guide
   Parts 4 and 6); the command-line hand-off with its envelope; then live providers and Ollama,
   marked optional, with providers.example.toml.
7. **If something goes wrong** (everyone). Exit codes once (0; 1, including waiting for an answer;
   2; 10; 130) and a link to the guide's Troubleshooting.
   — then a rule and one line: "Reference: look things up here; a newcomer can stop reading." —
8. **Command reference** (developers). One short entry per command in `harness --help`, including
   `project`, `perf`, `features`, `promote`, `override`, `review`; the flags table; `--json`, the
   writer lock, Ctrl-C.
9. **The cockpit** (reference). How to open it (`harness-tui --target targets/zopfli`), its flags,
   the chat's safety rule in three lines, a link to the tutorial for keys and menus.
10. **harness-mcp** (developers). Install, the two `.mcp.json` blocks (with the adoption note), the
    deny list, the benchmark-tree warning, its flags; protocol prose linked to MCP-DESIGN.md.
11. **Project status and benchmark** (developers, reviewers). Milestones and the scores table
    brought up to date (hidden 16/17, 0 blind spots, 86/87), link to targets/tractor/README.md.
12. **Repository layout and working on the harness** (developers). As today, plus devtools/,
    providers.example.toml, libc and a link to the briefing.
13. **License** (everyone). As today.


---

# Review of docs/TUTORIAL.md, read as a non-technical newcomer

Read at commit 0d79abe as someone who can paste into Terminal but has never programmed. Checked
against a fresh build, `harness --help` and sub-helps, the cockpit driven headless on a scratch
copy of zopfli, its help screen and source strings, and the guide's first 120 lines and headings.

Verdict: a good reference for someone who already knows what a program, a compiler and a command
are; not yet a place for this reader to learn the ideas. It opens on words it never explains, its
first commands fail on a fresh copy, it repeats some ideas three times and lacks others, and about
a third of it is step lists and key tables that belong in the guide.

## Stops a non-technical reader

1. **The first command of every hands-on path fails on a fresh copy (lines 126–131, 162–171).**
   `harness state status --target targets/zopfli`, `harness verify …` and `harness plan …` all
   end in `error: … this folder already holds migration results made elsewhere (11 units,
   1 verified): to trust them here, add --adopt once`. The cockpit (line 129) first prints a
   full-screen question, "Adopt this folder? Type y and Enter to adopt", about "build folders"
   and "a fresh random token". The tutorial never mentions trust or "made elsewhere" (the
   guide's Step 0.21 does), and afterwards the cockpit shows `u001-katajainen` as `⚠`, not `✓`.
2. **The opening never says what C and Rust are, or why anyone moves one to the other
   (lines 1–9).** "A program written in C to Rust" assumes both words. The guide's first line
   does better ("a newer language that rules out a whole family of memory mistakes").
   Without the why, "proves it behaves exactly like the C" has no stakes.
3. **Program, function, file, compile, build and link are never explained, yet everything
   rests on them.** Line 9 ("the same function names … drops straight into the program"),
   line 46 ("each function, and which file uses which"), line 61 ("exports"), line 64
   ("compile"), line 66 ("built twice"), line 370 ("the compiler refuses … the parser"),
   lines 430–449 ("`main()`", "link", "defined exactly once"). A reader who does not know a
   program is made of many files of functions, each file turned into machine code and then
   joined, cannot picture a unit, a swap, or the map.
4. **Line 3 and line 7 say the same sentence; neither says what the reader will understand by
   the end, nor when to read this versus the guide.** The guide is named only once, at line
   425, for Part 12. A newcomer does not learn that the guide is the hands-on walk and the
   tutorial the explanations, or in which order to use them.
5. **The cockpit opens on a paste of `cargo …` commands with no idea of what building is
   (lines 118–133).** "This needs Rust and a C compiler", "`xcode-select --install`",
   "`cargo build --workspace`", "`cargo run -p harness-tui -- --target …`": four unexplained
   things in five lines, and a different recipe from the guide's Part 0 (which installs three
   programs with `cargo install`). Two set-up recipes in two documents stop a careful reader.
6. **The walk in "Your first migration" cannot be done (line 499).** It needs "a practice copy
   of the zopfli project in which the unit `u001-katajainen` has been reset to planned". No
   such copy exists and nothing says how to make one. The guide's Part 4 is the real walk.
7. **The hand-off is described as if anyone could answer it (lines 81, 100, 549).** "Anyone
   can write the matching `…response.json` next to it". A newcomer following that writes plain
   text and is refused: the answer must sit in the envelope (`{"text": …, "input_tokens": 0,
   "output_tokens": 0, "stop_reason": "end_turn"}`), and the command must name who answers
   with `--model`. The guide's Parts 3, 6, 12 and Plan B lean on this and teach it from scratch.
8. **The two diagrams are Mermaid code (lines 17–25, 33–42).** On GitHub they draw; in Terminal,
   a text editor or `cat` they are lines like `pick["Pick a unit<br/>you, or ask the chat"] -->
   write`, which look broken to a non-programmer. A picture in plain characters (as the cockpit
   sketch at 208–221 does) works everywhere.

## Slows them

9. **Words used before they are explained.** attempt (21; explained 95), verdict (39, 51; 107),
   hazard (53; 109), driver (63; 70), repair turn (23, 50; 96), sandbox (86; 111); API key
   (81, 83; never — the guide does, line 5); Claude Code (80; never said to be a separate app).
10. **Words never explained at all.** C, Rust, function, file/folder path, compile, build, link,
    macro and "pointers to functions" (48), exports (61), byte (65), sanitizer and "memory-
    checking tools" (67), compiler optimisation (70), fingerprint (72, 107), terminal command
    and option/flag (`--target`, 185; "a flag or a word, never a path", 353), exit code (204),
    commit and `git checkout` (175, 374, 414, 417), crate (412), editor/nano (356), scratch
    copy (364), exit status and "printed" (361), perf (384: introduced as if it were a person,
    "perf runs each workload"), CPU, instructions, cores (391–402), MiB (386), `make`, `cmake`,
    `-I`, `-D`, compiler flags (440–442), JSON and TOML (455, 478), `git diff` (469),
    AGENTS.md (199), MCP and `.mcp.json` (535), environment variable (`export …`, 540),
    undefined behaviour (562), held-back test (555), "the edge between the Rust and the C"
    (562), Replace (411: a menu item the tutorial never introduces), kept edit (help shows `E`).
11. **Explanations that lean on another unexplained word.** Line 14 "checks the Rust does
    nothing it should not" (what could it do?); line 61 "replace the system's printing function
    to fake its results" (needs "exports" and "linking"); line 62 "no files, network, other
    programs or clocks" (good, but under the name "Allowed calls only" with no "calls");
    line 67 "reads or writes past the end of its memory" (needs memory); line 70 "the same with
    and without compiler optimisation"; line 86 "the thin layer that connects it to the C"
    (the guide calls it the FFI wrapper and C ABI); line 111 "a locked-down space".
12. **The glossary sits in the middle (88–114),** after most of its words were met, and defines
    Benchmark, Workload and "As it stands" long before their sections. Define each word where
    it first appears; keep the table as a recap at the end.
13. **Ideas the guide leans on that the tutorial lacks.** Trust and "made elsewhere" (guide
    0.21); the envelope and naming who answers (Parts 3, 12); the question's key; candidate,
    stale; the driver's seven checks (3.10; line 70 names none); tool versus target and
    `--tool` (457 says "tool" without "an accepted program, which then plays the target's
    part"); the three reasons a model may give for a held choice (`platform`,
    `alternative-implementation`, `cannot-tell`); feature map versus project map (never said
    to differ); exit code 130 and "1 also means waiting for an answer" (0.22).
14. **Ideas explained two or three times.** The six steps (33–51, 187–200, 258–274); "nothing
    is accepted without you" (7, 29, 101, 267, 272, 529, 570); the chat's permission to
    continue (326, 507, 571); the sandbox (86, 111, 573, 578, 601); keys (235–248, 330–340).
15. **The order jumps.** The command line (153) comes before the cockpit (206) though line 139
    says "start with the cockpit"; features, speed and the map (344–495) come before the first
    migration (497) and "Reading the results" (514); safety (566) comes after the benchmark.
    The guide's order is: set up, trust, target, scan and plan, driver, hand-off, migrate,
    judge, accept, features, status, speed, map.
16. **Walks that belong in the guide.** 116–135 (= Part 0), 153–183 (= 0.21–0.22, Part 5),
    497–512 (= Part 4), 537–547, the Features and Speed "Write/Measure" bullets (355–358,
    389–397 = Parts 8, 11), 462–470 and 483–495 (= 12C). Keep each idea, move the keys.
17. **Reference a learner does not need on first read:** the command table (187–202), the menu
    table (258–274), both key tables, the chat's state words (307) and the 19-row trouble table
    (584–602, overlapping the guide's). Useful, but as an appendix, not between ideas.

## Polish

18. Line 14 "hundreds of the same inputs": depends on the driver; "many" is safer.
19. Line 248 shortcuts `a m e r R v d`; the help screen has `a m e E r R x d v` (kept edit,
    Cancel). The symbol table (278–289) lacks the help screen's `·`, `–` and `⊖`.
20. Lines 120 and 223 "156 characters": say "columns, the window's width counted in letters".
21. Lines 346–348 and 378 open with the document's best plain sentences ("what a person notices
    is the whole program"); every section should open that way.
22. Line 529 "A GREEN verdict is only as good as the tests the oracle ran" is the key caution,
    buried in a table section; it belongs with the judge (and the guide's Step 6.15).
23. The tutorial says "oracle"; the guide teaches "judge (the files call it the oracle)". Agree.

## Untrue or stale

24. **Lines 129, 165, 170, 180 (finding 1) and 499** "in the project itself it is already
    migrated": on a fresh copy each refuses or asks first, then shows `⚠` "made elsewhere".
25. **Line 167 "Expect eight PASS lines":** today 16 (8 checks plus 8 feature scenarios),
    as the guide's Known quirks (6785) and Step 0.22 already say. Line 508 "8 of 8 checks"
    is also zopfli-without-features.
26. **The check table (59–68) does not match the screen.** "The Rust builds" appears only when
    the build fails (and is then the only check); "Whole program" is one line per sample file
    (three for zopfli); the `feature:` checks are missing from the table though they now run
    on every verify (and are mentioned only at line 359). "In order" is not the order the
    screen prints. Better: name the checks as the screen prints them (`symbol-set` …) with the
    cockpit's words beside them, as the guide's Part 5 does.
27. **Line 133 and line 503: "says which before your first message" / "The first time, it says
    starting Claude Code".** The cockpit starts Claude Code, and prints that line, only when
    the first message is sent (chat/mod.rs `send` → `start`); the guide's Step 4.5 has it right.
28. **The command table (187–200) misses four commands `harness --help` lists:** `features`,
    `project`, `perf` and `review`; it also omits `--adopt` and `--tool`, both needed in the
    guide. "Every command … takes `--target <folder>`" — and defaults to the current folder,
    which Part 12 relies on ("No `--target` in this part").
29. **Line 204 exit codes:** the guide adds `130` (stopped with Ctrl-C) and that `1` also means
    "waiting for an answer" at a hand-off.
30. **Line 549 "Anyone can write the matching …response.json":** a bare answer is refused
    (`the response file must hold the envelope …`); see finding 7.
31. **Cross-document disagreement on `◐`:** the tutorial (284) and the help screen say "tried:
    attempts exist, none accepted"; the guide's Part 4 word table says it marks "an attempt that
    has not finished (it stopped at a hand-off)". One of the two needs to change; the help
    screen agrees with the tutorial.
32. **Line 535: harness-mcp "can … accept a GREEN attempt"** is right (`harness_promote`), but
    "never starts a fresh translation" is true only of the stand-alone server; inside the cockpit
    the chat does ask for a fresh Migrate (313). Say which is which.

Everything else checked held: every quoted menu label, dialog phrase, chat message and Speed
answer exists in the cockpit; the file and line at 172 exist; the benchmark numbers match README.

## Pictures in words that would help

- **The swap** (big idea): `[main.c] [checksum.c → its Rust] [decode.c]` — the rest stays C,
  the function names stay the same, so the rest cannot tell.
- **The judge** (two lanes, one driver): `driver + C → output A`, `driver + Rust → output B`,
  `A = B byte for byte? → PASS`; then the whole program the same way, then the features.
- **The hand-off** (missing): harness writes a question file and stops → you or the chat write
  the answer in its envelope → the same command again reads it and goes on.
- **The ledger** as a short folder tree (`plan.toml`, `units/<unit>/`, `features/`, `perf/`,
  `map/`, `tools/`), so "commit `migration/features/`" means something.
- **The map** (420–495 has none): download → three `main()` files → the files each needs → a
  fork where two files offer one function (held choice) → configuration → accept → a tool
  folder. The guide's "The picture, in words" (5451–5470) is this; share it.
- **Feature map versus project map**, side by side: "which functions each use of the program
  runs" versus "which files make up each program".

## Length

604 lines; about 230 are walks, key tables and troubleshooting that belong in the guide or an
appendix; about 120 are missing (C/Rust/building, trust, the hand-off and envelope, tool versus
target, the two maps, the three reasons, how to read it with the guide). About 450 is reachable.

## Proposed shape (one page)

Opening (10 lines): what you will understand by the end (the ideas below, in the order the guide
meets them); read each section before the guide part named beside it; the guide is where you
type and see, this page never asks you to run anything.

1. **Programs, files and building** — a program is many files of functions; the compiler turns
   each into machine code; linking joins them; C and Rust, and why move (memory mistakes).
   Prepares guide Part 0 and Part 1. Keeps lines 3, 9.
2. **The big idea: one unit at a time** — a unit, the swap picture, nothing changes until you
   accept. Prepares Parts 1–2. Keeps 7, 9, 27.
3. **The harness, the judge and the ledger** — three parts, with the ledger's folder tree; the
   trust question and "made elsewhere" (why a downloaded ledger is asked about once).
   Prepares Part 0 Step 0.21. Keeps 13–15, 72.
4. **From C to a plan** — scan, plan, hazards (and what a macro or shared data is), in words;
   the six steps once, as one list. Prepares Part 2. Keeps 44–51.
5. **The driver** — a test program for one unit; why it is itself tested first (repeatable,
   speed-ups on and off, planted bugs). Prepares Part 3. Keeps 70.
6. **The hand-off** — question file, stop, answer in its envelope, who answers (`--model`),
   run again; providers (chat, hand-off, local, cloud, replay) and what an API key is.
   Prepares Parts 3, 4, 6, 12 and Plan B. Keeps 76–86, 549–551.
7. **The judge and its checks** — the two-lane picture; each check as the screen names it, with
   plain words; GREEN, RED, verdict fingerprint; "only as good as the tests". Prepares Part 5.
   Keeps 57–68 (corrected), 72, 529.
8. **A migration, turns and your decision** — attempt, turn, repair turns, accept/promote,
   modify with a note, retry, hand edit, what RED changes (nothing). Prepares Parts 4 and 6.
   Keeps 516–529 (as ideas, without the table of keys).
9. **The cockpit and the chat** — the screen sketch (keep 208–221), why every action waits in
   a dialog, what the chat can and cannot do, its permission to continue. Prepares Parts 4, 9.
   Keeps 223–229, 293–299 (shortened), 303–326 (as ideas), 566–578.
10. **Features** — what a person notices; scenarios; the feature map, named as distinct from
    the project map. Prepares Part 8. Keeps 346–374 without the editor keys.
11. **Status and picking up later** — fresh, stale, out of date, commit what you did.
    Prepares Part 10.
12. **Speed** — workloads, the C and the Rust taking turns, how to read "can't tell".
    Prepares Part 11. Keeps 378–418 trimmed of the menu steps.
13. **Mapping a whole C project** — the map picture; programs, shared files, held choices and the
    three reasons a model may give; configuration; why "it links" is not proof; tool versus
    target and `--tool`. Prepares Part 12. Keeps 428–450, 457–481.
14. **How we know it works** — the benchmark. Keeps 553–564.
15. **Words** — the glossary as a recap, every word with the section that explains it.
16. **Appendix: reference** — the commands (completed: features, project, perf, review,
    `--adopt`, `--tool`, `--target` default), exit codes (with 130), the cockpit's keys and
    symbols (matching the help screen), the menu items, and a short "when something looks
    wrong" that points to the guide's Troubleshooting for the rest.

Moves to the guide (or is dropped as already there): "Before you start" (116–135), the
command-line tour (153–183), "Your first migration, step by step" (497–512), the local-model
commands (537–547), the map's cockpit steps (483–495) and "Accepting again" steps (462–470).


---

# Decisions (the session, 2026-10-09)

Both documents are rewritten to the shapes the reviewers proposed, with these rules:

1. **Who owns what.** The README is the front door and owns Quick start and the short own-project
   outline; the tutorial owns the ideas, explained once each in the order the guide meets them,
   and never asks the reader to run anything; the guide owns every walk. Where two documents said
   the same thing, the owner keeps it and the others link to it with the part or step number.
2. **The README's first screen** says in plain words what this is (C and Rust are programming
   languages; the harness moves a program from one to the other a piece at a time and proves each
   piece behaves the same), who it is for, that it runs on a Mac (Linux unconfined only), what it
   touches (a `migration/` folder; your C files are never changed), and what to do first (never
   programmed: the guide's Part 0; the ideas first: the tutorial; tools set up: Quick start; your
   own project: the section after it). The README follows the reviewer's 13-section shape; a rule
   and one line mark where the reference begins.
3. **Quick start and the own-project outline are experiments**: what you need, one command per
   step, You should see, What it means, If not; `--adopt` on the first command with one sentence
   on why; the three `cargo install --locked` lines as the guide installs; `nano` for the edit;
   the `cat >` box for `config.toml` and `cat` to read the written file; `--model` naming who
   answers; every expected `error:` announced; a hand-over to the guide's Part 12 for the
   hand-offs. "Eight PASS lines" becomes today's screen; the guide's Known quirk 1 goes with it.
4. **Using AI**: the cockpit chat on a Claude subscription first (no API key), then the
   command-line hand-off with its envelope, then live providers and a local model marked optional.
5. **Untrue or stale lines fixed from the program and its records**: the benchmark table's hidden
   row (16/17, 0 blind spots, 86/87, the blind spot closed, from targets/tractor/scores.json and
   DECISIONS 2026-09-24); the hand-off names the response file and the envelope; `git diff` on a
   new file becomes `cat`; the cockpit example opens `targets/zopfli`; the command reference lists
   `perf`, `project`, `promote`, `override`, `review`, `--adopt`, `--tool`; exit codes once, with
   130 and "1 also means waiting for an answer"; the `.mcp.json` examples note adoption.
6. **The tutorial** follows the reviewer's 16-section shape: it opens with what you will
   understand by the end and when to read it versus the guide, each section names the guide part
   it prepares, and it explains what was missing (programs, files, compiling, building and
   linking; C and Rust and why move; the trust question; the hand-off with its envelope and who
   answers; the question's key; candidate and stale; the driver's seven checks; tool versus
   target and `--tool`; the three reasons a model may give; the feature map versus the project
   map). Pictures are in plain characters, never Mermaid. The check table names the checks as the
   screen prints them with the cockpit's words beside them, feature checks included. The glossary
   becomes a recap at the end; the command, key and menu tables an appendix matching the help
   screen. The walks move to the guide or are dropped where the guide already has them.
7. **Words agreed across all three documents**: "judge (the files call it the oracle)"; `◐` means
   "tried: attempts exist, none accepted" (the help screen's meaning; the guide's Part 4 word
   table is corrected); "the feature map" and "the project map"; "a tool is an accepted program,
   which then plays the target's part".
8. **The `accept` command's closing line** says "review it with `git diff`", which shows nothing
   for a new file: a one-line code change to "read it with `cat`, or `git diff` once it is
   committed", for the next wording pass (not the docs writers').

## The rewrite

Two writers in parallel (README; tutorial), each keeping every real command and output and
running what they quote; the guide's two small corrections (Known quirk 1, the `◐` row) go with
the tutorial's writer. Then one fresh-eyes read over README → tutorial → the guide's first 120
lines as a newcomer would meet them, a fix pass, and the push.
