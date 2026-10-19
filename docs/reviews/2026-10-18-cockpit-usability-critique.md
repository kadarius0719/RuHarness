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
