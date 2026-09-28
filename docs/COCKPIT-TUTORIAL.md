# Getting Started with the RuHarness Cockpit

The cockpit is a screen for moving a C program to Rust one piece at a time: you pick a piece, a model writes the Rust, an automatic checker judges it, and nothing becomes final until you accept it. This guide walks through every part of the screen, every action, and the built-in chat that can do the model work for you.

## The big idea

RuHarness moves a program written in C to Rust one small piece at a time, and proves each piece behaves exactly like the original before it counts. You never have to read or trust the new code yourself: an automatic judge decides.

The program is split into **units** — one C file, or a small group of files that belong together. The rest of the program stays in C while one unit moves. The new Rust unit keeps the same function names as the C it replaces, so it drops straight into the program.

Three parts do the work:

- **The harness** is the tool that reads the C, works out a safe order for the units, asks an AI model to write the Rust, and keeps records. It never decides whether a translation is right.
- **The oracle** is the judge. It runs the old C and the new Rust side by side on hundreds of the same inputs and demands identical output, runs the whole program both ways, and checks the Rust does nothing it should not. Every check passes → **GREEN**. Anything else → **RED**.
- **The ledger** is a folder of plain text files inside the project. Every scan, plan, attempt and verdict is written there, so the work can be picked up by anyone at any time.

```mermaid
flowchart LR
    pick["Pick a unit<br/>you, or ask the chat"] --> write["The model writes Rust,<br/>after you confirm"]
    write --> check{"Oracle: all<br/>checks pass?"}
    check -- yes --> green["GREEN attempt<br/>kept, not yet used"]
    green --> accept["You look, then Accept:<br/>the program uses the Rust"]
    check -- no --> repair["RED: the model<br/>gets a repair turn"]
    repair -- try again --> write
```

A RED verdict sends the Rust back to the model for repair (still RED after 3 repairs: modify, retry or hand edit); a GREEN one waits for you — until you accept it, the program keeps running the C.

The **cockpit** is the screen that puts all of this in front of you: the program's files and units on the left, the selected item's details on the right, and a chat where you can ask an AI assistant to do the model work. Nothing the model writes becomes part of the program until the oracle says GREEN **and** you accept it.

## Words you will see

| Word | What it means |
| --- | --- |
| Target | The project being migrated — the folder you open the cockpit on. |
| Unit | One piece of the program that moves to Rust as a whole: a C file or a few files that belong together. |
| Plan | The list of units in the order they can safely move (a unit goes after the units it depends on). |
| Attempt | One try at translating a unit. A unit can have many attempts; each is kept. |
| Turn | One round inside an attempt: the model writes, the oracle checks, and if something failed the model gets another turn to repair it (3 repairs by default). |
| Oracle | The automatic judge that compares the new Rust with the original C. |
| Checks | The oracle's individual tests (for example "same outputs as C"). A verdict lists each one as passed or failed. |
| GREEN / RED | The oracle's verdict: every check passed / at least one failed. |
| Hand-off | A point where the harness pauses and waits for an answer from a model — in the cockpit, the chat supplies it. |
| Accept | Your decision to make a GREEN attempt the unit's official Rust ("promote" it). Nothing is ever accepted automatically. |
| Modify with a note | Ask for a new attempt based on an earlier one, with your written guidance ("steer" in the records). |
| Retry | Ask the model for the same translation again, as a new attempt; the earlier one is kept. |
| Hand edit | Change the Rust yourself in an editor; the oracle still judges it. |
| Ledger | The folder of plain text records inside the target (`migration/`). |
| Scan | Read the C files and record what is in them (functions, which file uses which). |

## Before you start

You need a terminal window, the RuHarness tools built on your computer, and — for the chat — Claude Code installed and signed in. If someone set RuHarness up for you, they only need to give you the command in step 3.

1. **A terminal.** On a Mac, open the Terminal app. Make the window wide: at 156 characters or more the chat gets its own column; narrower, it shares the right side of the screen.
2. **The tools**, built once from the RuHarness folder (this needs Rust and a C compiler; on a Mac, `xcode-select --install` provides the compiler):

   ```bash
   cargo build --workspace
   ```
3. **Open the cockpit** on a project (the target folder), from the RuHarness folder:

   ```bash
   cargo run -p harness-tui -- --target targets/zopfli
   ```

   To practise without touching the real project, open a copy of the folder instead — everything the cockpit does is written inside the folder you point it at.
4. **For the chat:** Claude Code (`claude`) installed and signed in. The chat uses your own sign-in (your subscription or your API key) and says which before your first message. Add `--chat-model haiku` to the command in step 3 to use a smaller, cheaper model.

To leave, press `q` (outside the chat) or `Ctrl-C` in the chat. The cockpit asks first if a command is running or a conversation exists; the conversation is not saved.

## A tour of the screen

```
┌ Files ────────────────┐┌ View ───────────────────────────┐┌ Chat ─────────────────────────┐
│ The project's C files ││ What you selected, in detail:   ││ Ask for model work in plain   │
│ and its units, each   ││ a file's C beside its Rust,     ││ words; it asks the cockpit,   │
│ with a state symbol   ││ a unit's checks, an attempt's   ││ never runs anything itself    │
│                       ││ turns, the project's next step  ││                               │
│ Enter: what you can   ││                                 ││ Asks: Migrate u001-…          │
│ do with the selection ││                                 ││ [Review Enter] [Decline Esc]  │
│                       ││                                 ││                               │
│                       ││                                 ││ › your message                │
└───────────────────────┘└─────────────────────────────────┘└───────────────────────────────┘
 Activity line: what the running command is doing   [Cancel x] [Details c]
 Key bar: the keys that work right now — click one to press it
```

You work left to right: pick something in **Files**, read about it in the **View**, and ask the **Chat** for model work. The pane you are in has a highlighted border; `Tab` moves to the next one. Narrower than 156 columns, the chat shares the right side: a `View │ Chat` tab on its border switches.

- **Files** lists the project's C files (open a file to see its functions) and, below them, its **Units** with each unit's Rust and attempts. The symbol beside each row is its state.
- **View** shows the selection in detail: the project's summary and next step, a file's C beside its Rust, a unit's checks in words, or an attempt's turns.
- **Chat** is where you talk to the assistant. Its requests appear on a yellow line above the input with **[Review]** and **[Decline]** buttons.
- The **activity line** narrates the running command in plain words ("Turn 1: asking the model…", "Checked: same outputs as C — passed") and offers **[Cancel]** and **[Details]**.
- The **key bar** at the very bottom lists the keys that work right now; each is also a button you can click.

## Moving around

Everything works with the arrow keys and `Enter`, or with the mouse; there is nothing to memorise. The bottom row of the screen always lists the keys that work right now, and `?` opens the full help.

| Key | What it does |
| --- | --- |
| `↑` `↓` | Move up and down in the pane you are in (in the View: scroll) |
| `←` `→` | In Files: fold or open a row; `→` on the last level moves into the View, `←` comes back |
| `Enter` | Open the menu of what you can do with the selected row |
| `Esc` | Close a menu, dialog or overlay; from the View, go back to Files |
| `Tab` | Go to the next pane: Files → View → Chat |
| `?` | Help: every key, what each symbol means, the chat's keys |
| `c` | Details of the running command: the exact command and everything it reported |
| `x` | Cancel the running command (asked first) |
| `g` | Re-read the project, after a change made outside the cockpit |
| `t` | Try again, after a refusal |
| `q` | Quit (asked first while a command runs) |
| `a m e r R v d` | Shortcuts for menu items of the selected row: Accept, Modify, Hand edit, Retry, Resume, Show the checks, Compare |

**With the mouse:** a click selects a row and its pane; a click on `▸` or `▾` opens or folds it; a double click is `Enter`; the wheel scrolls; a click on a key in the bottom row presses it. To select text on screen, hold `Shift` while you drag (`Option` in iTerm2). If clicks print odd characters, start the cockpit with `--no-mouse`, or turn the mouse off with `m` in the help screen.

Inside the chat, letters are text you type — the chat's own keys are in "The chat" below.

## Doing things: the Enter menu

Select a row and press `Enter` (or double-click it): a short menu lists what you can do with that item right now. An item that cannot run yet is greyed, and choosing it tells you why (for example "a command is running (one at a time)").

| Menu item | Where | What it does |
| --- | --- | --- |
| Scan the project | The project (top row) | Reads the C files and records what is in them. Do this first on a new project, and again after the C changes. |
| Refresh the plan | The project | Works out the units and the safe order to move them in. |
| Find hazards (run the detectors) | The project | Flags C patterns that are risky to translate, so they get extra care. |
| Migrate — ask in chat | A unit not yet migrated, or the project | Puts a request in the chat for a translation of the unit. You still confirm it. |
| Ask in chat… | Anywhere | Moves you to the chat to ask about the selected item. |
| Re-check with the oracle | A unit | Runs the judge again on the unit's current Rust. |
| Show the checks | A unit or attempt with a verdict | Lists every check in words, passed or failed, with what it compared. |
| Accept … into … | A GREEN attempt not yet accepted | Makes that attempt the unit's official Rust. Only you can do this. |
| Compare with the promoted attempt | An attempt | Shows what differs from the Rust already accepted. |
| Modify with a note | An attempt | Starts a new attempt from this one, guided by a note you write ("handle the empty-input case like the C does"). |
| Retry | An attempt | Asks the model for the same translation again, as a new attempt. |
| Resume | A paused attempt | Continues an attempt that was waiting for an answer. |
| Hand edit | A unit's Rust | Opens the Rust in your editor; the oracle judges your edit, and it is recorded as a human attempt. It is never accepted automatically. |
| Cancel the running command | While something runs | Stops it, after asking. |
| Re-read the project | Anywhere | Loads the ledger again (same as `g`). |

Every item that changes something opens a confirmation dialog first — next section. Each unit's state is shown by a symbol beside it:

| Symbol | Meaning |
| --- | --- |
| `✓` | Migrated: its Rust is accepted and GREEN |
| `✓?` | Verified, but no recorded attempt is known to have produced it |
| `✗` | Failing: the latest verdict is RED |
| `⚠` | Needs attention: a cause you can fix, named in the View |
| `◐` | Tried: attempts exist, none accepted yet |
| `◇` | Planned: no attempt yet |
| `⊘` | Blocked |
| `!` | Changed since the last scan |
| `+` | Not scanned yet |
| `?` | Missing: the scan recorded it, the folder no longer has it |

## Confirming an action

Nothing runs until you confirm it in a dialog, and the dialog is built so you cannot confirm by accident. It shows, in words, what the action will write and change, then the exact command it will run.

1. **Read it.** The dialog starts on the safe button (`Cancel`). If it is longer than the window, scroll to the end with `↓` or the wheel — it will not unlock until you have seen all of it ("↓ more below — scroll to the end").
2. **Wait for "ready".** After it has been on screen, whole, for a moment with no keys pending, the status line turns green: "ready: → then Enter". Keys typed or pasted before that point, and a held-down `Enter`, never confirm anything.
3. **Confirm or cancel.** Press `→` to move to the action's button (`Run`), then `Enter`. Or press `Esc` to cancel. With the mouse, the buttons answer a click a second after the dialog opened, and the action button only once it says ready.

If you click too early the dialog says "Too soon — click again". While a command runs, `c` shows its details and `x` offers to cancel it ("Keep running" or "Stop it"). Only one command runs at a time.

## The chat

The chat is an AI assistant (your own Claude Code) that can look up the project's state — its units, verdicts, attempts and pending questions — and ask the cockpit to do model work for you — but it never runs anything itself. Every request it makes waits for you to review and confirm it.

**Getting there.** Press `Tab` until the chat is highlighted, or click **Chat**. On a wide window the chat has its own column; otherwise it takes the right side while you are in it, and a `View │ Chat` tab on the right pane's border brings it back. "Chat ●" means it has something for you. You can also choose **Migrate — ask in chat** or **Ask in chat…** from a row's menu.

**Talking to it.** Type in plain words and press `Enter`: "Please migrate u001-katajainen", "Why is this unit red?", "What does this function do?". It knows what you have selected. The pane's title shows what it is doing: *starting…*, *thinking…*, *ready*, *waiting for you*, *running a command for the chat*, *continuing…*, *stopped*, *ended*.

**What it can ask for, and what stays yours.**

| The chat can ask to… | Only you can… |
| --- | --- |
| Migrate a unit (a new translation) | Scan the project |
| Modify an attempt with a note | Refresh the plan |
| Retry an attempt | Re-check with the oracle |
| Answer the model's questions during a migration (Continue) | Accept an attempt |

For anything in the right column the chat tells you which menu item to use.

**When it asks.** A yellow line appears above the input — "Asks: Migrate u001-katajainen …" — with buttons. It ignores keys for its first second so a keystroke meant for something else never answers it. Then:

- `Enter` (with an empty input) or **[Review Enter]** opens the usual confirmation dialog, with the exact command and a line saying the chat asked for it.
- `Esc` or **[Decline Esc]** says no; the chat is told.
- **[Decline with my draft]** says no and sends what you typed as the reason ("use the other unit first").

**Letting it continue on its own.** A migration is a conversation with the model: it writes Rust, the oracle checks it, and the model gets turns to repair what failed. When you confirm a migration the chat asked for, the dialog tells you the chat will answer these turns itself ("up to 4"). Each answer then continues the run after a quiet second, with a line "Continues … — waits for a quiet moment; Esc holds it". This permission ends as soon as you hold one (`Esc`), decline or cancel one, stop the chat or start a new one — after that every answer asks you first. It is never given when the cockpit runs without its sandbox. It never accepts anything: Accept is always yours.

**The chat's keys** (letters are text here):

| Key | What it does |
| --- | --- |
| `Enter` | Send your message (or review a waiting request, when the input is empty) |
| `Ctrl-J`, `Alt-Enter`, or `\` then `Enter` | New line in your message |
| `Esc` | Decline a request, hold a waiting Continue, or stop the reply |
| `Ctrl-C` | Stop the reply; press again to clear your draft; then quit (asked) |
| `Ctrl-X` | Cancel the running command (asked) |
| `Ctrl-N` | Start a new chat (asked); the new one does not see the old conversation |
| `↑` `↓` `PgUp` `PgDn` | Scroll the conversation |
| `Tab` | Leave the chat |
| `F1` | Help |

If you leave the chat with an unsent draft, letters in the other panes are ignored until you press an arrow, `Tab` or `Esc` — so typing that was meant for the chat never triggers a shortcut. Everything the chat asked for is recorded as "asked in chat".

## Your first migration, step by step

This walk-through migrates one unit through the chat, from asking to accepting. It takes a few minutes, most of it waiting for the model and the oracle. The example uses a practice copy of the zopfli project in which the unit `u001-katajainen` has been reset to planned (in the project itself it is already migrated). In your own project, any unit marked `◇` (planned) or `✗` (failing) works the same way.

1. **Open the cockpit** on the project (see "Before you start"). The Files pane lists the C files and, below them, the Units.
2. **Find the unit.** Move down to it with `↓` and look at the View on the right: its state, its C files and what it depends on.
3. **Go to the chat.** Press `Tab` until the chat is highlighted. The first time, it says "starting Claude Code (signed in with …)".
4. **Ask.** Type `Please migrate u001-katajainen.` and press `Enter`. The chat reads the project, then a yellow line appears: "Asks: Migrate u001-katajainen — a model call, answered here in chat".
5. **Review.** Wait a second, then press `Enter`. The confirmation dialog opens: who asked (the chat and its model), that the chat will answer the model's turns, and the exact command.
6. **Confirm.** When it says "ready", press `→` then `Enter` (or `Esc` to say no).
7. **Watch.** The line under the panes narrates the run: "Turn 1: asking the model…", "Checked: same outputs as C — passed". Each time the model's answer is needed, the chat reads the request and answers it; the answer continues the run after a quiet second. Press `Esc` on the "Continues …" line if you want to look first — from then on each answer asks you.
8. **Read the verdict.** The chat shows a line such as "✓ Migrate u001-katajainen (asked in chat) — GREEN, 8 of 8 checks passed" and explains it. In Files, the unit now has a new attempt.
9. **Look before you accept.** Select the new attempt in Files: the View shows its Rust beside the C, and **Show the checks** (`v`) lists every check. **Compare** (`d`) shows what differs from any Rust already accepted.
10. **Accept.** Press `Enter` on the GREEN attempt, choose **Accept … into u001-katajainen**, and confirm. The unit turns `✓` — its Rust is now the program's.

If the verdict is RED, nothing changes in the program: see the next section.

## Reading the results

Every run ends in a verdict from the oracle. GREEN means the new Rust matched the C on every check; RED means at least one check failed. A RED attempt changes nothing in the program — it is kept on record, and you choose what to try next.

**Where to look.** Select the attempt (or its unit) and press `v` for **Show the checks**: each check in words, with what it compared and whether it passed. The View also shows the attempt's turns: what the model wrote each time and what the oracle said.

| What you see | What to do |
| --- | --- |
| GREEN, not yet accepted (`◐` on the unit) | Look at the Rust and the checks, then **Accept** it. Until you do, the program keeps its old code. |
| RED after the model's repair turns | Ask the chat "why is this red?"; then ask it to **modify** the attempt with guidance ("keep the counts unsigned like the C") or to **retry** for a fresh translation. |
| RED, and you know the fix | **Hand edit** the Rust yourself; the oracle judges your edit like any other. |
| `⚠` needs attention | The View names the cause and how to fix it. |
| `!` changed since scan | The C files changed: **Scan the project**, then **Refresh the plan**. |
| "refused: …" in the activity line | The harness would not start the action; the words say why (for example another command holds the project). `t` tries again once the cause is gone. |

A GREEN verdict is only as good as the tests the oracle ran, which is why accepting stays a human decision. The model's output quality varies: the same request can come back GREEN one time and RED the next — a retry is normal, not a failure of the tool.

## What keeps you safe

The cockpit is built on a few rules it never breaks, so you can let the model and the chat work without watching every step.

- **Nothing is accepted without you.** Not by the model, not by the chat, not by a GREEN verdict. Only your **Accept** changes the program's Rust.
- **The chat never runs anything.** It can only ask; every request goes through the same confirmation dialog as your own actions, showing the exact command. The single exception is the permission you give when you confirm a migration it asked for — letting it answer that run's model turns — and that ends the moment you hold, decline, cancel, stop or start a new chat.
- **No action runs by accident.** Dialogs start on Cancel, wait until they have been fully on screen, and ignore keys typed or pasted ahead and a held `Enter`. A click only counts a second after a dialog opened.
- **Model-written code runs in a sandbox:** no network, no access to your home folder, and a time limit. The dialog says so if the cockpit was started without it (`--allow-unsandboxed`), and the chat never gets permission to continue on its own then.
- **Everything is on record.** Every attempt, verdict and hand-off is written to the ledger in plain text; what the chat asked for is labelled "asked in chat". Verdicts are tied to the exact code they judged, so a change afterwards shows as needing a re-check.
- **The chat cannot change files.** Its only tools are the cockpit's read-only look-ups and its requests; it has no tool that writes a file or runs a command. Your sign-in stays with Claude Code, never with the cockpit.
- **Quitting cleans up.** Closing the cockpit or its terminal window stops the chat and any command it started.

## When something looks wrong

The cockpit says what went wrong in words, usually with what to do. The common cases:

| What you see | What it means, and what to do |
| --- | --- |
| A menu item is greyed | It cannot run right now. Choose it anyway: the reason appears at the bottom of the menu. |
| The dialog never says "ready" | Scroll to its end with `↓`; if it says "too small", make the window bigger (or press `Esc`). |
| "The chat is unavailable here: …" (in Help) | Claude Code was not found, or the cockpit was started with `--no-chat`. Install Claude Code and sign in, then restart. |
| "the chat has not started — a sign-in or keychain prompt may be waiting" | Run `claude` once in a normal terminal to finish signing in, then press `Ctrl-N` for a new chat. |
| "the harness tools did not start … — the chat is off" | The chat's helper program is missing or broken. Rebuild with `cargo build --workspace` and restart. |
| "the chat ended: …" | The chat stopped; the words say why (for example an unknown model name given with `--chat-model`). Your next message starts a new chat. |
| "nothing from the model for a minute" | The model is slow or stuck. `Esc` stops the reply; `Ctrl-N` starts over. |
| "a message the cockpit did not send reached the chat" | Something outside the cockpit wrote into the conversation. As a precaution the chat loses its permission to continue on its own; start a new chat with `Ctrl-N` if you did not expect it. |
| Letters do nothing in Files or View | You left the chat with an unsent draft, so typing is paused there. Press an arrow, `Tab` or `Esc`. |
| "refused: … locked" | Another command (perhaps in another window) is working on the project. Wait for it, then press `t` to try again. |
| Clicks print odd characters | Your terminal's mouse mode is not supported: restart with `--no-mouse`. |
| The results look out of date | Something changed the project outside the cockpit: press `g` to re-read it. |
| Actions behave like an older version | An older `harness` tool installed on your computer is being used. Reinstall it (`cargo install --path crates/harness-cli`) or start the cockpit with `--harness target/debug/harness`. |

Still stuck? Press `c` while a command runs, or after it ends, to see its exact command and everything it reported — that is what to share when you ask for help.
