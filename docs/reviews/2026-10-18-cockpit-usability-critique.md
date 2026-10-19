# The cockpit, critiqued for someone who has never used it — 2026-10-18

The person asked for a critical usability review of the cockpit (`harness-tui`): we want anyone
to be able to use it. The session (Fable) drove it headless on a copy of targets/zopfli at 150×44
and 80×30, and on an untouched liblzg copy (the project mode), and read every screen: the first
screen, the help, the file tree, the Enter menu on a file and on a unit, a unit's View, the
activity details, the chat pane, the end of the tree (Features, Speed), the project mode's menu and
its Map dialog. What follows is what a newcomer meets, in the order they meet it, then the work.

## What is good and must stay

- Every action sits behind an Enter menu and a dialog that says what it runs, how long, what it
  writes, and runs only on an explicit yes. Disabled items say why ("Re-check with the oracle — the
  unit has no crate yet"). The "── Uses a model — can take minutes ──" separator is honest.
- The bottom key bar changes with the context and is short.
- The 80-column layout works; the summary wraps instead of being cut.
- The chat pane says what it may and may not do before the first message.
- A unit's View shows the C and the Rust side by side once both exist, with the exported function
  named and its line.

## What stops or slows a newcomer

1. **No screen answers "what should I do now?"** The first screen is counts ("26 files scanned",
   "Units (11): 1 ⚠ needs attention · 10 ◇ planned", "Features: 7 — 5 hold so far, 2 all C · they
   ran 108 of 111 watched functions (each unit checked with only its own Rust swapped in)") and a
   "Needs attention" list whose one item says "needs attention" twice and never says why or what to
   do. The status line says "Ready." A mapped tool's screen does better ("Next step: Nothing is
   scanned yet — press Enter and choose Scan the project"); every screen should.
2. **The words are the program's, not a person's.** "oracle" where every document now says judge;
   "No crate yet" (crate is Rust jargon; say "no Rust yet"); "Find hazards (run the detectors)";
   "planned · status pending" (two words for one state); "watched functions", "hold so far", "all
   C"; "Speed (no file)"; "status pending"; the root's "⚠1 ✓0/13" (13 what? units are 11); the
   chat's first sentence reads "The chat reads the project through the harness and it runs
   anything; you confirm every act", which says the opposite of what it means, and then prints the
   Claude binary's full path.
3. **Features and Speed hide below the fold.** The tree lists every `.c` and `.h` (26 rows), then
   Units (11), then Features (7), then Speed. On a 44-row terminal Features and Speed are off
   screen; on 30 rows even Units is. A newcomer never finds them. Units also appear twice (a mark
   on each `.c` row, then the Units list).
4. **The Enter menu's first item is "Open", which opens what is already open.** The row's file is
   already in the View. The action a newcomer wants on a planned unit (Migrate) is fourth, after
   "Re-read the project" and a disabled "Re-check". The recommended next action should be first and
   highlighted, with one line saying why.
5. **A unit's View leads with raw C.** Thirty lines of a C function with a `~`-filled "no crate"
   column beside it is noise for someone who cannot read C. The View should lead with what the unit
   is in words (its functions, their size, which features run it), its state and its next step, and
   fold the code below.
6. **The help is a manual.** A long mouse paragraph, then a key table with `a m e E r R x d v` and
   `j k ]f [f J K`, then a states legend with four kinds of tick. A newcomer needs six keys. The
   rest belongs on a second page.
7. **The project mode is a different program.** On an untouched project the cockpit becomes a
   plain-text numbered menu ("Type a number (1-1) and Enter; anything else leaves:") headed by the
   full absolute path, with dialog text cut at the terminal's edge; after the map it drops the
   person back to that menu. Everything else is a full-screen cockpit with the same dialogs. The
   two should look and behave the same.
8. **Empty states say nothing.** The activity details box before any command: "no command yet" and
   nineteen blank lines. The feature marks ◉/◌ have no words on the selected row.

## The work, in three passes (Opus, sequential; they share view.rs, menu.rs, app.rs)

**Pass 1 — words and next steps** (lowest risk, most value): one vocabulary across every screen
string, matching the documents (judge; "no Rust yet"; "find risky patterns"; one state word per
row; "Speed — not set up yet"; the root counts labelled; the chat's first sentence corrected and
the binary path dropped); a **Next step** line on every View (the project summary, a unit, a
feature, the speed view, the needs-attention list: cause + action per item); the Enter menu's
first item is the recommended next action, highlighted, with its one-line reason, and "Open" goes
when the row is already open; the help split into a short first page (six keys, three marks) and
"more" for the rest; empty states say what will appear and how; "Ready." becomes "Ready — ↑↓ to
choose a row, Enter for what you can do" until the first action.

**Pass 2 — layout**: Units, Features and Speed above the file tree (or the file tree folded by
default with the units' marks summarised on folders), so all three are on screen at 30 rows; a
unit's View leads with the plain summary and folds the code below (a key to open it); the
side-by-side C/Rust only when both exist.

**Pass 3 — the project mode as a cockpit screen**: the same full-screen look and dialogs as the
rest (the folder's name, not the full path; wrapped text; the map's result shown as a screen with
the programs as rows and Accept in their Enter menu), so a person never meets the numbered text
menu.

Each pass: every changed string pinned by a test, goldens re-rendered and read, the cockpit driven
headless to confirm, the tutorial's and topics' quoted labels updated where they change, and a
reader who has never used it walking the first ten minutes after pass 3.


---

# A newcomer's ten minutes after the three passes (2026-10-18)

A reader who had read only docs/topics/cockpit.md opened the cockpit on an untouched liblzg copy and on zopfli at 120×36 and tried to get somewhere. They reached every goal in under eight minutes. Their findings are fixed in pass 4.

# The cockpit in ten minutes, as a newcomer

Read only docs/topics/cockpit.md, then drove `harness-tui` headless at 120 columns × 36 rows on a
fresh copy of liblzg and a copy of targets/zopfli (head 04b842b, the three cockpit passes). The
adoption file pointed at a scratch path. Nothing was run that needs a model.

Time: liblzg 19:59:18 to 20:03:31 (4 min 13 s, of which about 1 min 30 s went on a setup mistake of
mine, see the note under the log); zopfli 20:04:09 to 20:07:05 (3 min). About 7 min 45 s in all.
Every goal was reached inside ten minutes.

## The log

Screen → what I did → what happened → stuck or not.

1. liblzg, first screen (Project view, "not mapped yet") → read it → one row "Map the project",
   Next step "press Enter and choose Map the project", a plain paragraph on what mapping does.
   Not stuck. Knew what to do without the page.
2. Enter → a menu titled "Map the project" holding one item "Map the project" → Enter. Not stuck,
   but three copies of the same words and two Enters for one choice.
3. The dialog "Map the project?" → waited for `ready`, pressed → then Enter. Clear: what it does,
   how long, what it writes. Not stuck. The command is printed twice ("It runs:" and "Command:"),
   and with a long path the two take half the box.
4. First map result → "3 programs found": t-benchmark and t-unlzg "held on a choice", t-lzg
   "linked". Next step: "The configuration is a guess — state the build in
   migration/map/config.toml and choose Map the project again, or press Enter and choose Ask for a
   proposal". Slowed: I wanted the compressor; nothing says which program compresses. Guessed
   t-lzg from the file name lzg.c.
5. t-lzg, Enter → Accept is greyed; the first item is "Ask for a proposal (uses a model)".
   Choosing Accept shows a clear "Why not": write config.toml, or ask a model, then map again.
   **Stuck**: the cockpit has no way to write that file and does not say what goes in it.
6. Pressed `c` → the details of the map run; the example config.toml is at the very end of a long
   log. Found it after several scrolls. Wrote the file by hand outside the cockpit (with
   `-Isrc/include`, a guess from the example's comment).
7. `g` (re-read) → the View still says "no migration/map/config.toml states the build yet",
   though the file now exists (true only after a re-map, but it reads as wrong).
8. Enter → "Map the project again" → → Enter → done. Next step now: "t-benchmark holds a choice".
   The Accept item on t-lzg is live, "recommended: t-lzg links and is ready". Not stuck.
9. Accept t-lzg → dialog → pressed `y` this time (the dialog offers it) → "accepted", a new group
   "Accepted tools (1)" with the row "Open tool t-lzg". The Next step still points at
   t-benchmark, not at the tool I just accepted. Slowed a little.
10. Down once from t-lzg → landed on t-unlzg (expected: the Open row; each program is two lines,
    only the first selectable). Second Down → "Open tool t-lzg". The View says "Enter on its row
    opens it"; Enter shows a menu with "Open tool t-lzg", a second Enter opens. Not stuck.
11. Tool t-lzg → "◂ Back to the project view" first, Next step "Nothing is scanned yet — press
    Enter and choose Scan the project". Found the next thing to do. Marks `⊖`, `+`, `·` and `+4`
    were new to me; "Scan the project" repeats under each of four files. q → back to the shell.
12. zopfli → before the cockpit, a plain-text question "Adopt this folder?" full of words I did not
    know (ledger, verdict, drivers, token). Typed y and Enter.
13. zopfli first screen → title "zopfli · 1 needs attention · 0 of 13 C files in Rust", Next step
    "u001-katajainen needs attention: verdict made elsewhere — Re-check with the judge — open it".
    Found the one thing in seconds. Not stuck.
14. Down twice to u001-katajainen → the View in words: one function, its file, its checks as
    "✓ same outputs as C" and so on; "▸ C source (→ to open)". Pressed → → it opened the unit's
    row in the tree (crate, three attempts) instead of the C source. Tab then → opened the code
    beside its Rust. Took two tries.
15. Tab once more → Chat ("not started"), a plain sentence on what it does. The activity line
    still says "Ready — ↑↓ to choose a row, Enter for what you can do", which is not what the keys
    do in the chat. Tab → back to Files.
16. Enter on the unit → "Re-check with the judge" first, with the reason. What to do is clear.
    Esc (did not run it).
17. Down to "Features (7)" → a readable list: "holds so far · 1 of 10 units", "still all C", and
    three functions no feature ran. Next step "Nothing to do — choose a feature". Not stuck.
18. Down to "Speed — not set up" → a plain paragraph, Next step "Write your workloads file";
    Enter shows "Write your workloads file (in nano)". Not stuck.
19. `?` → a short first page (six keys, three marks), ↓ shows every key and every mark. All the
    marks I had wondered about are there. Any other key closed it. q → back to the shell.

Note on my setup (not a cockpit fault): I first pointed the adoption file at an empty file. The
first map said "done", but its details held a parse error ("could not record … as made on this
computer"); the second map then refused with "add `--adopt` once", which the cockpit cannot do. I
restarted on a fresh copy with a path to no file, and all went well. Also, after the adoption
question the headless driver showed leftover text over the cockpit; that is the driver (it does
not clear on the switch to the full-screen view), not the cockpit, and a restart cleared it.

## Findings

### Stopped me

1. Project view, a fresh project: Accept stays greyed until migration/map/config.toml exists, and
   the cockpit can neither write it nor show what goes in it except at the bottom of the `c` log.
   Fix: offer "Write the build configuration (in nano)" in the menu, opened on the guessed
   template, the way Speed offers its workloads file.
2. Project view, a run that half-failed: the first map said "done" while its log said it could not
   record the folder as made here, and the next map then refused with "add `--adopt` once", which
   no cockpit item does. Fix: report "done, with a problem — c for its words" when a run prints an
   error, and offer an Adopt item (or the adoption question) when a command refuses for that reason.

### Slowed me

3. Unit View: "▸ C source (→ to open)" while Files has the focus; → opened the unit's attempts in
   the tree instead. Fix: make → on an already-open unit go into the View and open its code (as the
   help says), or write "(Tab, then → to open)".
4. Project view: nothing says which program is the compressor; rows are only "t-lzg (tool)",
   "t-unlzg (tool)". Fix: show a one-line description per program when the map can find one (its
   usage text or first comment), or say "tool: a program you run" so the guess is easier.
5. Project view, after accepting t-lzg: the Next step stays on t-benchmark's choice instead of
   "Open tool t-lzg to scan it". Fix: right after an Accept, make the Next step the new tool.
6. Project view: "held on a choice (d1)", "Choice d1 … d1.1 … d1.2", "linked", "configuration is a
   guess" are new words with no plain gloss on screen. Fix: one sentence under the program list:
   "held on a choice: two files define the same function; Accept asks which to keep".
7. zopfli unit View: "needs attention: verdict made elsewhere" sits next to "verdict green, fresh",
   "Crate origin not recorded" and "0 of 13 C files in Rust" while a GREEN attempt exists; I could
   not tell whether this unit is done. Fix: one line in words: "It has Rust that passed on another
   computer; re-check it here, then Accept to count it."
8. The adoption question before the cockpit: ledger, verdict, drivers, token. Fix: lead with one
   plain sentence ("This folder holds migration results from another computer; trust them here?")
   and put the detail below it.

### Polish

9. Project view: a one-item menu repeats the row's words three times and needs two Enters. Fix:
   when the menu holds only the Next step's item, open its dialog straight away.
10. Dialogs: the command is shown twice ("It runs:" and "Command:"), each with the full path. Fix:
    keep one, the full command, at the end.
11. Project view after `g`: "no migration/map/config.toml states the build yet" stays while the
    file exists. Fix: say "config.toml found — choose Map the project again to use it".
12. Activity line: says "Ready — ↑↓ to choose a row, Enter for what you can do" while a dialog or
    the chat has the keys. Fix: say only "Ready" when the focus is not on Files.
13. Files: long unit names lose their "planned" word (u-gzip_container, u-zlib_container,
    u-blocksplitter…), and the selected u001 shows "u001-kataj… needs att…". Fix: shorten the name,
    never the state word.
14. Project view: "Enter on its row opens it" under Accepted tools, but Enter shows a menu first.
    Fix: open the tool on Enter, or say "Enter, then Open tool".
15. Tool screen: "Scan the project" printed under each of the four new files. Fix: say it once
    under the list.
16. Casing: "1 GREEN, 2 red" in the unit View. Fix: one casing for both.
17. Tree rows "crate", "truncated", "green" under a unit and the word "perf" on the Speed View are
    unexplained on screen (the help covers the marks, not these words). Fix: a short gloss in the
    View when such a row is selected; "perf" → "the harness".

## Where the page and the screen disagree

- Confirming: the page says `→` then `Enter`; every dialog also says "or y · or click" and `y`
  works. The page should mention `y` (or the dialog should not offer it).
- "Items that ask a model sit below the line `── Uses a model — can take minutes ──`": before a
  config exists, t-lzg's menu shows "Ask for a proposal" first, with "uses a model" only in its
  recommended line and no separator line.
- Marks: the page lists `✓ ◇ ◐ ✗ ⚠ !`; the screens also show `⊖`, `+`, `·` (tool screen), `◉`,
  `◌` (features), `✓?` and `⊘` (help). The page could say "`?` lists them all" first, or list the
  feature marks.
- "There is no Open item": the Project view's tool row has an "Open tool t-lzg" item (right, since
  it changes screen, but the page says otherwise).
- The page says the first row of Files is the project with a summary like `✓2/5`; on a newly
  accepted tool it is "liblzg · tool t-lzg +4" and there is no Units group yet (only Features and
  Speed, and the source folder open). Not wrong, but the page could say so.
- The page's description of the Project view says "Map, Ask and Accept in each row's Enter menu";
  the screen names them "Ask for a proposal" and "Ask a model for advice" (two different Asks).
- The page does not say that a copied project asks "Adopt this folder?" before the cockpit opens.
- The page says `→` opens a unit's folded code ("C source (→ to open)" on screen); in Files, `→`
  opens the unit's row instead (see finding 3).

## What was good

- A Next step on every screen, in a sentence, and the menu opening on it with a "recommended"
  reason. I never had to guess the next move except where the project needed a file I had to write.
- Greyed items say why when chosen, in words, with what to do.
- Dialogs say what they do, how long, and what they write, before the command.
- zopfli's one problem was in the title, the Next step and the first unit row: found in seconds.
- The unit View reads like a summary (functions, files, features, attempts, checks as "✓ same
  outputs as C"), with the code folded out of the way.
- Features and Speed Views are plain and say what to do next.
- The help is two-layered as the page says: six keys and three marks first, everything on `↓`.
- `q` quits cleanly at once when the chat was never used.
