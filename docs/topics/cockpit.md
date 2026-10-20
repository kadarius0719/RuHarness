# The cockpit

By the end of this page you will know what the cockpit's screen shows, how the Enter menu and its dialogs keep every action deliberate, and what the chat may and may not do.

## What the screen shows

The **cockpit** (the program `harness-tui`) is a full-screen view of a project inside Terminal. It runs the same `harness` commands you could type, and reads and writes the same **ledger** (the project's record folder), so the cockpit and the command line can be mixed.

A copy of a project that already holds migration results (copied from another computer, or another checkout) first asks, in plain text before the full screen, `Adopt this folder?`: type `y` and `Enter` to trust its results here, anything else to leave it untouched. If a command later fails because the folder is not adopted, the cockpit asks the same question again in a dialog, and `Enter`'s menu offers **Adopt this folder…** until you answer.

On a C project with no `harness.toml` (one you have not mapped yet, or one with several tools) the cockpit opens on the **Project view** instead: one list on the left — the programs the map found, each with its state in words (`links`, `needs a decision`, `accepted`) — and on the right the Next step in one line, then a card for the selected row (its file and what the map guesses it is, such as `src/tools/lzg.c · looks like a tool (it is not in a test, example or benchmark folder)`, a model's label once one was asked, each decision with its files numbered and the model's advice and why, and what `Enter` does), and in each row's `Enter` menu **Map the project** (again), **Write the build configuration** (it opens `migration/map/config.toml` in your editor, filled in with the map's example; afterwards you choose Map the project again), **Accept this program**, and, under the model line, **Ask for a proposal — ask in chat** (the chat proposes the build) or **Ask for advice — ask in chat** (on the held choices), and **Ask in chat…**. These type the question into the chat for you — `Which file should t-unlzg keep for choice d1, and why?` or `Propose the build configuration for this project from its Makefiles` — and you press `Enter` to send it, as **Migrate — ask in chat** does for a unit. The chat reads the map's open question through the harness and records its answer as advice: the advice then shows on the card, the advised file marked `← the model advises this one` with the model's why, and again when you Accept (the choice stays yours). On the command line, `harness project ask` still writes the question for a model you answer yourself (the models topic). An accepted program's row is its tool's row: `Enter` on it leads with **Open tool t-lzg**; right after an Accept it is the Next step and is selected (the map topic says more). The focused pane's frame is drawn in the heading colour, and the chat, when it has the focus, takes half of the width beside the left pane. A tool's screen keeps a first row, `◂ Back to the project view`.

**The steps, on every screen.** Under the View's title one line says where you are among the eight steps a program goes through: `Map ✓ · Configure ✓ · Accept program ✓ · Scan ✓ · Plan ✓ · Test program ○ · Rust · Accept Rust`. The first three are the project's (mapped, its build configuration written, the program accepted as a tool), the next two the tool's (its C scanned, its plan made), the last three each unit's: its **test program** (a small C program that calls the unit's functions and prints what comes back, which the judge checks against the C before it is trusted), its Rust, and your Accept. A step done has `✓` in the good colour, the step the `Next:` line names has `○` in the Next step's colour, the others are plain. On a unit's screen the last three are that unit's; on the tool's own row they count the pieces that can move past each step (`Test program 2/2 · Rust 1/2 ○ · Accept Rust 1/2`; a piece that calls other pieces stays C and is not counted); before a plan there are no counts, only `Scan ○` or `Plan ○`; on the Project view only the first three show, then `then: scan, plan, and each piece` (after you write a new build configuration, `Map again ○`). On a window under 100 columns (or a View too narrow for it) the line is only the step you are at: `Step 6 of 8: Test program`. The `Next:` line under it always names that step's action.

You work left to right. **Files**, on the left, lists the groups `Units`, `Features` and `Speed` first, then the project's source folder, folded once the project has units (its row sums up their marks; `→` opens it; a tool just accepted has no `Units` yet — scan and plan it first — and its folder starts open); the symbol beside each row is its state (`◇ no test program` or `◇ test program out of date`, `◈ test program ready`, `◐ Rust tried`, `⚑ waiting for your answer` (a try stopped at a question for a model: its Next names the answer file), `✓? Rust green — accept it`, `✓ done`, `· stays C (calls u-checksum)` (it calls another piece, so in this version it cannot move), `✗` failing, `⚠` needs attention, `!` changed since the scan, `+` new (not scanned yet), `·` a header or a file with no unit, `⊖` a file outside this tool; under Features, `◉` runs Rust now and `◌` still all C; `?` lists every mark). The Files pane (and the Project view's list) is as wide as its longest row needs, at least 30 columns and at most a third of the window, growing only while the View keeps 80; when a row is still too narrow its state word gets short (`no test`, `ready`, `tried`, `accept?`, `done`, `stays C`) and its name is cut in the middle, never under six letters (`u-checksum` in eight columns reads `u-ch…sum`), and the project's row keeps the tool's name (`…g-map · tool t-lzg`). `files_width = 40` at the top of `cockpit.toml` fixes its width instead (between 24 and half the window). A unit is listed once, under `Units`; its C file in the source folder shows the same mark. The **View**, on the right, shows the selection in detail: a **unit** (one program piece) in words first — its functions and their sizes, its files, its Rust, which features run it, its attempts, its checks — and its code folded under `▸ C source (Tab to the View, then Enter)` (in the View the heading reads `▸ C source (Enter to open)`; C beside its Rust once it has Rust, else the C alone); a file's code; the turns of an **attempt** (one translation try); the project's next step. A unit that needs attention leads with that one state and its cause, such as `needs attention: its verdict was made on another computer, not here yet`, and the judge's own words come under it, as `The judge's last verdict: …`. Selecting a unit's crate or one of its attempts in Files says what the row's words (`crate`, `green`, `red`, `truncated`, …) mean. The **Chat** sits beside them, or shares the right side on a window narrower than 156 **columns** (letters across the window). Each area is a **pane**; the one with the highlighted border has the **focus** (receives your keys), and `Tab` moves it. Under the panes, the activity line says what is running, or how the last command ended — `done`, `done, with a problem: …` or `failed (exit N): …` with its first error line, never `done` for a run that failed. A command that stops for a model's answer you give by hand (the hand-off) has not failed: the line says `waiting for your answer: <file> — write the reply there, then run Ask a model again (or answer it in the chat)` (for a unit's attempt: `then choose Resume`), in the needs-attention colour, and the Next step says the same. A long path on that line is shortened to its last three parts (`…/a/b/c`), and its end (`c for details`) stays in sight; the key bar below lists the keys that work right now.

**Colours and themes.** Colour tells the parts of the screen apart and says states. Pane titles and section headings (`Project`, `Files`, `Programs (3)`, `Libraries (1)`, `Checks`, …) have a colour of their own, in bold, and so does the frame of the pane that has the focus; notices (a file written, the project changed, the chat waiting) another; the status line at the bottom and the secondary labels (a program's file, `c for details`) a third and fourth in the designed themes; one colour marks only the `Next:` label (`Next:` on the Project view) and the menu's recommended item; and three colours say states: good (`✓`), failing (`✗`) and needs attention or waiting (`⚠`). A state colour always sits beside its mark or word, so nothing is said by colour alone, and nothing is dim or blinks. By default the cockpit uses your terminal's own blue (sections), cyan (the Next step), magenta (notices), green, red and yellow (the states), and its own foreground for everything else (the theme `terminal`), so your terminal's colour scheme decides how it looks. If text still looks faint or blends into the background, start it with `--theme dark` or `--theme light` (colours designed and checked for a dark or a white background), or `--theme mono` (no colour at all: bold, underline and reverse carry everything). `RUHARNESS_THEME=light` does the same as the flag; `NO_COLOR` picks mono.

To keep a choice, or to change one colour, write the settings file `cockpit.toml`: on a Mac `~/Library/Application Support/ruharness/cockpit.toml`, on Linux `~/.config/ruharness/cockpit.toml` (or the path in `RUHARNESS_COCKPIT_CONFIG`). The cockpit reads it at start; `--theme` beats it. A colour is a name (`red`, `bright-blue`, `default` for the terminal's own), a number from 0 to 255, or `#rrggbb`. Anything it cannot use is named in one sentence when the cockpit starts, and the default is used instead.

```toml
# ~/Library/Application Support/ruharness/cockpit.toml

files_width = 40         # the left pane's width, fixed (leave it out: it follows its rows)

[theme]
name = "terminal"        # terminal (the default), dark, light or mono

[colors]                 # any of the sixteen roles; leave out what you like as it is
# text      = "default"  # ordinary words
# muted     = 250        # code comments, a greyed item's reason
# heading   = "default"  # bold words that are not a section
# accent    = "bright-cyan"   # the Next step's label, the recommended item
# section   = "blue"     # pane titles and section headings, in bold
# status    = "default"  # the status line: "Ready…", "Last: …", a running command
# notice    = "magenta"  # notices: a file written, the project changed
# label     = 248        # secondary labels: a program's file, "c for details"
# good      = "green"    # beside ✓
# bad       = "red"      # beside ✗
attention   = "#ffaf00"  # beside ⚠ — an amber, for example
# selected  = "default"  # the selected row is reversed
# disabled  = 249        # a greyed menu item
# border    = 244        # an unfocused pane's frame
# backdrop  = 243        # the panes behind an open dialog, menu or help
# panel     = 237        # the background of a dialog, a menu and the help
```

A row with `▸` is folded and `▾` is open; `→` and `←` open and close. `Esc` steps back one level. `g` reads the project again, for example after a command you typed elsewhere. `c` shows the exact command of the last action and everything it reported.

## The Enter menu and the dialogs

Every View starts, under the steps line, with a **Next step** line: what to do now, in one sentence (on the project: scan, plan, the unit that needs attention and why, the first unit to migrate, …). Select a row and press `Enter`: a short menu lists what you can do with it now, the Next step's item first and highlighted, with a `↳ recommended: …` line saying why. The row is already shown in the View, so there is no Open item (except on a window too narrow to show both, and the Project view's **Open tool**, which changes screen). A menu that would hold only the Next step's own item is skipped: `Enter` opens its dialog straight away. An item that cannot run is greyed, and says why when you choose it. Items that ask a model follow one rule everywhere: the recommended item stays first, and when it asks a model its `↳ recommended` line ends `· uses a model`; every other such item sits below the line `── Uses a model — can take minutes ──`.

**A unit's test program.** A unit with no test program says so in Files (`◇ no test program`), and its Next step is `Make the test program — Enter, then ask in chat`: its menu leads with **Make the test program — ask in chat**, which types the request into the chat; the chat asks for it, and its dialog (`Make u-checksum's test program?`) says what the chat writes, that the judge runs it against the C before keeping it, and what is written (the driver under the unit's folder, and the plan's entry for it), then the exact `harness gen-driver … --requester=chat` command. Once it is ready the unit reads `◈ test program ready` and its Next step is its Rust. Until then **Migrate — ask in chat** is greyed: `it has no test program yet — Make the test program first`. On the command line, `harness gen-driver <unit>` does the same. With the chat off (`--no-chat`), the Next line and the greyed items name the command-line step instead, spelled for the tool: `Make the test program — run harness gen-driver u-version --tool t-lzg (the chat is off)`, then `Migrate — run harness migrate u-version --no-promote --tool t-lzg`, so the Accept stays yours in the cockpit.

Every item that changes something opens a **dialog** (a confirming box). While a dialog, a menu or the help is open, the screen behind it turns one grey (dim in the mono theme), and the box itself stands on a background one shade off the page (in the dark and light themes), its frame and title in the heading colour, its command in the label colour, and only the button `Enter` would press reversed (the armed one in the Next step's colour). If the page behind a box vanishes in your terminal's colours (the terminal theme draws it in the terminal's own "bright black", which is the background on Solarized Dark), `backdrop = "244"` (or any colour) under `[colors]` in `cockpit.toml` fixes it. A greyed item's reason shows whole: the menu widens to it, and wraps what still does not fit. It says in words what it will write, then shows the exact command. It starts on the safe button, must be scrolled to its end, and only after a moment whole on screen says `ready: → then Enter, or y`. Confirming takes `y`, or two keys, `→` then `Enter`, so one stray `Enter` never runs anything; keys typed or pasted before `ready` are ignored. Only one command runs at a time. In the cockpit nothing is ever accepted automatically: making a GREEN try the unit's Rust is always your **Accept** (its dialog's button says `Accept`). The command line's `harness migrate` accepts a green try itself unless you pass `--no-promote`.

## The chat

The chat is your own Claude Code, signed in with your own account. The cockpit starts it when you send your first message ("starting Claude Code (signed in with …)"); it never uses a key of the cockpit's. It can look up the project's state and **ask** the cockpit for model work, but it has no tool that writes a file or runs a command — the one thing it records itself is its answer to the project map's question, kept as advice.

**Signing in.** Before the chat starts (your first message, or any "ask in chat" item) the cockpit checks that Claude Code is installed (it runs `claude --version`). Whether it is signed in shows only when it starts. If something is wrong, a dialog says so in plain words — `Claude Code is not installed`, `Claude Code is not signed in`, or `Claude Code stopped` — with the steps:

```
1. Open a new Terminal window.
2. Type claude and press Return.
3. Follow the sign-in it shows.
4. Come back here and choose Check again.
What the cockpit saw: Claude Code said: Not logged in · Please run /login
 [ Cancel  Esc ] [ Check again  y ]
```

One more dialog is not about signing in: `The chat's harness tools are out of date`. The chat reads the project through a second program, `harness-mcp`, and when the cockpit was rebuilt but that program was not (`cargo run -p harness-tui` or `cargo build -p harness-tui` builds the cockpit alone), it lacks a tool the chat needs. The dialog's step 2 is the one line to paste, for example `cd ~/code/RuHarness && cargo build -p harness-mcp`; then Check again. Building the three programs together avoids it: `cargo build -p harness-cli -p harness-tui -p harness-mcp`.

**Check again** looks for `claude` again and, after a failed sign-in, sends your last message again in a new chat. While the chat cannot be used, its items (**Make the test program — ask in chat**, **Migrate — ask in chat**, the Project view's ask-in-chat items, **Ask in chat…**) are greyed with that reason, the menu offers **Check the chat's connection…** to open the dialog again, and the Next step says the chat is unavailable and leads to that dialog instead; with `--no-chat`, the Next step names the command-line step for the same work.

| The chat can ask to… | Only you can… |
|---|---|
| Make a unit's test program | Find risky patterns in the C |
| Migrate a unit (a new translation) | Scan the project |
| Modify an attempt with a note | Refresh the plan |
| Retry an attempt | Re-check with the judge |
| Answer the model's questions during a migration (Continue) | Accept an attempt |
| Answer the project map's question (its advice is recorded at once; it builds nothing) | Accept a program, and pick each held choice |

A request appears on a bold line above the input, such as `Asks: Migrate u-checksum — a model call, answered here in chat`, and ignores keys for its first second. `Enter` reviews it in the same dialog as your own actions; `Esc` declines it. Everything the chat asked for is recorded as "asked in chat".

When you confirm a migration the chat asked for, the dialog says the chat will answer its model turns itself. Each answer then continues the run after a quiet second ("waits for a quiet moment; Esc holds it"). That permission ends as soon as you hold, decline or cancel one, stop the chat or start a new one; it is never given when the cockpit runs without its **sandbox** (locked-down build space), and it never accepts anything. Quitting the cockpit stops the chat and any command it started.

## Try it

These boxes assume the guided liblzg run, with `u-checksum` verified. Make the window wide, and press one key at a time.

```bash
harness-tui --target targets/lzg
```

Look for: Files on the left with `lzg` as its first row and a summary such as `✓2/5` at its right edge (2 of its 5 C files are in Rust; the View's title says it in words), then `Units (5)`, `Features` and `Speed`, and the folded source folder last; the View on the right starting with the steps line (`… Test program 2/2 ✓ · Rust 2/2 ✓ · Accept Rust 2/2 ✓`: the two pieces that can move are done; or `All 8 steps done ✓` on a narrower window), then `Next:`; and `Ready — ↑↓ to choose a row, Enter for what you can do` under the panes (only `Ready.` while a menu, a dialog, the View or the chat has the keys).

```text
?
```

Look for: a first page of six keys and three marks (`◇` no Rust yet, `✓` done, `⚠` needs attention); `↓` shows every key and every symbol, each with its meaning; any other key closes it.

```text
v      (with u-checksum, under Units (5), selected)
```

Look for: **Show the checks**, each check in words with `✓`, such as `✓ same outputs as C`; `Esc` closes it.

```text
Enter  (on u-checksum), then choose Re-check with the judge
```

Look for: a dialog that shows `… verify u-checksum --target=…` and, after a moment, `ready`; `y` (or `→` then `Enter`) runs it, and the activity line ends `Ready. Last: Re-check u-checksum — GREEN — all … checks passed`. `Esc` instead backs out without running anything.

```text
q
```

Look for: your Terminal prompt; if you used the chat, a `Quit the cockpit?` dialog first, where a second `q` once it is ready quits.

The full walk: docs/TESTING-GUIDE.md Part 9. The ideas in full: docs/TUTORIAL.md section 9.
