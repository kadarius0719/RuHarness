# The cockpit

By the end of this page you will know what the cockpit's screen shows, how the Enter menu and its dialogs keep every action deliberate, and what the chat may and may not do.

## What the screen shows

The **cockpit** (the program `harness-tui`) is a full-screen view of a project inside Terminal. It runs the same `harness` commands you could type, and reads and writes the same **ledger** (the project's record folder), so the cockpit and the command line can be mixed.

You work left to right. **Files**, on the left, lists the project's C files, then the groups `Units`, `Features` and `Speed`; the symbol beside each row is its state (`✓` migrated, `◇` planned, `◐` tried, `✗` failing, `⚠` needs attention, `!` changed since the scan; `?` lists them all). The **View**, on the right, shows the selection in detail: a file's C beside its Rust, the checks of a **unit** (one program piece), the turns of an **attempt** (one translation try), the project's next step. The **Chat** sits beside them, or shares the right side on a window narrower than 156 **columns** (letters across the window). Each area is a **pane**; the one with the highlighted border has the **focus** (receives your keys), and `Tab` moves it. Under the panes, the activity line says what is running; the key bar below lists the keys that work right now.

A row with `▸` is folded and `▾` is open; `→` and `←` open and close. `Esc` steps back one level. `g` reads the project again, for example after a command you typed elsewhere. `c` shows the exact command of the last action and everything it reported.

## The Enter menu and the dialogs

Select a row and press `Enter`: a short menu lists what you can do with it now. An item that cannot run is greyed, and says why when you choose it. Items that ask a model sit below the line `── Uses a model — can take minutes ──`.

Every item that changes something opens a **dialog** (a confirming box). It says in words what it will write, then shows the exact command. It starts on the safe button, must be scrolled to its end, and only after a moment whole on screen says `ready: → then Enter`. Confirming takes two keys, `→` then `Enter`, so one stray `Enter` never runs anything; keys typed or pasted before `ready` are ignored. Only one command runs at a time. Nothing is ever accepted automatically: making a GREEN attempt the unit's Rust is always your **Accept**.

## The chat

The chat is your own Claude Code, signed in with your own account. The cockpit starts it when you send your first message ("starting Claude Code (signed in with …)"); it never uses a key of the cockpit's. It can look up the project's state and **ask** the cockpit for model work, but it has no tool that writes a file or runs a command.

| The chat can ask to… | Only you can… |
|---|---|
| Migrate a unit (a new translation) | Scan the project |
| Modify an attempt with a note | Refresh the plan |
| Retry an attempt | Re-check with the oracle |
| Answer the model's questions during a migration (Continue) | Accept an attempt |

A request appears on a yellow line above the input, such as `Asks: Migrate u-checksum — a model call, answered here in chat`, and ignores keys for its first second. `Enter` reviews it in the same dialog as your own actions; `Esc` declines it. Everything the chat asked for is recorded as "asked in chat".

When you confirm a migration the chat asked for, the dialog says the chat will answer its model turns itself. Each answer then continues the run after a quiet second ("waits for a quiet moment; Esc holds it"). That permission ends as soon as you hold, decline or cancel one, stop the chat or start a new one; it is never given when the cockpit runs without its **sandbox** (locked-down build space), and it never accepts anything. Quitting the cockpit stops the chat and any command it started.

## Try it

These boxes assume the guided liblzg run, with `u-checksum` verified. Make the window wide, and press one key at a time.

```bash
harness-tui --target targets/lzg
```

Look for: Files on the left with `lzg` as its first row and a summary such as `✓2/5` at its right edge, the View on the right, and `Ready.` under the panes.

```text
?
```

Look for: every key and every symbol, each with its meaning; any key closes it.

```text
v      (with u-checksum, under Units (5), selected)
```

Look for: **Show the checks**, each check in words with `✓`, such as `✓ same outputs as C`; `Esc` closes it.

```text
Enter  (on u-checksum), then choose Re-check with the oracle
```

Look for: a dialog that shows `… verify u-checksum --target=…` and, after a moment, `ready`; `→` then `Enter` runs it, and the activity line ends `Ready. Last: Re-check u-checksum — GREEN — all … checks passed`. `Esc` instead backs out without running anything.

```text
q
```

Look for: your Terminal prompt; if you used the chat, a `Quit the cockpit?` dialog first, where a second `q` once it is ready quits.

The full walk: docs/TESTING-GUIDE.md Part 9. The ideas in full: docs/TUTORIAL.md section 9.
