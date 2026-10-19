# The cockpit

By the end of this page you will know what the cockpit's screen shows, how the Enter menu and its dialogs keep every action deliberate, and what the chat may and may not do.

## What the screen shows

The **cockpit** (the program `harness-tui`) is a full-screen view of a project inside Terminal. It runs the same `harness` commands you could type, and reads and writes the same **ledger** (the project's record folder), so the cockpit and the command line can be mixed.

A copy of a project that already holds migration results (copied from another computer, or another checkout) first asks, in plain text before the full screen, `Adopt this folder?`: type `y` and `Enter` to trust its results here, anything else to leave it untouched. If a command later fails because the folder is not adopted, the cockpit asks the same question again in a dialog, and `Enter`'s menu offers **Adopt this folder…** until you answer.

On a C project with no `harness.toml` (one you have not mapped yet, or one with several tools) the cockpit opens on the **Project view** instead: the programs the map found on the left (each with its file and what the map guesses it is, such as `src/tools/lzg.c (a tool, by its folder)`, and a model's label once one was asked), the project and its Next step on the right, and in each row's `Enter` menu **Map the project** (again), **Write the build configuration** (it opens `migration/map/config.toml` in your editor, filled in with the map's example; afterwards you choose Map the project again), **Accept this program**, and, under the model line, **Ask for a proposal** (a model proposes the build) or **Ask a model for advice** (on the held choices). Each accepted tool gets a row **Open tool t-lzg — src/tools/lzg.c**; right after an Accept it is the Next step and is selected (the map topic says more). A tool's screen keeps a first row, `◂ Back to the project view`.

You work left to right. **Files**, on the left, lists the groups `Units`, `Features` and `Speed` first, then the project's source folder, folded once the project has units (its row sums up their marks; `→` opens it; a tool just accepted has no `Units` yet — scan and plan it first — and its folder starts open); the symbol beside each row is its state (`✓` migrated, `◇` planned, `◐` tried, `✗` failing, `⚠` needs attention, `!` changed since the scan, `+` new (not scanned yet), `·` a header or a file with no unit, `⊖` a file outside this tool; under Features, `◉` runs Rust now and `◌` still all C; `?` lists every mark). When a row is too narrow, its name is shortened, never its state word. A unit is listed once, under `Units`; its C file in the source folder shows the same mark. The **View**, on the right, shows the selection in detail: a **unit** (one program piece) in words first — its functions and their sizes, its files, its Rust, which features run it, its attempts, its checks — and its code folded under `▸ C source (Tab to the View, then Enter)` (in the View the heading reads `▸ C source (Enter to open)`; C beside its Rust once it has Rust, else the C alone); a file's code; the turns of an **attempt** (one translation try); the project's next step. A unit that needs attention leads with that one state and its cause, such as `needs attention: its verdict was made on another computer, not here yet`, and the judge's own words come under it, as `The judge's last verdict: …`. Selecting a unit's crate or one of its attempts in Files says what the row's words (`crate`, `green`, `red`, `truncated`, …) mean. The **Chat** sits beside them, or shares the right side on a window narrower than 156 **columns** (letters across the window). Each area is a **pane**; the one with the highlighted border has the **focus** (receives your keys), and `Tab` moves it. Under the panes, the activity line says what is running, or how the last command ended — `done`, `done, with a problem: …` or `failed (exit N): …` with its first error line, never `done` for a run that failed; the key bar below lists the keys that work right now.

**Colours and themes.** The screen is plain text with bold headings, one colour for the `Next step:` label and the menu's recommended item, and three colours for states: good (`✓`), failing (`✗`) and needs attention (`⚠`). A state colour always sits beside its mark or word, so nothing is said by colour alone, and nothing is dim or blinks. By default the cockpit uses your terminal's own green, red, yellow and cyan (the theme `terminal`), so your terminal's colour scheme decides how it looks. If text still looks faint or blends into the background, start it with `--theme dark` or `--theme light` (colours designed and checked for a dark or a white background), or `--theme mono` (no colour at all: bold, underline and reverse carry everything). `RUHARNESS_THEME=light` does the same as the flag; `NO_COLOR` picks mono.

To keep a choice, or to change one colour, write the settings file `cockpit.toml`: on a Mac `~/Library/Application Support/ruharness/cockpit.toml`, on Linux `~/.config/ruharness/cockpit.toml` (or the path in `RUHARNESS_COCKPIT_CONFIG`). The cockpit reads it at start; `--theme` beats it. A colour is a name (`red`, `bright-blue`, `default` for the terminal's own), a number from 0 to 255, or `#rrggbb`. Anything it cannot use is named in one sentence when the cockpit starts, and the default is used instead.

```toml
# ~/Library/Application Support/ruharness/cockpit.toml

[theme]
name = "terminal"        # terminal (the default), dark, light or mono

[colors]                 # any of the ten roles; leave out what you like as it is
# text      = "default"  # ordinary words
# muted     = 250        # secondary words (the file's path, "Last: …")
# heading   = "default"  # headings are bold
# accent    = "bright-cyan"   # the Next step's label, the recommended item
# good      = "green"    # beside ✓
# bad       = "red"      # beside ✗
attention   = "#ffaf00"  # beside ⚠ — an amber, for example
# selected  = "default"  # the selected row is reversed
# disabled  = 245        # a greyed menu item
# border    = 244        # an unfocused pane's frame
```

A row with `▸` is folded and `▾` is open; `→` and `←` open and close. `Esc` steps back one level. `g` reads the project again, for example after a command you typed elsewhere. `c` shows the exact command of the last action and everything it reported.

## The Enter menu and the dialogs

Every View starts with a **Next step** line: what to do now, in one sentence (on the project: scan, plan, the unit that needs attention and why, the first unit to migrate, …). Select a row and press `Enter`: a short menu lists what you can do with it now, the Next step's item first and highlighted, with a `↳ recommended: …` line saying why. The row is already shown in the View, so there is no Open item (except on a window too narrow to show both, and the Project view's **Open tool**, which changes screen). A menu that would hold only the Next step's own item is skipped: `Enter` opens its dialog straight away. An item that cannot run is greyed, and says why when you choose it. Items that ask a model follow one rule everywhere: the recommended item stays first, and when it asks a model its `↳ recommended` line ends `· uses a model`; every other such item sits below the line `── Uses a model — can take minutes ──`.

Every item that changes something opens a **dialog** (a confirming box). It says in words what it will write, then shows the exact command. It starts on the safe button, must be scrolled to its end, and only after a moment whole on screen says `ready: → then Enter, or y`. Confirming takes `y`, or two keys, `→` then `Enter`, so one stray `Enter` never runs anything; keys typed or pasted before `ready` are ignored. Only one command runs at a time. Nothing is ever accepted automatically: making a GREEN attempt the unit's Rust is always your **Accept**.

## The chat

The chat is your own Claude Code, signed in with your own account. The cockpit starts it when you send your first message ("starting Claude Code (signed in with …)"); it never uses a key of the cockpit's. It can look up the project's state and **ask** the cockpit for model work, but it has no tool that writes a file or runs a command.

| The chat can ask to… | Only you can… |
|---|---|
| Migrate a unit (a new translation) | Scan the project |
| Modify an attempt with a note | Refresh the plan |
| Retry an attempt | Re-check with the judge |
| Answer the model's questions during a migration (Continue) | Accept an attempt |

A request appears on a bold line above the input, such as `Asks: Migrate u-checksum — a model call, answered here in chat`, and ignores keys for its first second. `Enter` reviews it in the same dialog as your own actions; `Esc` declines it. Everything the chat asked for is recorded as "asked in chat".

When you confirm a migration the chat asked for, the dialog says the chat will answer its model turns itself. Each answer then continues the run after a quiet second ("waits for a quiet moment; Esc holds it"). That permission ends as soon as you hold, decline or cancel one, stop the chat or start a new one; it is never given when the cockpit runs without its **sandbox** (locked-down build space), and it never accepts anything. Quitting the cockpit stops the chat and any command it started.

## Try it

These boxes assume the guided liblzg run, with `u-checksum` verified. Make the window wide, and press one key at a time.

```bash
harness-tui --target targets/lzg
```

Look for: Files on the left with `lzg` as its first row and a summary such as `✓2/5` at its right edge (2 of its 5 C files are in Rust; the View's title says it in words), then `Units (5)`, `Features` and `Speed`, and the folded source folder last; the View on the right starting with `Next step:`; and `Ready — ↑↓ to choose a row, Enter for what you can do` under the panes (only `Ready.` while a menu, a dialog, the View or the chat has the keys).

```text
?
```

Look for: a first page of six keys and three marks (`◇` planned, `✓` in Rust, `⚠` needs attention); `↓` shows every key and every symbol, each with its meaning; any other key closes it.

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
