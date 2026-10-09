# RuHarness

RuHarness moves a program written in **C**, an older programming language, to **Rust**, a newer
one that rules out a whole family of **memory mistakes** (a program reading or writing parts of the
computer's memory it should not, a common cause of crashes and security holes). It moves one piece
at a time, and proves each piece behaves the same as the C it replaces by running both and
comparing what they print. An AI model may write the Rust; it never decides whether the Rust is
right.

- **Who it is for.** Anyone with a C program they want in Rust without reading every line of the
  translation. The testing guide walks a careful non-programmer through it.
- **Where it runs.** A Mac. On Linux it runs only without its safety box (the *sandbox*, below);
  Windows has not been tried.
- **What it touches.** It adds one folder, `migration/`, inside the project, plus one small trust
  file in your Library folder (step 2 below). Your C files are never changed.

## Start here

- **Never programmed?** Read the [tutorial](docs/TUTORIAL.md)'s sections 1–3 first (about 20
  minutes; it asks you to run nothing), then follow the testing guide from
  [Part 0](docs/TESTING-GUIDE.md#part-0--get-your-mac-ready). Part 0 sets up your Mac (30–60
  minutes); the whole guide takes about 4–5 hours in all, setup included. The tutorial's opening
  says which of its sections to read before each later part of the guide.
- **Tools already set up?** Do the [Quick start](#quick-start-the-built-in-example) below.
- **Your own C project?** The section after it, [Your own C project](#your-own-c-project).

## How it works, in eight points

1. The program is cut into **units**: pieces (usually one C file each) that move to Rust as a whole.
2. For one unit, a **model** (an AI such as Claude) writes the Rust; the rest of the program stays C.
3. The Rust keeps the C's **function** names (a function is a named piece of code other files
   call), so it drops into the program in place of the C file.
4. The **judge** (the files call it the **oracle**) runs a **driver**, a small test program that
   calls the unit with fixed inputs and prints every result, once with the C and once with the
   Rust; then the whole program both ways. The outputs must match byte for byte (letter for letter).
5. Every check passes: the **verdict** is **GREEN**. One fails: **RED**.
6. Everything is recorded as plain text in the project's **ledger**, its `migration/` folder.
7. Code a model wrote runs only in a **sandbox**: a locked-down space with no network, no access to
   your personal files, and a time limit.
8. Nothing a model wrote replaces the C until it is GREEN *and* you say yes. The cockpit (the
   full-screen view, below) asks you. On the command line, typing `harness migrate` is your yes: it
   puts a GREEN result in place by itself, unless you add `--no-promote`, which holds it until you
   run `harness promote`.

The six steps, for every unit (the tutorial's [section 4](docs/TUTORIAL.md#4-from-c-to-a-plan)
explains each):

```text
scan ──▶  plan ──▶   driver ──▶ migrate ──▶ verify ──▶ accept
read C    cut into   a test     a model     the judge  you say
          units      program    writes Rust compares   yes
```

## Quick start: the built-in example

An experiment in six steps on **zopfli**, Google's compression program, which comes with RuHarness
in `targets/zopfli`; one unit, `u001-katajainen`, is already in Rust. No AI and no API key (a secret
code that bills an AI service to a paid account) are needed.

**What you need.** The guide's Part 0 done; or Apple's developer tools (`xcode-select --install`),
Rust ([rustup.rs](https://rustup.rs)) and git (a tool that keeps every saved version of a
project's files). Each box is one **command**: paste it into the Terminal app, press Return. If you
did the guide's Part 0, you already have the next box's folder and steps 1 to 3 are Steps 0.19 to
0.22 again: running them again is harmless.

To download RuHarness (you ran this in the guide's Step 0.8; skip it if so). `~` is your home
folder, the one named after your Mac user name; `cd` ("change directory") moves Terminal into a
folder, and every command after it works there:

```bash
git clone https://github.com/kadarius0719/RuHarness.git ~/code/RuHarness && cd ~/code/RuHarness
```

Every command below runs inside that folder. In a new Terminal window, first run
`cd ~/code/RuHarness`.

**1. Build and install the three programs** with cargo, Rust's build tool (you ran this in the
guide's Step 0.19; skip it if so).

```bash
cargo install --locked --path crates/harness-cli
cargo install --locked --path crates/harness-tui
cargo install --locked --path crates/harness-mcp
```

*You should see* many `Compiling` lines, each install ending `` (executable `harness`) ``, then
`harness-tui`, then `harness-mcp`; minutes each the first time. *What it means:* `harness` (the
command-line tool), `harness-tui` (the **cockpit**, a full-screen view with an AI chat) and
`harness-mcp` (the chat's connector) are in `~/.cargo/bin`. `--locked` builds with the exact
versions of the building blocks listed in the project's **lock file** (`Cargo.lock`), the ones it
was tested with. *If not:* a message that the lock file needs updating: run the line again without
`--locked`.

**2. Trust the example's records, once.** `state status` is the "where am I?" command; `--target`
names the project.

```bash
harness state status --target targets/zopfli --adopt
```

*You should see* these two lines first (`<you>` is your Mac user name), then lines starting
`status:`, one per unit, ending with one that tells you to run step 3's command:

```text
adopt: /Users/<you>/code/RuHarness/targets/zopfli is now trusted on this computer (11 units, 1 verified)
adopt: 1 verified unit came with it, marked "made elsewhere" until you run `harness verify <unit> --target targets/zopfli` here (`harness state status --target targets/zopfli` lists them)
```

*What it means:* the judge builds and runs the code a ledger holds, so the first time the harness
meets a ledger made on another computer it asks once; `--adopt` is your yes, kept in
`~/Library/Application Support/ruharness/adopted.toml`. *If not:* `adopt: … is already trusted on
this computer; nothing deleted` is fine (you did this in Part 0); `error: … is not a harness target`
means you are not in `~/code/RuHarness`.

**3. Run the judge on the unit already in Rust**, and print how it ended.

```bash
harness verify u001-katajainen --target targets/zopfli; echo "exit=$?"
```

*You should see*, after 15 seconds or so, 16 `[PASS]` lines (8 checks, then 8 of zopfli's
**features**, runs of the whole program with fixed options), `GREEN` and `exit=0`:

```text
verify: running your 8 feature scenarios after the other checks
verify: [PASS] symbol-set — 1 exported symbol(s) match the unit's symbols exactly
verify: [PASS] capabilities — no capability beyond the C unit's (allowed: none); no asm
verify: [PASS] driver-shape — driver object defines only main, references only the unit and allowlisted libc; source lint clean
verify: [PASS] differential-driver — 183832 bytes identical
verify: [PASS] whole-program:sample_text.txt — 205 bytes identical
… (10 more [PASS] lines: two more sample files, sanitizers, 7 more features)
verify: [PASS] feature:no-file/missing — exit 0; stdout empty; stderr 29 bytes identical
verify: u001-katajainen GREEN — status set to verified
exit=0
```

Words in those lines: a **capability** is something a program asks the computer for (files, the
network, the clock); **asm** is assembly, machine instructions written by hand; **allowlisted
libc** means only harmless standard C functions from a fixed list; a **lint** is an automatic check
of a program's text against rules; **differential** means the C and the Rust ran on the same input
and their outputs were compared.

*What it means:* the Rust offers the same function names, asks for nothing more than the C, and
prints the same bytes as the C (driver, whole program, features). The `sanitizers` line means the
memory checkers ran the C side and the test program and found nothing wrong; they do not run the
Rust. The Rust is checked another way: its working part is safe Rust, which the Rust compiler
itself checks for memory mistakes. `exit=0` is the command's **exit code** ("If something goes
wrong" lists them). *If not:* the "made elsewhere … add `--adopt` once" error means step 2 was
skipped.

**4. Plant a bug and watch the judge catch it.** Find the line to break (`grep` searches a file for
a piece of text; `-n` prints the line's number):

```bash
grep -n 'leaves\[0\]\.count as usize\] =' targets/zopfli/migration/units/u001-katajainen/katajainen_rs/src/lib.rs
```

*You should see* `211:        bitlengths[leaves[0].count as usize] = 1;`. Open the file in the
`nano` editor (`-w` stops it from splitting long lines; on a Mac its title says pico, which is
normal):

```bash
nano -w targets/zopfli/migration/units/u001-katajainen/katajainen_rs/src/lib.rs
```

Press Ctrl-W, then Ctrl-T, type `211`, press Return: the cursor is on that line. Press Ctrl-E (end
of line), then `←` once: the cursor is just after the `1`. Press Backspace, type `2`. Save with
Ctrl-O, then Return; leave with Ctrl-X. Run the `grep` box again: the line now ends `= 2;`. Then
run step 3's box again. *You should see* (some lines left out here):

```text
verify: [FAIL] differential-driver — outputs differ (lens 183832 vs 183832, first diff at byte 88)
verify: [FAIL] whole-program:sample_text.txt — outputs differ (lens 205 vs 206, first diff at byte 10)
…
verify: u001-katajainen RED — status demoted verified -> in-progress
exit=10
```

*What it means:* one changed number makes the Rust print different bytes (`lens` are the lengths of
the two outputs, in bytes; the first difference is at byte 88); the judge says RED (exit code 10)
and the unit no longer counts as migrated. *If not:* still GREEN means the file was not saved: open
it again, Ctrl-O, Return.

**5. Put everything back.** git keeps each saved version (a **commit**); this restores the saved one:

```bash
git checkout targets/zopfli
```

*You should see* `Updated 4 paths from the index`: the file, the plan and two verdict files.
*If not:* another number of paths is fine: it counts the files that had changed.
Run step 3's box once more if you like: it is GREEN again.

**6. See the plan and the risk report.**

```bash
harness plan --target targets/zopfli
```

*You should see* these three lines:

```text
plan: no changes (11 units)
plan: execution order: u-cache -> u-hash -> u-lz77 -> u-util -> u001-katajainen -> u-tree -> u-blocksplitter-deflate-squeeze -> u-gzip_container -> u-zlib_container -> u-zopfli_lib -> u-zopfli_bin
plan: next, write the first unit's differential driver: `harness gen-driver u-cache --target targets/zopfli`
```

*What it means:* the order units can move in, each after those it depends on. The suggested next
step asks a model for a driver (the "differential driver" is the test program of "How it works",
point 4): that is the guide's Part 3, not this experiment. *If not:* `error: … is not a harness target`
means you are not in `~/code/RuHarness`. Then the risk report:

```bash
head -12 targets/zopfli/migration/observer/observations.md
```

*You should see* `# Observations`, `findings: 31` (31 places in the C the detectors flagged as
risky to translate) and a table `Units by risk` starting `| u-blocksplitter-deflate-squeeze |
pending | 51 |`: each unit's risk score, from the risky C patterns (**hazards**) found in it.

**You are done with the experiment.** Next: your own project, just below, or the guide's
[Part 1](docs/TESTING-GUIDE.md#part-1--get-liblzg-and-turn-it-into-a-target), which walks a whole
migration with an AI.

## Your own C project

A project of your own has no settings file for the harness, so the harness first **maps** it: it
finds each program (a `.c` file holding `main()`, where a program starts), the files each needs,
and the **held choices**, where two files offer the same function and you must pick one. You say
how the project is built and **accept** a program as a **tool**: the harness writes its settings
file, `harness.toml`, in `migration/tools/<id>/`, and you name it from then on with `--tool <id>`.
Then the six steps of "How it works" run on the tool as on any project. The ideas are in the
tutorial's [section 13](docs/TUTORIAL.md#13-mapping-a-whole-c-project); the guide's
[Part 12](docs/TESTING-GUIDE.md#part-12--liblzg-by-map-let-the-harness-find-the-program) walks it
whole, every screen included. The screens quoted here are liblzg's, a small compression library
the guide's Part 1 downloads into `~/code/liblzg-upstream`; your names will differ.

**1. Make a copy to work on.** The harness writes only inside the project's `migration/` folder,
but a copy keeps your original untouched. Put your project's folder in place of
`~/code/liblzg-upstream` (dragging a folder from Finder into Terminal types its path), and a new
name that does not exist yet in place of `~/my-copy`.

```bash
cp -R ~/code/liblzg-upstream ~/my-copy
```

*You should see* nothing; the prompt comes back. *What it means:* `cp -R` copies a folder with
everything inside it. *If not:* `No such file or directory` means the first name is wrong (`ls
~/code` lists what is there). The guide's Step 12.1 makes the same kind of copy another way.

**2. Go into the copy.** Every command from here works on the folder you are in, so none of them
names `--target`.

```bash
cd ~/my-copy
```

*You should see* nothing. *What it means:* Terminal now works in `~/my-copy`. In a new Terminal
window, run this box again first. *If not:* `no such file or directory`: step 1 did not make the
copy.

**3. Map it.**

```bash
harness project map
```

*You should see* a long screen. Among its lines: `programs: 3`; under each program, `link check:
linked` (every function it needs found exactly once) or `link check: not linked while d1 is open`
with a line `duplicate set d1 (…): held, …`; and at the end `the configuration is a guess, so
nothing can be accepted yet`, with an example of the next step's file. *What it means:* it compiled
every file in the sandbox under a guess and wrote `migration/map/`; none of your files changed.
*If not:* `error: no C files (.c or .h) were found in …`: you are not in the copy (step 2).

**4. Say how it is built.** The **compiler** turns C text into a program the Mac can run; a
**flag** is an option given to it: `-I` plus a folder says where the **header** files (`.h`) are,
`-D` sets a name the C can test, `-O` a speed level. Find them in the project's `Makefile` (the
file that tells the `make` program how to build) and keep only the `-I`, `-D` and `-O` flags; the
harness refuses the others. Write each folder from the project's top folder: liblzg's
`src/tools/Makefile` says `-I../include`, meaning "one folder up from `src/tools`, then `include`",
which from the top is `src/include`. An `-O` level is recorded only, never used: the harness always
compiles at its own level. The box writes a file of four lines, a heading and three settings:
everything from `cat >` down to `EOF` is one command, so paste it whole. While it pastes, Terminal
starts each line with `heredoc>`; that is normal.

```bash
cat > migration/map/config.toml <<'EOF'
[[configuration]]
name = "make"
from = "make"
flags = ["-O3", "-Isrc/include"]
EOF
```

*You should see* nothing. Read the file back:

```bash
cat migration/map/config.toml
```

*You should see* the four lines between `<<'EOF'` and `EOF`. *What it means:* the harness now knows
how the project is built. (A model can propose this file instead: `harness project ask --build`,
answered as in step 6.)

**5. Map again.**

```bash
harness project map
```

*You should see* the second line now say `configuration: make, from make (stated in config.toml),
flags -O3, -Isrc/include; -O3 is recorded only, never applied (every compile keeps the harness's
own)`, and the last line name `harness project accept <id>` and the held choices (`d1`). *What it
means:* the guess is replaced by your configuration, so programs can be accepted. *If not:* `error:
migration/map/config.toml: …` names each refused flag and why: take those out and redo step 4.

**6. Optional: a model's advice on a held choice.** Skip to step 7 if you like: the advice decides
nothing. `--model` names who will answer (a model, or you), and is recorded with the answer.

```bash
harness project ask --model by-hand; echo "exit=$?"
```

*You should see* it **meant to stop**: `error: awaiting response:
…/migration/map/traces/<key>.response.json` (`<key>` is eight letters and digits naming the
question), then `exit=1`. *What it means:* a **hand-off** (see "Using AI"): the harness wrote its
question to a file and waits for an answer file beside it. The guide's Step 12.4 gives the answer
and runs the same command again, which then shows the advice. *If not:* a different key than the
guide's is normal when your project is not liblzg.

**7. Accept a program.**

```bash
harness project accept t-lzg
```

*You should see* two lines: `project accept t-lzg: the whole-program check is off until you fill in
[oracle.whole_program] in migration/tools/t-lzg/harness.toml (a commented example is there)`, then
`project accept: wrote migration/tools/t-lzg/harness.toml (4 file(s), linked, run as lzg; …)`.
*What it means:* `t-lzg` is now a tool, with its own settings file and ledger in
`migration/tools/t-lzg/`; `cat migration/tools/t-lzg/harness.toml` shows the file. The
whole-program check is turned on later (the end of this section). *If not:* `error: the configuration is a
guess, …`: do steps 4 and 5.

**8. Accept a program with a held choice.** The map lists the files of a held choice as `d1.1`,
`d1.2`, …: for liblzg, `d1.1` is `src/extra/lzgmini.c` (a small decoder) and `d1.2` is
`src/lib/decode.c` (the library's own). Keep the one the project's own build uses: liblzg's
`src/lib/Makefile` builds `decode.c`. Without `--keep`, accept refuses with `error: duplicate set
d1 of t-unlzg (…) is not settled`, naming both files.

```bash
harness project accept t-unlzg --keep d1=d1.2
```

*You should see* ``project accept t-unlzg: keeping `src/lib/decode.c` over `src/extra/lzgmini.c`
…`` and, last, `project accept: wrote migration/tools/t-unlzg/harness.toml (3 file(s), linked, run
as unlzg; …)`. *What it means:* the decompressor is a second tool. This walk goes on with `t-lzg`.

**9. Read the tool's C.** Name the tool each time with `--tool`.

```bash
harness scan --tool t-lzg
```

*You should see* `scan: 6 files, 18 symbols, 42 refs -> …` and a line naming the next command.
*What it means:* the harness recorded every function of the tool's files (a **symbol**) and which
file uses which (the **refs**). *If not:* `error: … has 2 mapped tools … pick one with --tool (t-lzg, t-unlzg)`:
you left out `--tool t-lzg`.

**10. Cut it into units.**

```bash
harness plan --tool t-lzg
```

*You should see* four `plan: unit … added (pending)` lines, then `plan: execution order:
u-checksum -> u-encode -> u-version -> u-lzg` and a line suggesting `harness gen-driver
u-checksum`. *What it means:* four units, in the order they can move.

**11. Ask for the first unit's driver.** The plan suggests `u-checksum`; the guide's Part 12 takes
`u-version`, the smallest unit, and so does this walk. The answer's author is named
`guide-written`, the word the guide uses for this same command, so the guide can pick up from here.
Use the same `--model` word every time you run this command: the word is part of the question, so
another word asks a new question, under a new key, and your answer is not found.

```bash
harness gen-driver u-version --tool t-lzg --model guide-written; echo "exit=$?"
```

*You should see* it **meant to stop**: `error: awaiting response:
…/migration/tools/t-lzg/units/u-version/driver-traces/<key>.response.json`, then `exit=1`. *What it
means:* the harness asked for a driver and waits for the answer. *If not:* `error: … pick one with
--tool …`: you left out `--tool t-lzg`.

**Then the guide takes over.** The box above is the guide's Step 12.7: go on from its
[Step 12.8](docs/TESTING-GUIDE.md#step-128--write-the-driver-into-your-scratch-folder), which
writes the driver's answer (Steps 12.8–12.12), translates the unit (12.13), verifies it and turns on
the whole-program check (12.14) by adding to the tool's `harness.toml`:

```toml
[oracle.whole_program]
args = ["-9"]
```

`args` are the options the program is run with (for liblzg's `lzg`, `-9` means "compress as small
as possible"); until they are there, `verify` shows that check as `[SKIP] … not run`. Run before the
driver is answered, `harness migrate u-version --tool t-lzg` refuses with ``unit `u-version` is
stale: its generated driver's validation is `missing` `` (here "stale" means the unit has no tested
driver yet), and `harness verify u-version --tool t-lzg` refuses with `has no [unit.oracle]
configured` (`[unit.oracle]` is the part of the plan that tells the judge how to test a unit;
answering the driver writes it). Both are expected. When your project changes later, `harness
project map` names each tool that changed and what to do (scan it, or accept it again); the guide's
12C walks that, the cockpit way included.

## Using AI

**The easy way: the cockpit's chat, on your Claude subscription, no API key.** You need Claude Code
(Anthropic's program for using Claude in a terminal) installed and signed in with your
subscription; the guide's Steps 0.13–0.15 check both. Run `harness-tui --target targets/zopfli`,
press `Tab` until the Chat pane is highlighted, and ask in plain words ("migrate this unit"). The
chat never runs anything itself: what it wants waits on a yellow line until you review and confirm
it. The guide's Parts 4 and 6 walk it. *If not:* the cockpit asks `Adopt this folder?` first when
step 2 of Quick start was not done: type `y` and Enter. `q` quits.

**On the command line: a hand-off.** The harness writes its question to a file and stops; you, or
any AI you paste it into, write the answer file, and the same command run again reads it and goes
on. Each try at a translation is an **attempt**; `--model` names who answers, recorded with it.

```bash
harness migrate u001-katajainen --target targets/zopfli --model my-test
```

*You should see* `awaiting response: …/u001-katajainen/traces/<key>.response.json`, a line on the
answer's format, then `error: awaiting response: …` and exit code 1: waiting, not failed. The
answer file holds the reply inside this **envelope** of JSON (a common way of writing data as
text): `{"text": <the reply>, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}`.
The reply goes in `"text"`; the other three fields are bookkeeping a paid service fills in, and an
answer written by hand keeps them as shown. The guide's Plan B (after Part 4) does it with Claude
Code. To drop the waiting attempt instead (you should see `Removing …/attempts/a-<code>/`):

```bash
git clean -fd targets/zopfli/migration/units/u001-katajainen/attempts
```

**Optional: a live service or a local model.** A **provider** is how the harness reaches a model.
`--provider anthropic` uses your `ANTHROPIC_API_KEY` (billed per use). An OpenAI-compatible
service, or a model on your Mac through Ollama, is a profile in your own providers file:
[providers.example.toml](providers.example.toml) shows two, `ollama-anthropic` and
`ollama-openai`. They need Ollama installed and the model `llama3.2-1b-32k` created first, as the
file's comment shows; then run `export RUHARNESS_PROVIDERS=$PWD/providers.example.toml` and add
`--provider ollama-openai --model llama3.2-1b-32k`. A tiny model fails the judge, which is the
point.

## If something goes wrong

`echo "exit=$?"`, run right after a command, prints its exit code:

| Code | Meaning |
|---|---|
| `0` | success, or GREEN |
| `1` | the harness refused or stopped with an error (the message says why); also "waiting for an answer" at a hand-off |
| `2` | the command was typed wrong |
| `10` | the judge said RED |
| `130` | you stopped it with Ctrl-C, and it stopped everything it had started; run it again |

For any other line, the guide's [Troubleshooting](docs/TESTING-GUIDE.md#troubleshooting) says what
to do, and its [Known quirks](docs/TESTING-GUIDE.md#known-quirks-in-this-version) lists the few
out-of-date messages.

---

**Reference: look things up here; a newcomer can stop reading here.** Newcomers: look things up in
the tutorial's friendly [appendix](docs/TUTORIAL.md#16-appendix-reference) instead; the tables
below are written for developers.

## Command reference

Every command is `harness <command>`; `cargo run -p harness-cli -- <command>` is the same program
run from the source. Each takes `--target <folder>` (default: the current folder) and, on a mapped
project, `--tool <id>`; `harness <command> --help` lists every option.

| Command | What it does |
|---|---|
| `scan` | Parses the C (tree-sitter) into `migration/facts.jsonl`: files with include edges, symbols with canonical ids and signatures, call refs. |
| `plan` | Clusters files into units (cycles collapse into one), hashes each unit's include closure (`source_hash`), and reconciles `migration/plan.toml`: statuses, comments and unknown fields survive; order is re-derived from `depends_on`. |
| `gen-driver <unit>` | Asks a model for the unit's differential driver, then validates it against the original C only: strict build, `driver-shape`, every symbol called, three identical runs, `-O0` == `-O2`, ASan/UBSan, mutation adequacy (broken copies of the C must change its output; equivalent mutants discarded). Green: `units/<id>/driver.c` + `driver-validation.json`. |
| `migrate <unit>` | Asks the provider for exactly `src/logic.rs` (safe Rust) and `src/ffi.rs` (the C-ABI shim) in a harness-owned crate whose `lib.rs` confines `unsafe` to the shim; runs the oracle; up to three stateless repair turns. Each attempt is recorded under `units/<id>/attempts/<id>/` (turns, tokens, candidate, `prompt_digest`). Green is promoted (crash-safe two-rename) and re-verified in place. Refuses a unit whose driver is not freshly validated. |
| `verify <unit>` | Refuses if the source changed since planning; else runs the oracle (symbol-set, capabilities, driver-shape, differential driver, whole program all-C vs mixed, sanitizers, features) and writes a content-bound verdict, `oracle-latest.json` (blake3 digests of all it tested). Red demotes `verified → in-progress` and keeps `oracle-last-green.json`. |
| `promote <unit> <attempt>` | Promotes a recorded green attempt and verifies it in place: the explicit act a review's Accept is (`--replace` over a verified unit). |
| `override <unit> <dir>` | Records a hand edit (exactly `src/logic.rs`, `src/ffi.rs` of `dir`) as a labelled `human` attempt, judged like a model reply; never promotes. |
| `state status` | Staleness: facts, plan hashes and verdict digests vs the tree; contradictions; verdicts "made elsewhere". |
| `features init\|save\|map` | Named runs of the whole program, checked on every `verify` (`feature:<name>/<scenario>`); `map` records which functions each runs. Guide Part 8; docs/FEATURES-DESIGN.md. |
| `project map\|accept\|ask` | A whole C project's programs, files and held choices; `accept` writes a tool's `harness.toml`. docs/PROJECT-MAP-DESIGN.md. |
| `perf init\|save\|run\|show` | The C against the Rust on your workloads (CPU time, instructions, memory), information only, in `migration/perf/`. Guide Part 11; docs/PERF-DESIGN.md. |
| `detect` | Deterministic hazard detectors (macros, function pointers, unions/bitfields, setjmp/signals/threads, variadics, mutable globals, allocator ownership) into content-keyed findings; what the suite cannot see (pointer arithmetic, aliasing) stays a standing caveat. |
| `observe` | Model triage of findings per unit (confirm/dismiss/uncertain; nonce-delimited code, content hashes binding each verdict) into `triage.jsonl` and `observations.md`, ranked by a deterministic risk score. |
| `review <finding>` | A person's review: `--uphold-dismiss` or `--reinstate`. A dismissal keeps full risk weight until upheld. |
| `bench …` | The benchmark: `vendor`, `verify-corpus`, `init`, `status`, `score [--write]`, `boundary`, `check [--replay]` (the regression suite, exit 10 on a regression; `--replay` re-judges every recorded trajectory from its evidence, zero tokens; docs/REPLAY-DESIGN.md). targets/tractor/README.md. |
| `sync-runtime` | Regenerates the managed block of the target's `AGENTS.md`; `--check` for CI. |

| Flag | Meaning |
|---|---|
| `--adopt` | Trust a ledger made on another computer, once per checkout: deletes its build folders, writes a fresh token; its verdicts show "made elsewhere" until verified here. |
| `--target DIR`, `--tool ID` | The project; on a mapped project, which accepted tool. With two or more tools and no `--tool`, the command asks you to pick. |
| `--provider NAME` | `external` (file hand-off, the default), `replay`, `anthropic` (`ANTHROPIC_API_KEY`), or a profile from `$RUHARNESS_PROVIDERS`. A target's `harness.toml` can only name a profile; endpoints and keys live in your file. |
| `--model NAME` | The model sent to the provider; with `external`, who answers (recorded). |
| `--retry` | Records a new sample; finished attempts are never overwritten. |
| `--promote`, `--no-promote` | Replace a verified unit's Rust with a new green candidate; or record green without promoting (`[llm.migrate] promote_on_green = false` makes that the only path). |
| `--steer <NOTE> --from <ATTEMPT>` | A new attempt seeded from a finished one (its code, verdict and your note on every turn); a note starting with `-` is attached: `--steer='- keep the loop'`. Never counted as unassisted. |
| `--requester=chat` | Labels the attempt as asked in a chat (hand-offs in `traces/chat/`, never scored); with `--answer=FILE --answer-key=KEY` (`-` reads stdin, framed by `--answer-bytes=N`) files the answer to pending request KEY and resumes, else `answer-refused`. |
| `--attempt ID` | With `--provider replay`: which recorded attempt to re-check. |
| `--allow-unsandboxed` | Where no sandbox exists (Linux): accept running untrusted code unconfined. |
| `--json` | Global: stdout becomes newline-delimited `ruharness-events` (docs/SCHEMAS.md "CLI hardening"); human logs stay on stderr. |

Exit codes as above; for `migrate`, `10` also means blocked, truncated or format. Machine consumers
read the ledger, not stdout. Writing commands hold a writer lock on `migration/.lock`; a second
writer fails fast naming the holder. Ctrl-C kills every live sandboxed process group and the
harness dies by the signal. Target and model code builds and runs under `sandbox-exec` (no network,
no reads of your home folder, writes confined to the build folder, scrubbed environment, timeouts),
and the symbol-set check rejects a candidate exporting more than the unit's symbols or a pre-main
constructor.

## The cockpit (`harness-tui`)

```bash
harness-tui --target targets/zopfli
```

Files and units on the left, the selection on the right, a chat pane; `Enter` opens a menu of what
can be done now. On a ledger not yet adopted it first asks `Adopt this folder? Type y and Enter to
adopt`. Keys, menus and symbols: [docs/TUTORIAL.md](docs/TUTORIAL.md); the walk: guide Parts 4 and 9.
Flags (`harness-tui --help`): `--target DIR` (default `.`), `--tool ID`, `--harness PATH` (default
on PATH, else next to the cockpit), `--provider NAME` (repeatable, default `external`; the target
never chooses it), `--allow-unsandboxed`, `--layout split|stacked`, `--no-mouse`, `--no-chat`,
`--chat-runtime PATH` (the `claude` to run), `--harness-mcp PATH` (default next to the cockpit,
else on PATH), `--chat-model NAME`. `cargo build` at the root skips it (`cargo build -p
harness-tui`); reinstall it with harness-cli, as it uses the `harness` first on your PATH.

The chat's safety rule: the chat is your own Claude Code, reading the project through
`harness-mcp --cockpit`, with no tool that writes. Every act it wants waits until you review it in
the same armed dialog as any act; it is labelled `requester: chat`, never scored (docs/CHAT-PANE-DESIGN.md).

## The ledger in chat (`harness-mcp`)

A stdio MCP server (the Model Context Protocol: how an agent runtime such as Claude Code calls
outside tools). It reads the ledger as data and poses labelled review acts (`harness_status`,
`harness_unit`, `harness_steer`, `harness_request`, `harness_answer`, `harness_retry`,
`harness_promote`), each a spawned `harness --json …` under the writer lock, sandbox and oracle. It
records steer attempts only, never a fresh translation, and refuses every pending blind hand-off.
Protocol and design: [docs/MCP-DESIGN.md](docs/MCP-DESIGN.md). Install it as in Quick start, and
adopt the target first (Quick start step 2, or the cockpit's question): it refuses a ledger made
elsewhere and never adopts. In the project's `.mcp.json`; on a mapped project, add the tool:

```json
{ "mcpServers": { "ruharness": { "command": "harness-mcp", "args": ["--target", "targets/zopfli"] } } }
```

```json
{ "mcpServers": { "ruharness": { "command": "harness-mcp", "args": ["--target", ".", "--tool", "t-lzg"] } } }
```

Flags (`harness-mcp --help`), all yours, none reachable from a tool argument: `--target DIR`,
`--tool ID`, `--target-root DIR` (repeatable; a call may name a target strictly inside one; never
`/` or `$HOME`), `--harness PATH`, `--provider NAME` (repeatable; default `external` only: no
credentials, no spend), `--allow-unsandboxed`; `--cockpit` is the cockpit's mode (reads, and acts
that only ask). **Never serve `targets/tractor/cases` to a chat agent:** the blind protocol answers
hand-offs there and the committed tree holds a pending one. Close the ledger to the runtime's own
file tools (best effort: a shell can still write any file), in `.claude/settings.json`:

```json
{ "permissions": { "deny": ["Edit(/targets/**/migration/**)", "Bash(harness *)", "Bash(cargo run -p harness-cli *)"] } }
```

## Project status and benchmark

| Milestone | Scope | Status |
|---|---|---|
| **M0** end-to-end thread | One leaf unit migrated and differentially verified | ✅ |
| **M1** ledger + schemas | Fact model, plan, content-bound verdicts, multi-unit ordering, workspace, CI | ✅ |
| **M2** observer | Hazard detectors, risk scoring, model triage, human review loop, runtime view | ✅ |
| **M3** executor + provider #2 | `migrate` (translate → oracle → repair), sandboxed execution, two wire adapters live | ✅ |
| **M4** benchmark | TRACTOR B01 library suite (100 cases), driver generation with C-vs-C self-validation, held-out scoring, scores as regression suite | ✅ |
| **M5** extension proof | External detector plugin + `EXTENDING.md` | — |
| **M6+** Phase 2 spike | Second language frontend, golden-test oracle | — |

Since M4: the cockpit and its chat, harness-mcp, features, perf and the project map. zopfli is
vendored at a pinned commit (DECISIONS.md). Benchmark: DARPA TRACTOR public corpus v2, Battery-01
library cases, macOS arm64, `targets/tractor/scores.json`; per-case strict pass (every non-UB
held-out vector passes) over scorable cases.

| Split | Strict pass | Oracle-verified | Blind spots | Non-UB vectors |
|---|---|---|---|---|
| public (80 cases) | **70/77** (90.9%) | 70 | 0 | 908/950 |
| released-hidden (20 cases) | **16/17** (94.1%) | 16 | 0 | 86/87 |

A *blind spot* is a unit the oracle verified that still fails a held-out vector. M4 found three,
diagnosed in DECISIONS.md: an oracle hole (stderr not compared), fixed; the corpus's own undefined
behaviour, excused with disclosure (`unmarked-UB`); an FFI-boundary bug, closed 2026-09-24 when
`read_scalefactors_lib` was re-baselined. Public-vector scores (the vectors predate the models'
training cutoff), **not comparable** to the First TRACTOR Evaluation Report; see
targets/tractor/README.md.

## Repository layout and working on the harness

```text
crates/harness-core/    fact model, schemas, plan, verdicts, observer, risk, planner, adoption
crates/harness-scan/    C frontend (tree-sitter): facts, mutation sites, driver lint
crates/harness-detect/  built-in hazard detectors (c-treesitter-v1)
crates/harness-llm/     provider profiles + adapters, triage, the trajectory engine
crates/harness-oracle/  c-abi-differential oracle: sandbox, gates, validate_driver, scorer
crates/harness-cli/     the `harness` binary
crates/harness-tui/     the cockpit: read model + events reader (a library) and front end
crates/harness-mcp/     the stdio MCP server (reuses harness-tui's library)
devtools/               scripts for testing RuHarness itself: cockpit driver, map spike, guide gates
docs/                   SCHEMAS.md (normative ledger schemas), TESTING-GUIDE.md, TUTORIAL.md,
                        AGENT-BRIEFING.md; designs: TUI, CHAT-PANE, MCP, CLI-HARDENING, FEATURES,
                        FEATURES-PROBE-REDESIGN, PERF, PROJECT-MAP (+ -INVESTIGATION, -ROADMAP),
                        ORACLE-HARDENING, REPLAY, M4, COCKPIT-WRAPPER; FEATURES-PROGRESS.md,
                        NEXT-SESSION.md, NEXT-WEEK-PLAN.md; reviews/
providers.example.toml  example provider profiles (user-level; never read from a target)
targets/tractor/        TRACTOR B01 suite: suite.toml, corpus.lock, cases/, heldout/, scores.json,
                        handoff-tools/
targets/zopfli/         the example: harness.toml, AGENTS.md (generated), migration/ (the ledger)
DECISIONS.md            engineering log: spikes, decisions, milestone handoffs
```

Unit crates under `targets/` are not workspace members: the oracle builds them via
`--manifest-path`, so a broken unit never bricks the harness's own build. Stable Rust (pinned in
`rust-toolchain.toml`) and a C compiler with ASan/UBSan. CI (GitHub Actions, macOS + Ubuntu):
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`
(including an end-to-end migrate-and-verify of u001 in a temp copy), `cargo deny`; `Cargo.lock` is
committed. Every dependency is justified in DECISIONS.md ([docs/AGENT-BRIEFING.md](docs/AGENT-BRIEFING.md)
§11): serde/serde_json, toml/toml_edit, blake3, thiserror, tree-sitter (+C and Rust grammars),
clap, anyhow, signal-hook, libc, ureq (provider HTTP); the cockpit adds ratatui (on crossterm),
similar, tree-sitter-highlight and unicode-width. Working on RuHarness: the briefing first, then
docs/NEXT-SESSION.md and DECISIONS.md's last entries.

## License

Harness crates: MIT OR Apache-2.0. `targets/zopfli/` is Google's zopfli, Apache-2.0 (see its
`COPYING`); `katajainen_rs` is a derivative of it and stays Apache-2.0.
