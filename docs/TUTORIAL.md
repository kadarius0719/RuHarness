# Understanding RuHarness

RuHarness moves a program written in C to Rust one piece at a time: a model writes the Rust, an automatic judge proves it behaves exactly like the C, and nothing becomes final until you accept it. This page explains how and why, in plain words, for a careful reader who has never programmed.

By the end you will know what a program is made of and how it is built; what C and Rust are and why anyone moves a program from one to the other; how RuHarness splits the work into pieces, asks a model for each, and judges the answer; what that proof covers and what it does not; and how to read everything the cockpit and the commands show you.

This page is the companion of the [testing guide](TESTING-GUIDE.md). **The guide is where you type and see**: every command, what it prints, and what to do when it differs. **This page never asks you to run anything.** Each section explains one idea, in the order the guide meets it, and names the guide part it prepares: read the section, then do that part. You can also read the whole page first; it takes about an hour. The words are recapped in section 15, and section 16 is a reference to look things up in.

Which section to read before which part of the guide: sections 1 and 2 before Parts 0 and 1; section 3 before Step 0.21 in Part 0; section 4 before Part 2 (and Part 7); sections 5 and 6 before Part 3; sections 7, 8 and 9 before Parts 4 and 5 (they come back in Parts 6 and 9); section 10 before Part 8; section 11 before Part 10; section 12 before Part 11; section 13 before Part 12. Section 14 says how RuHarness itself is tested.

## 1. Programs, files and building

*Prepares the guide's [Part 0](TESTING-GUIDE.md#part-0--get-your-mac-ready) and [Part 1](TESTING-GUIDE.md#part-1--get-liblzg-and-turn-it-into-a-target).*

A **program** is a list of instructions a computer follows, written as text by a programmer: its **source code**. Nobody writes it as one long text. It is split into **functions**: named pieces of code that each do one job, such as "compute a checksum" or "compress this block", and that call each other. The functions live in many **files**: a few in a small program, thousands in a large one. In the C language a `.c` file holds functions, and a **header** (a `.h` file) lists functions and settings that several `.c` files share. One function, **`main()`**, is where the program starts when you run it; a program has exactly one.

A computer cannot run that text as it is. A **compiler** turns each `.c` file into **machine code**, the numbers a processor carries out. Then **linking** joins the pieces: every function one file asks for must be found in exactly one other file, and the result is one program you can run. Turning a whole project into a runnable program this way is called **building** it. A **library** is a collection of ready-made functions that programs use; liblzg, the guide's example, is a library that makes data smaller.

**C** and **Rust** are two programming languages: two ways of writing that text. C is about fifty years old and still runs a large share of the world's software. It trusts the programmer completely, so a C program can read or write memory it should not touch: past the end of a list, or through an address that points nowhere. These **memory mistakes** cause a large share of crashes and security holes. Rust is a newer language whose compiler refuses a whole family of them before the program ever runs.

That is why people move C programs to Rust. Rewriting a whole program at once is risky: it may do things nobody remembers, and a rewrite that differs in one corner breaks someone's work. What makes a gentler way possible is linking: compiled C and compiled Rust can be linked into one program, as long as the Rust offers the same function names, called the same way, as the C it replaces (the guide's Part 4 calls these calling rules the **C ABI**). The rest of the program cannot tell which language a function was written in.

## 2. The big idea: one unit at a time

*Prepares the guide's [Part 1](TESTING-GUIDE.md#part-1--get-liblzg-and-turn-it-into-a-target).*

RuHarness moves a program written in C to Rust one small piece at a time, and proves each piece behaves exactly like the original before it counts. You never have to read or trust the new code yourself: an automatic judge decides. Nothing becomes final until you accept it: a translation the judge passes waits for you, and until you accept it, the program keeps running the C.

The program is split into **units**: one C file, or a small group of files that belong together. The rest of the program stays in C while one unit moves. The new Rust unit keeps the same function names as the C it replaces, so it drops straight into the program:

```text
before   [ lzg.c ]  [ checksum.c ]       [ encode.c ]    all C, linked into one program
after    [ lzg.c ]  [ checksum in Rust ] [ encode.c ]    same function names: the rest cannot tell
```

A unit is named `u-` plus its file's name: `u-checksum` is `checksum.c`. (The built-in example, zopfli, has one unit named by hand, `u001-katajainen`.) The C project being migrated is the **target**: a folder with a short file, `harness.toml`, that says which C files to translate and how to build them. In Part 1 you write one yourself; section 13 shows how the harness can write it for you.

## 3. The harness, the judge and the ledger

*Prepares the guide's [Part 0](TESTING-GUIDE.md#part-0--get-your-mac-ready), from Step 0.21, and [Part 2](TESTING-GUIDE.md#part-2--let-the-harness-read-the-c-and-make-a-plan).*

Three parts do the work:

- **The harness** is the tool that reads the C, works out a safe order for the units, asks an AI model to write the Rust, and keeps records. It never decides whether a translation is right. It includes no model of its own: it asks one, whichever you choose, and judges every answer the same way, so models are interchangeable.
- **The judge** (the harness's files and messages call it **the oracle**) decides. It runs the old C and the new Rust side by side on many of the same inputs and demands identical output, runs the whole program both ways, and checks the Rust does nothing it should not. Every check passes: **GREEN**. Anything else: **RED**. Section 7 names every check.
- **The ledger** is a folder of plain text files inside the target, `migration/`. Every scan, plan, attempt and verdict is written there, so the work can be picked up by anyone at any time. You save it with git like the rest of the project (a **commit** is one saved version; the guide's Part 0 explains git).

```text
targets/zopfli/              the target: the C project
├── harness.toml             which C files to translate, and how to build them
├── src/…                    the C itself: the harness never changes it
└── migration/               the ledger
    ├── facts.jsonl          what the scan found in the C (section 4)
    ├── plan.toml            the units, in the order they can move (section 4)
    ├── observer/            the risky patterns found, and the risk report observations.md
    ├── units/<unit>/        one folder per unit: its driver, its Rust, its attempts, its verdicts
    ├── features/            your features and the feature map (section 10)
    ├── perf/                your workloads and the speed results (section 12)
    ├── map/                 the project map (section 13)
    └── tools/<id>/          each accepted program, with its own ledger (section 13)
```

**Trust, and "made elsewhere".** The judge builds and runs the code a ledger holds: drivers, Rust, features and workloads. A ledger that came with a download or a copy was made on another computer, by someone you may not know. So the first time the harness meets one, it refuses: "this folder already holds migration results made elsewhere … to trust them here, add `--adopt` once". The cockpit asks the same question in words before it opens ("Adopt this folder? Type y and Enter to adopt"). Adopting records the folder as trusted on this computer, deletes the build folders that came with it, and marks each verdict that came with it "made elsewhere" (the unit shows `⚠` in the cockpit) until the judge runs it again here. It is asked once per copy: another copy of the same project is asked again. zopfli comes with one verified unit, so this is the first thing Part 0 meets.

## 4. From C to a plan

*Prepares the guide's [Part 2](TESTING-GUIDE.md#part-2--let-the-harness-read-the-c-and-make-a-plan), and [Part 7](TESTING-GUIDE.md#part-7--why-the-other-three-units-stay-in-c).*

A project goes through six steps, and only two of them involve an AI model. The others are ordinary code that gives the same answer every time, so the facts the AI works from, and the judge that checks it, cannot be talked into anything.

1. **Scan** reads every C file and writes down what it contains (the **facts**): each function, and which file uses which. The harness calls a named function a **symbol**: a **public** one can be called from other files, an **internal** one is private to its own file.
2. **Plan** splits the program into units and orders them so each unit moves after the units it depends on.
3. **Detect** flags C patterns that are known to be tricky to translate, so they are not missed: **macros** (shorthands the compiler expands into code before compiling), **function pointers** (handing a function to another function, to be called back later), and data shared across the whole program. Each flag is a **hazard**.
4. **Observe** asks a model to review each flag, keeping the real ones, and ranks the units by risk in `observations.md`. A flag the model dismisses still counts until a person agrees (`harness review` records that person's word).
5. **Migrate** asks a model to write the Rust for one unit, has the judge check it, and gives the model up to three turns to repair what failed (section 8). Every attempt is kept.
6. **Verify** runs the judge on the unit and records the verdict. You can run it again at any time; if the code has changed since, the unit shows as out of date.

In the cockpit, steps 1–3 are the project's menu items (Scan the project, Refresh the plan, Find hazards (run the detectors)), and step 5 is what the chat asks for.

**Why some units wait.** In this version only a **leaf unit** can be migrated: one whose C calls nothing in the project's other `.c` files. A unit that calls another unit's functions cannot be tested alone, because its test program would not link without the other unit's C. That is what Part 7 shows for liblzg's other three units.

## 5. The driver: a test program for one unit

*Prepares the guide's [Part 3](TESTING-GUIDE.md#part-3--give-u-checksum-a-test-program-driver).*

The judge cannot guess how to call a unit, so each unit has a **driver**: a small C program that calls every function the unit offers, with fixed inputs, and prints every result. The judge links it once with the unit's C and once with the Rust, and compares what the two print.

A driver can be written by a person or by a model (`harness gen-driver`, which asks through the hand-off of section 6). Before it is used, it is tested against the original C: a driver that cannot tell broken C from the real thing is rejected, because it could not catch a broken translation either. What the harness receives is first a **candidate**: kept in the attempt's own folder, never used, until it passes. Then it becomes the unit's driver. The test has seven checks, named on screen as below and always in this order:

| Check | What it asks |
|---|---|
| `driver-build` | Does the driver compile, with strict warnings, and link against the unit's C? |
| `driver-shape` | Does it define only `main` and call only the unit and harmless standard C functions? Does its text pass the rule check (the **lint**)? |
| `symbols-called` | Does it call every public function of the unit? |
| `determinism` | Do three runs print exactly the same bytes? |
| `opt-levels` | Does it print the same with the compiler's speed-ups off (`-O0`) and on (`-O2`)? |
| `sanitizers` | Does it run clean under the **sanitizers**, special builds that stop a program the moment it misuses memory or does something undefined? |
| `mutation` | The harness plants small bugs in copies of the C (**mutants**). Does the driver print something different for enough of them (it must "kill" at least 60 %)? |

**Undefined behaviour** is something the C language does not define, such as reading through an address that points nowhere (a **null pointer**). A program that does it may do anything at all, so a test must never do it: that is what `sanitizers` and `opt-levels` catch. A driver that passes all seven is recorded in the plan, and the unit can be migrated.

## 6. The hand-off: how the harness asks a model

*Prepares the guide's [Part 3](TESTING-GUIDE.md#part-3--give-u-checksum-a-test-program-driver), and later [Part 4](TESTING-GUIDE.md#part-4--translate-u-checksum-to-rust), [Part 6](TESTING-GUIDE.md#part-6--the-second-unit-u-version), [Part 12](TESTING-GUIDE.md#part-12--liblzg-by-map-let-the-harness-find-the-program) and [Plan B](TESTING-GUIDE.md#plan-b-appendix-to-part-4--translate-on-the-command-line).*

The harness never talks to a model directly unless you set that up. By default it uses the **hand-off** (its provider name is `external`): it writes its question into a file and stops, and goes on when an answer file appears beside it.

```text
1. harness gen-driver u-checksum --model …     the harness writes its question
                                                 …/driver-traces/<key>.request.json
                                               and stops, naming the answer it waits for:
                                                 awaiting response: …/<key>.response.json
                                               (its last line starts "error:", exit code 1)
2. someone answers                             the cockpit's chat, Claude Code, or you, writes
                                               <key>.response.json beside the question
3. the same command again                      the harness reads the answer and goes on
```

Stopping with `error:` and exit code 1 is how a hand-off looks; the guide says so before every step where it happens. A migration asks several questions (one per turn, section 8), so it may stop several times: each run answers one question and stops at the next.

**The envelope.** An answer is not plain text. The harness expects the reply wrapped in a small envelope of JSON (a common way of writing data as text): `{"text": …, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}`. The reply goes in `"text"`; the other three fields are bookkeeping a paid service would fill in (how much it read and wrote, and why it stopped), and for an answer written by hand they are `0`, `0` and `"end_turn"`. A bare answer is refused ("the response file must hold the envelope …"). The harness's own waiting message spells the envelope out, and the guide gives a box that writes it for you.

**The question's key.** Each question file is named by its **key**, eight characters such as `aaa26e46`, computed from the question itself. Asking the same question again gives the same key, which is how running the command again finds the answer. A different question (another unit, another turn) gets another key.

**Who answers.** `--model` names who answers, and the name is recorded with every attempt: `guide-written`, `my-claude-code`, or yourself. Left out, the answer is recorded as `claude-sonnet-5`'s, which would be untrue if someone else answered. Name it on the first run: the name is part of the question, so changing it later asks a new question with a new key.

**Providers.** The way the harness reaches a model is its **provider**:

| Provider | How it works | Good for |
|---|---|---|
| The chat (in the cockpit) | Your own Claude Code, signed in with your account, answers the hand-offs a migration asks. | Most people: nothing to set up beyond signing in to Claude Code. |
| Hand-off (`external`, the default) | The file hand-off above; an AI assistant or a person writes the answer file. | Any AI assistant, with no API key. |
| A local model | A model running on your own computer (for example through Ollama). | Free and private; small models usually fail, and the judge catches it. |
| A cloud model | A service such as Anthropic's or an OpenAI-compatible one, with your API key. | Unattended runs. |
| Replay | Re-checks a recorded attempt from its saved answers, calling no model at all. | Proving a past result still holds, at zero cost. |

An **API key** is a secret code that lets a program use an AI service and bills every use to a paid account. The guide never needs one. If you use one, it lives in your own settings file, never in the project, so a project you open cannot send it anywhere.

Whichever you use, the model only ever writes two files for a unit, the translated logic and the thin layer that connects it to the C (the **FFI wrapper**), and everything it writes is built and run inside a **sandbox** before the judge sees it: a locked-down space with no network, no access to your personal files, and a time limit. Work done in a chat or steered by a person is recorded as such, and the benchmark never counts it as the harness's own unassisted result.

## 7. The judge and its checks

*Prepares the guide's [Part 5](TESTING-GUIDE.md#part-5--check-it-yourself-and-read-every-check), and [Part 6](TESTING-GUIDE.md#part-6--the-second-unit-u-version).*

The judge runs a fixed list of checks, and the unit is GREEN only if every one passes. None of them relies on anyone's opinion: not yours, not the model's. Its central idea has two lanes:

```text
              ┌── driver + the unit's C ─────► output A ──┐
same inputs ──┤                                           ├──  A = B, byte for byte?  PASS or FAIL
              └── driver + the unit's Rust ──► output B ──┘

then the whole program, all C and with the Rust inside, on three sample files: same output?
then each of your features, the same way (section 10)
```

**Differential** is the word for this: the same input, two versions, outputs compared. Outputs include what a program prints normally (**stdout**) and its messages and errors (**stderr**). When they differ, the judge says where: "first diff at byte 23" means bytes 0 to 22 were the same. `harness verify` prints one line per check, in the order below; the cockpit's activity line and **Show the checks** say the same checks in words ("Checked: same outputs as C — passed").

| On screen | In the cockpit | What it asks |
|---|---|---|
| `symbol-set` | same exports | Does the compiled Rust offer (**export**) exactly the functions the C unit did, and nothing extra? So it cannot, for example, replace the system's printing function to fake its results. |
| `capabilities` | allowed calls only | Does the Rust use no more of the computer than the C did: no files, network, other programs or clocks unless the C used them too? |
| `driver-shape` | driver shape | Is the driver still only calling the unit and harmless standard functions? |
| `rust-build` | the Rust builds | Does the Rust compile? This line appears only when it does not, and then it is the only check. |
| `differential-driver` | same outputs as C | The two lanes above: does every output of the driver match byte for byte? |
| `whole-program:<sample>` | whole program | The whole real program, built all-C and C-with-the-Rust, run on a sample file: same exit code and output? One line per sample (`sample_text.txt`, `sample_rand.bin`, `sample_empty`); `[SKIP] … not run` when the target has not said how its program is run (section 13). |
| `sanitizers` | sanitizers | Does the C side of the driver run clean under the memory checkers? This proves the test itself never does anything illegal; it checks the C and the driver, not the Rust. |
| `boundary` | boundary calls | For some units only: called with memory sized exactly as the C needs, does the Rust avoid touching anything the C would not? |
| `feature:<feature>/<scenario>` | (the same name) | Each of your feature scenarios (section 10), run on the whole program both ways: same exit code, stdout and stderr? They run after the other checks, and a target without features has none. |

zopfli's verify prints 16 `[PASS]` lines: eight check lines (three of them whole-program lines, one per sample) and eight feature scenarios, then `GREEN`. A GREEN verify ends with exit code 0, a RED one with 10 (section 16 lists them all). When a verified unit fails, the harness lowers its status back to `in-progress` (it is **demoted**).

**What the verdict remembers.** A **verdict** is the judge's recorded result for a unit: every check and its result, plus a **fingerprint** of exactly the code it tested (a code computed from the files' bytes, written `blake3:` and 64 characters; change one byte and it changes). If anything changes afterwards, the C, the driver or the Rust, the unit shows as needing a re-check instead of quietly staying GREEN.

**GREEN is only as good as the tests.** The judge proves the Rust matched the C on the inputs it tried, and nothing more. A weak driver, or a whole-program run that never calls the unit, passes whatever the Rust does. Part 6 shows exactly this: `u-version`'s whole-program runs never call its Rust, so only its driver tests it. That is why the driver is itself tested first (section 5), why your features add tests of their own (section 10), and why accepting stays a human decision.

## 8. A migration, its turns, and your decision

*Prepares the guide's [Part 4](TESTING-GUIDE.md#part-4--translate-u-checksum-to-rust), and [Part 6](TESTING-GUIDE.md#part-6--the-second-unit-u-version).*

An **attempt** is one try at translating a unit, named `a-` plus twelve characters. A unit can have many; every one is kept. Inside an attempt, a **turn** is one question to the model and its answer: one translation turn, then up to three **repair turns**, where the model is told which check failed and tries again. The model writes two files: `logic.rs`, the unit's work in **safe Rust** (Rust the compiler fully checks for memory mistakes), and `ffi.rs`, the FFI wrapper that lets the C call it.

What happens next depends on the verdict:

- **GREEN, not yet accepted.** The attempt waits. Look at the Rust beside the C and at the checks, then **Accept** it (the command line calls this **promote**): the harness copies it into place as the unit's official Rust and runs every check again there. Until you do, the program keeps its C.
- **RED after the repair turns.** Nothing changes in the program: the attempt is kept on record and you choose what to try next. **Modify with a note** starts a new attempt from this one with your written guidance ("keep the counts unsigned like the C"; the records call it a **steer**). **Retry** asks for the same translation again, as a new attempt. **Hand edit** lets you change the Rust yourself in an editor; the judge judges your edit like any other, and it is recorded as a human attempt (`harness override` does the same from the command line).

In the cockpit nothing is ever accepted automatically: not by the model, not by the chat, not by a GREEN verdict. `harness migrate` on the command line accepts a GREEN result by itself, because typing the command asked for it; add `--no-promote` to keep it waiting for `harness promote`, as the guide does. The model's output quality varies: the same request can come back GREEN one time and RED the next. A retry is normal, not a failure of the tool.

## 9. The cockpit and the chat

*Prepares the guide's [Part 4](TESTING-GUIDE.md#part-4--translate-u-checksum-to-rust), and [Part 9](TESTING-GUIDE.md#part-9--tour-the-cockpit-on-the-finished-project).*

The **cockpit** (the program `harness-tui`) is a full-screen view of the project inside Terminal. It does everything most people need from menus, shows the exact command behind every action, and runs the same `harness` commands underneath, so the cockpit and the command line read and write the same ledger and can be mixed.

```text
┌ Files ────────────────┐┌ View ───────────────────────────┐┌ Chat ─────────────────────────┐
│ The project's C files ││ What you selected, in detail:   ││ Ask for model work in plain   │
│ and its units, each   ││ a file's C beside its Rust,     ││ words; it asks the cockpit,   │
│ with a state symbol   ││ a unit's checks, an attempt's   ││ never runs anything itself    │
│                       ││ turns, the project's next step  ││                               │
│ Enter: what you can   ││                                 ││ Asks: Migrate u001-…          │
│ do with the selection ││                                 ││ [Review Enter] [Decline Esc]  │
│                       ││                                 ││ › your message                │
└───────────────────────┘└─────────────────────────────────┘└───────────────────────────────┘
 Activity line: what the running command is doing   [Cancel x] [Details c]
 Key bar: the keys that work right now — click one to press it
```

You work left to right: pick something in **Files**, read about it in the **View**, and ask the **Chat** for model work. Each area is a **pane**; the one your keys go to has the **focus** and a highlighted border, and `Tab` moves it. A row with `▸` is folded and `▾` is open. The symbol beside each row is its state (section 16 lists them). On a window narrower than 156 **columns** (the window's width, counted in letters) the chat shares the right side, and a `View │ Chat` tab on its border switches.

**Every action waits in a dialog.** Select a row and press `Enter`: a short menu lists what you can do with it now (an item that cannot run is greyed, and says why). Every item that changes something opens a dialog that says in words what it will write, then the exact command. It starts on the safe button, must be scrolled to its end, and only after a moment whole on screen says "ready: → then Enter". Keys typed or pasted before that, and a held-down `Enter`, never confirm anything. Only one command runs at a time.

**The chat** is an AI assistant (your own Claude Code) that can look up the project's state (its units, verdicts, attempts and pending questions) and ask the cockpit to do model work for you, but never runs anything itself. The cockpit starts Claude Code when you send your first message, with the line "starting Claude Code (signed in with …)"; it uses your own sign-in, never a key of the cockpit's.

| The chat can ask to… | Only you can… |
|---|---|
| Migrate a unit (a new translation) | Scan the project |
| Modify an attempt with a note | Refresh the plan |
| Retry an attempt | Re-check with the oracle |
| Answer the model's questions during a migration (Continue) | Accept an attempt |

For anything in the right column the chat tells you which menu item to use. A request appears on a yellow line above the input, "Asks: …", and ignores keys for its first second; `Enter` reviews it in the same dialog as your own actions, `Esc` declines it. Everything the chat asked for is recorded as "asked in chat".

**Letting it continue on its own.** A migration is a conversation with the model, and each turn is a hand-off. When you confirm a migration the chat asked for, the dialog says the chat will answer these turns itself; each answer then continues the run after a quiet second ("waits for a quiet moment; Esc holds it"). That permission ends as soon as you hold one, decline or cancel one, stop the chat or start a new one; after that every answer asks you first. It is never given when the cockpit runs without its sandbox, and it never accepts anything.

**What keeps you safe.** Nothing is accepted without you. The chat has no tool that writes a file or runs a command. Model-written code runs in the sandbox, and the dialog says so if the cockpit was started without it (`--allow-unsandboxed`). Every attempt, verdict and hand-off is on record in plain text. Quitting the cockpit stops the chat and any command it started.

**Claude Code in its own window.** A small connector, `harness-mcp`, lets a separate Claude Code session read the project's records, ask for attempts steered from earlier ones, and accept a GREEN attempt (with your permission in Claude Code); that stand-alone route never starts a fresh translation. Inside the cockpit, the chat can ask for a fresh Migrate. The README shows how to set the connector up.

## 10. Features: what a person notices

*Prepares the guide's [Part 8](TESTING-GUIDE.md#part-8--features-check-what-a-person-actually-sees).*

The judge compares each translated piece with the C one on its own. But what a person notices is the whole program: "compress this file to gzip", "show the help", "say the file is missing". Your **features** are those things, in your own words, and RuHarness checks them too.

A feature is made of one or more **scenarios**: one run of the whole program with fixed arguments and, if you like, one of three sample files as its input (a page of English text, some random bytes, or an empty file). The program runs in an empty folder of its own, so an argument is a **flag** (an option starting with `-`) or a word, never a path. You write them in the ledger's `features/features.toml`; the cockpit opens your editor on it and checks the file before saving it.

From then on, every Re-check (and every translation) runs each scenario on the C program and on the program with that piece's Rust swapped in, and compares what a person would see: the exit status and everything printed. These are the `feature:` lines of section 7. A scenario the C itself cannot run the same way twice is skipped and named: it never stops you from working.

**The feature map** records which functions each scenario actually runs. The harness makes it by running every scenario on a scratch copy of the C in which every function notes that it ran. The Features view then shows, for each feature, which units it runs, which of those are already in Rust, and whether its checks passed there; `◉ holds so far` means every check of its Rust against that feature has passed. A unit that none of your features runs is said plainly: its feature checks pass whatever its Rust does, so you may want a scenario that reaches it. A few functions cannot carry a note: they are left **unwatched**, never counted as "not run", and the cockpit shows why beside each. The feature map is a different thing from the project map of section 13, and it is always called the feature map. Each unit is still checked with only its own Rust swapped in. Commit `migration/features/` with your work.

## 11. Status, and picking up later

*Prepares the guide's [Part 10](TESTING-GUIDE.md#part-10--check-the-status-and-resume-later).*

`harness state status` is the "where am I?" command. It checks every record in the ledger against the files and prints one line per unit, such as `u-checksum [verified] plan=fresh verdict=green (fresh)`:

- `[verified]`, `[pending]`, `[in-progress]` is the unit's status in the plan.
- **Fresh** means a record still matches the files it was made from. **Stale** is the opposite: something changed after the record was made, and the line says `STALE: …` and names what. `plan=fresh` means the C has not changed since planning; `verdict=green (fresh)` means the last verdict is GREEN and still matches today's C, driver and Rust.
- `made-elsewhere` marks a verdict that came with an adopted folder (section 3) until the judge runs it here.
- The `attempts` line lists every attempt, how it was answered and how it ended.

The cockpit shows the same: `!` on a file changed since the scan, `⚠` with its cause named in the View, and "out of date" on a result whose inputs changed. Because everything lives in the ledger, picking up another day means opening the same folder: the status line says what is next. Commit the ledger with git as you go, so your work is saved with the project. Only one command may change a project at a time; a second one is refused with the name of the one holding it ("Refused: another command is changing this project").

## 12. Speed

*Prepares the guide's [Part 11](TESTING-GUIDE.md#part-11--speed-is-the-rust-as-fast-as-the-c).*

The judge checks that the Rust does the same thing as the C. **Speed** (the command `harness perf`, so the screens say **perf**) tells you whether it does it as fast. It is never part of a verdict: a slower unit is still a correct one, and you decide whether the difference matters.

You describe a few **workloads**: runs of the whole program the way it is really used, each with your own options and, if you like, one input file of yours inside the project (a real file, at most 64 MiB, about 64 million bytes, committed with the project). perf runs each workload as the original C, then with each verified unit's Rust swapped in on its own, then with every verified unit together ("the program as it stands"). The C and the Rust take turns, many times each (15 by default), on the same computer, and perf says which was faster and by how much, or honestly that it cannot tell.

It measures **CPU time** (how long the processor worked, which varies a little from run to run, so perf repeats each run and says how sure it is) and **instructions** (how many basic steps the processor carried out, which varies much less). The answers read like "about as fast", "slower 6.2 % (4.1–8.3 %)" (the best guess and the range it lies in; perf is at least 95 % sure of it), "can't tell: ±3.4 %" (the runs varied: measure again on a quiet computer) or "too short to time" (use a bigger input; more runs will not help). The cockpit's help lists every answer.

perf also compares what the program prints and how it ends: if the Rust prints or ends differently on one of your workloads, that row says "behaves differently". The judge's checks do not run your workloads, so this is something only perf can find. Numbers are of one computer at one time: commit `migration/perf/` with your work to keep a history, because measuring again replaces a row. Speed runs on macOS for now.

## 13. Mapping a whole C project

*Prepares the guide's [Part 12](TESTING-GUIDE.md#part-12--liblzg-by-map-let-the-harness-find-the-program).*

The example project comes with a `harness.toml` someone wrote by hand. A C project you download has none. RuHarness can make it for you, one program at a time, by mapping the project:

```text
the download: a box of .c files
│
├── lzg.c        holds main() ──► program t-lzg        needs checksum.c, encode.c, …   links
├── unlzg.c      holds main() ──► program t-unlzg      needs a decoder: which one?
└── benchmark.c  holds main() ──► program t-benchmark  needs a decoder: which one?
                                         │
                                         └── held choice d1 (LZG_Decode):
                                               d1.1  src/extra/lzgmini.c   a small decoder
                                               d1.2  src/lib/decode.c      the library's decoder
                                             you pick one; a model may advise, never choose

configuration    migration/map/config.toml: the project's name, what it is built with, its flags
accept t-lzg ──► migration/tools/t-lzg/harness.toml + its own ledger: t-lzg is now a tool
```

- A **program** is a `.c` file with its own `main()`, plus every file it needs: the files that define the functions it calls, and the files those call in turn. The map names it `t-` plus that file's name.
- A **shared file** is one that several programs need, such as a library's `checksum.c`. Each program you accept gets its own copy of the work on it.
- A **held choice** (a "duplicate set", named `d1`, `d2`, …) is a place where two files offer the same functions. A program can link only one. When the map cannot tell which is meant, it holds the choice, lists the files as `d1.1`, `d1.2`, …, and waits for you.
- A **configuration** is how the project is normally built: its name, what it is built with (`make`, `cmake`, …) and the compiler flags that matter: `-I` followed by a folder says where to look for headers, and `-D` sets a definition the C can test. You write it in `migration/map/config.toml` (or ask a model to propose one). Until you do, the map compiles under a guess with no flags, and nothing can be accepted: a guess may hide the very errors that matter.

**Advice, with its reason.** You can ask a model which file to keep in each held choice. Its answer is advice only, shown beside the choice and labelled as the model's, and it gives one of three reasons:

| Reason | Meaning |
|---|---|
| `platform` | The two files serve different platforms or builds. |
| `alternative-implementation` | They are interchangeable versions of the same thing. |
| `cannot-tell` | The facts do not tell. |

**Why "it links" does not prove the right file.** "Linked" means every function the program calls is defined exactly once. It does not mean the program is the one the project ships, that the file kept in a held choice is the right one (both a full and a small decoder link), or that your configuration matches the project's own build. That is why a held choice stays yours even when every option links.

**Tool and target.** When you accept a program it becomes a **tool**: an accepted program, which then plays the target's part. Its `harness.toml` (the acceptance itself: the program's files, the configuration, your picks and the map it came from) and its own ledger live in `migration/tools/<id>/`, exactly as the example keeps its own under `migration/`. From then on every command names it with `--tool t-lzg` where Parts 1–11 named the folder with `--target targets/lzg`. A command without `--target` works on the folder you are in, which is why Part 12 runs its commands from inside the project.

**The whole-program check.** An accepted tool does not know how its program is run, so it has no whole-program check at first: `accept` says so and leaves a commented example in the tool's `harness.toml`, and `verify` shows that check as `[SKIP] … not run` rather than as passed. Adding the program's arguments turns it on, for example:

```toml
[oracle.whole_program]
args = ["-9"]
```

**When the project changes**, the map says which tools changed and how, and the tool's status starts with the same sentence. When only the tool's own C changed, it says to scan the tool; when what the tool needs or how it links changed, it says to accept it again. Accepting again rewrites the parts of the tool's `harness.toml` that the map decides (the files, the include folders, the configuration, the picks, the run name, the libraries it links with) and keeps the rest: its ledger, and what you added yourself, such as an `[oracle.whole_program]` section or a `model`. The closing line says what was kept.

**In the cockpit**, a project folder with no tool yet shows the project's acts instead of the usual tree, each as a dialog that shows the exact command and runs only when you type `y`: **Map the project**, **Ask a model for advice** (for a configuration while it is a guess, then for the held choices), and **Accept a program**, which while the configuration is a guess says "Accept needs a stated configuration …". In an open tool, project files outside it are shown greyed with `⊖`.

**Two maps, two questions.** RuHarness has two maps, and they answer different questions:

```text
the feature map  (Part 8)                        the project map  (Part 12)
migration/features/map.json                      migration/map/project-map.json
which functions each use of the program runs     which files make up each program
made by running the program and noting calls     made by compiling the files and linking them
"compress" runs u-checksum, u-encode, u-lzg      t-lzg needs lzg.c, checksum.c, encode.c, …
```

## 14. How we know it works

RuHarness is measured against a public set of C programs built for exactly this problem: DARPA's TRACTOR test corpus, 100 small C libraries. The harness migrates each one on its own, and the results are then scored on test inputs it never saw during the migration: **held-back tests**, kept aside on purpose, so passing them cannot be a fluke of the judge's own tests.

| Case set | Passed every held-back test | Share |
|---|---|---|
| Public cases | 70 of 77 scorable | 90.9% |
| Released hidden cases | 16 of 17 scorable | 94.1% |

A **blind spot** is a unit the judge passed that still failed a held-back test: exactly what the benchmark exists to find. The first full run found three. One was a real gap in the judge (it did not compare error messages), now fixed. One was undefined behaviour in the test corpus's own C, now excused and disclosed. One was a problem at the edge between the Rust and the C; it led to the `boundary` check, and since 2026-09-24 that unit passes too, so there are no blind spots today.

The benchmark also works as a safety net for changes to RuHarness itself: `harness bench check` re-runs every migration's judgement and score and fails if anything got worse. These scores come from the corpus's public inputs and a different platform, so they are **not comparable** to DARPA's official TRACTOR evaluation.

## 15. Words

Each word in a sentence, with the section that explains it.

| Word | In short | Section |
|---|---|---|
| Accept (promote) | Make a GREEN attempt the unit's official Rust; always yours in the cockpit | 8 |
| Adopt, made elsewhere | Trust a ledger made on another computer, once; its verdicts are marked until judged here | 3 |
| API key | A secret code that bills a paid AI service; never needed by the guide | 6 |
| Attempt | One try at translating a unit, `a-` plus twelve characters; every one is kept | 8 |
| Build, compile, link | Turn source code into a program: compile each file to machine code, then link them | 1 |
| C, Rust | Two programming languages; Rust rules out a family of memory mistakes | 1 |
| C ABI, FFI wrapper | The calling rules C and Rust share; the small Rust file that lets C call the Rust | 1, 6 |
| Candidate | Something the harness received but has not accepted; kept apart until it passes | 5 |
| Check | One test of the judge or of a driver; `PASS` or `FAIL` | 5, 7 |
| Cockpit, pane, focus | `harness-tui`, its areas, and the area your keys go to | 9 |
| Configuration | How a project is built: name, build tool, compiler flags | 13 |
| Driver, mutant | A small C program that calls a unit's functions and prints every result; a copy of the C with a planted bug | 5 |
| Envelope | The JSON wrapper around a hand-off answer | 6 |
| Feature, scenario | What a person does with the program; one run of it | 10 |
| Feature map, project map | Which functions each feature runs; which files make up each program | 10, 13 |
| Fresh, stale, out of date | A record still matches its files; something changed since | 11 |
| Function, file, `main()` | A named piece of code; where it lives; where a program starts | 1 |
| GREEN, RED | Every check passed; at least one failed | 3, 7 |
| Hand-off | A question written to a file, answered by writing a file beside it | 6 |
| Hazard | A C pattern that is risky to translate, flagged for care | 4 |
| Held choice | Two files offering the same functions; you pick one | 13 |
| Judge (oracle) | The part that runs C and Rust side by side and decides GREEN or RED | 3, 7 |
| Key | The eight-character name of a question, computed from it | 6 |
| Ledger | The target's `migration/` folder of plain-text records | 3 |
| Model, provider | The AI that writes; the way the harness reaches it | 3, 6 |
| perf, workload | The speed check; one real run of the whole program | 12 |
| Plan, facts, symbol | The units in a safe order; what the scan records; a named function | 4 |
| Sandbox | A locked-down space where model-written code is built and run | 6 |
| Target, tool | The C project being migrated; an accepted program, which then plays the target's part | 2, 13 |
| Turn, repair turn | One question to the model and its answer; a turn after a failed check | 8 |
| Undefined behaviour, sanitizer | Something C does not define, which a test must never do; a build that stops at it | 5 |
| Unit, leaf unit | One piece of the program that moves to Rust as a whole; one whose C calls no other unit | 2, 4 |
| Verdict, fingerprint | The judge's recorded result: every check, plus a code computed from the exact files it tested | 7 |

## 16. Appendix: reference

**The commands** that `harness --help` lists. Every one takes `--target <folder>` (the target; it defaults to the folder you are in), and on a mapped project `--tool <id>` (the tool to open). `--adopt` trusts a ledger made elsewhere, once (section 3); `--json` prints machine-readable events.

| Command | What it does |
|---|---|
| `harness scan` · `plan` | Reads the C files and records what is in them; groups them into units in a safe order. |
| `harness detect` · `observe` · `review` | Flags risky C patterns; asks a model to review each flag and writes the risk report; records a person's review of a flag. |
| `harness gen-driver <unit>` | Asks a model for the unit's driver and tests it against the C before it may be used. |
| `harness migrate <unit>` | Asks a model for the Rust, runs the judge, gives up to three repair turns, records the attempt; a GREEN one is accepted unless you add `--no-promote`. |
| `harness verify <unit>` | Runs the judge on the unit's current Rust and records the verdict. |
| `harness promote <unit> <attempt>` | Accepts a GREEN attempt: it becomes the unit's Rust. |
| `harness override <unit> <folder>` | Records your own edit of the Rust as a human attempt, judged like any other; never accepted by itself. |
| `harness state status` | Checks every record against the files: what is fresh, what is stale. |
| `harness features init / save / map` | Starts or saves your features file; makes the feature map. |
| `harness project map / accept / ask` | Maps a whole C project; accepts a program as a tool; asks a model for advice. |
| `harness perf init / save / run / show` | Starts or saves your workloads file; measures speed; shows the results. |
| `harness bench …` | The benchmark (section 14). |
| `harness sync-runtime` | Refreshes the summary an AI assistant reads first (the project's `AGENTS.md`). |

Common options of `migrate`: `--model <name>` names who answers; `--provider <name>` chooses the provider (the default is the hand-off); `--no-promote` keeps a GREEN attempt waiting for `promote`; `--retry` records a new attempt when one is already finished; `--steer "<note>" --from <attempt>` starts a new attempt from an earlier one, guided by your note. `--allow-unsandboxed` runs model-written code with no sandbox; use it only on a computer that has none, and only if you accept that.

**Exit codes.** Every command ends with a number that says how it went:

| Code | Meaning |
|---|---|
| `0` | success, or GREEN |
| `1` | the harness refused or stopped with an error; the message says why. It also means "waiting for an answer" at a hand-off (section 6). |
| `2` | the command was typed wrong |
| `10` | the judge said RED |
| `130` | stopped with Ctrl-C; the harness stopped everything it had started |

**The cockpit's keys**, as its help screen (`?`) lists them:

| Key | What it does |
|---|---|
| `↑ ↓` | move in the focused pane (tree: a row; View and details: scroll) |
| `← →` | tree: fold / open, or to the parent / into the View; View: scroll sideways, `←` at the edge back to Files |
| `Enter` | tree: the action menu · menu: choose · dialog: the focused button |
| `Esc` | close a menu, dialog or overlay; View: back to Files; Files: back along your jumps |
| `Backspace` | back along your jumps |
| `Tab` | the other pane (below 80 columns: Files ⇄ View) |
| `PgUp PgDn Home End` | page, or go to the ends |
| `c` | the activity details: the command, every event, its exit |
| `g` | re-read the project |
| `t` | try again, after a refusal |
| `q` | quit (a dialog while a command runs) |
| `m` (in Help) | the mouse on or off |
| `a m e E r R x d v` | shortcuts for the selection's menu items (Accept, Modify, Hand edit, kept edit, Retry, Resume, Cancel, Compare, Show the checks) |
| `j k  ]f [f  J K` | also: move, next/previous pair, next/previous unit |

In the chat, letters are text. `Enter` sends (or, on an empty input, reviews a waiting request); `Ctrl-J`, `\` then `Enter`, or `Alt-Enter` is a new line; `Esc` declines a request, holds a waiting Continue, or stops the reply; `Ctrl-C` stops, clears the draft, or quits (asked); `Ctrl-X` cancels the running command; `Ctrl-N` starts a new chat (asked); `↑↓ PgUp PgDn` scroll; `F1` is the help. With the mouse, a click selects, a double click is `Enter`, the wheel scrolls, and a click on a key in the bottom bar presses it; if clicks print odd characters, start with `--no-mouse`.

**The states**, as the help screen lists them:

| Symbol | Meaning |
|---|---|
| `✗` | failing: the current verdict is red |
| `⚠` | needs attention: a cause you can fix, named in the View |
| `✓` | migrated (steered / by hand when a note or an edit made it) |
| `✓?` | verified, origin not recorded: judged, but no attempt is known to have produced it |
| `◐` | tried: attempts exist, none accepted |
| `◇` | planned: no attempt yet |
| `⊘` | blocked |
| `!` | changed since the scan |
| `+` | not scanned yet |
| `?` | missing: the scan recorded it, the tree no longer has it |
| `·` | a header |
| `–` | no exported functions (never planned) |
| `○` | not in the plan |
| `⊖` | not part of this tool: a project file the open tool does not list |

Beside an attempt row (under its unit), `✓` is GREEN, `✗` is RED, and `◐` is an attempt that has not finished (it stopped at a hand-off). Beside a feature, `◉` holds so far (its migrated units pass), `◌` all its code is still C, `✗` a unit's check of it failed, and `↻` needs a re-check; the help screen lists the rest.

**The menu items** (`Enter` on a row; the name shows what it applies to):

| Menu item | Where | What it does |
|---|---|---|
| Scan the project · Refresh the plan · Find hazards (run the detectors) | The project | Steps 1–3 of section 4. |
| Migrate — ask in chat · Ask in chat… | A unit, or anywhere | Puts a request in the chat, or moves you there to ask. |
| Re-check with the oracle · Show the checks | A unit, or an attempt with a verdict | Runs the judge again; lists every check in words, passed or failed. |
| Accept … into … · Replace …'s verified crate with … | A GREEN attempt | Makes it the unit's Rust (only you can); on a verified unit, swaps it in for the one in use (used with Speed). |
| Compare with the promoted attempt | An attempt | What differs from the Rust already accepted. |
| Modify with a note · Retry · Resume | An attempt | A new attempt with your note; the same again; continue a paused one. |
| Hand edit · Continue my kept hand edit | A unit | Edit the Rust yourself; the judge judges it. |
| Write your features file · Map the features | Features | Section 10. |
| Write your workloads file · Measure speed · Measure this unit's speed · Compare the outputs | Speed, or a verified unit | Section 12. |
| Map the project · Ask a model for advice · Accept a program | A project with no tool yet | Section 13. |
| Cancel the running command · Re-read the project | Anywhere | Stop it (asked first); load the ledger again. |

**When something looks wrong.** The cockpit says what went wrong in words, usually with what to do; `c` shows the exact command and everything it reported, which is what to share when you ask for help. The common cases:

| What you see | What it means |
|---|---|
| "… made elsewhere … add `--adopt` once" | A ledger from another computer: section 3. |
| a command ends `error: awaiting response: …` | A hand-off, not a failure: section 6. |
| "the response file must hold the envelope …" | The answer needs its envelope: section 6. |
| "Refused: another command is changing this project" | Wait for it, then press `t`. |
| `verify` or `migrate` refuses: no sandbox | Not a Mac: see section 9 on `--allow-unsandboxed`. |

For every other line, the guide's [Troubleshooting](TESTING-GUIDE.md#troubleshooting) lists the message, what it means and what to do.
