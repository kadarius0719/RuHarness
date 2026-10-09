# RuHarness

RuHarness moves a program written in **C**, an older programming language, to **Rust**, a newer
one that rules out a whole family of memory mistakes. It moves one piece at a time, and proves each
piece behaves the same as the C it replaces by running both and comparing what they print. An AI
model may write the Rust; it never decides whether the Rust is right.

- **Who it is for.** Anyone with a C program they want in Rust without reading every line of the
  translation. The testing guide walks a careful non-programmer through it.
- **Where it runs.** A Mac. On Linux it runs only without its safety box (the *sandbox*, below);
  Windows has not been tried.
- **What it touches.** It adds one folder, `migration/`, inside the project. Your C files are
  never changed.

## Start here

- **Never programmed?** Follow the testing guide from [Part 0](docs/TESTING-GUIDE.md#part-0--get-your-mac-ready):
  it sets up your Mac (30–60 minutes), then walks a real migration with every screen (4–5 hours).
- **Want the ideas first?** Read [docs/TUTORIAL.md](docs/TUTORIAL.md). It asks you to run nothing.
- **Tools already set up?** Do the [Quick start](#quick-start-the-built-in-example) below.
- **Your own C project?** The section after it, [Your own C project](#your-own-c-project).

## How it works, in ten lines

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
   your home folder, and a time limit.
8. Nothing a model wrote becomes part of the program until it is GREEN *and* you accept it.

```text
scan ──▶  plan ──▶   driver ──▶ migrate ──▶ verify ──▶ accept
read C    cut into   a test     a model     the judge  yours
          units      program    writes Rust compares   to decide
```

## Quick start: the built-in example

An experiment in six steps on **zopfli**, Google's compression program, which comes with RuHarness
in `targets/zopfli`; one unit, `u001-katajainen`, is already in Rust. No AI and no API key (a secret
code that bills an AI service to a paid account) are needed.

**What you need.** The guide's Part 0 done; or Apple's developer tools (`xcode-select --install`),
Rust ([rustup.rs](https://rustup.rs)) and git (a tool that keeps every saved version of a
project's files). Each box is one **command**: paste it into the Terminal app, press Return. To
download RuHarness (the guide's Step 0.8; not run here, like step 1):

```bash
git clone https://github.com/kadarius0719/RuHarness.git ~/code/RuHarness && cd ~/code/RuHarness
```

Every command below runs inside that folder: if you already have it, run `cd ~/code/RuHarness`.

**1. Build and install the three programs** with cargo, Rust's build tool (guide Step 0.19).

```bash
cargo install --locked --path crates/harness-cli
cargo install --locked --path crates/harness-tui
cargo install --locked --path crates/harness-mcp
```

*You should see* many `Compiling` lines, each install ending `` (executable `harness`) ``, then
`harness-tui`, then `harness-mcp`; minutes each the first time. *What it means:* `harness` (the
command-line tool), `harness-tui` (the **cockpit**, a full-screen view with an AI chat) and
`harness-mcp` (the chat's connector) are in `~/.cargo/bin`. *If not:* a message that the lock file
needs updating: run the line again without `--locked`.

**2. Trust the example's records, once.** `state status` is the "where am I?" command; `--target`
names the project.

```bash
harness state status --target targets/zopfli --adopt
```

*You should see* these two lines first (`<you>` is your Mac user name), then a `status:` line per unit:

```text
adopt: /Users/<you>/code/RuHarness/targets/zopfli is now trusted on this computer (11 units, 1 verified)
adopt: 1 verified unit came with it, marked "made elsewhere" until you run `harness verify <unit> --target targets/zopfli` here (`harness state status --target targets/zopfli` lists them)
```

*What it means:* the judge builds and runs the code a ledger holds, so the first time the harness
meets a ledger made on another computer it asks once; `--adopt` is your yes, kept in
`~/Library/Application Support/ruharness/adopted.toml`. *If not:* `adopt: … is already trusted on
this computer; nothing deleted` is fine; `error: … is not a harness target` means you are not in
`~/code/RuHarness`.

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

*What it means:* the Rust offers the same function names, asks for nothing more than the C, prints
the same bytes as the C (driver, whole program, features) and runs clean under memory checkers.
`exit=0` is the command's **exit code** ("If something goes wrong" lists them). *If not:* the
"made elsewhere … add `--adopt` once" error means step 2 was skipped.

**4. Plant a bug and watch the judge catch it.** Find the line to break:

```bash
grep -n 'leaves\[0\]\.count as usize\] =' targets/zopfli/migration/units/u001-katajainen/katajainen_rs/src/lib.rs
```

*You should see* `211:        bitlengths[leaves[0].count as usize] = 1;`. Open the file in the
`nano` editor:

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

*What it means:* one changed number makes the Rust print different bytes; the judge says RED (exit
code 10) and the unit no longer counts as migrated. *If not:* still GREEN means the file was not
saved: open it again, Ctrl-O, Return.

**5. Put everything back.** git keeps each saved version (a **commit**); this restores the saved one:

```bash
git checkout targets/zopfli
```

*You should see* `Updated 4 paths from the index`: the file, the plan and two verdict files.

**6. See the plan and the risk report.** *You should see* the three lines under the box.

```bash
harness plan --target targets/zopfli
```

```text
plan: no changes (11 units)
plan: execution order: u-cache -> u-hash -> u-lz77 -> u-util -> u001-katajainen -> u-tree -> u-blocksplitter-deflate-squeeze -> u-gzip_container -> u-zlib_container -> u-zopfli_lib -> u-zopfli_bin
plan: next, write the first unit's differential driver: `harness gen-driver u-cache --target targets/zopfli`
```

*What it means:* the order units can move in, each after those it depends on. The suggested next
step asks a model for a driver: that is the guide's Part 3, not this experiment. Then run
`head -12 targets/zopfli/migration/observer/observations.md`: *you should see* `# Observations`,
`findings: 31` and a table `Units by risk` starting `| u-blocksplitter-deflate-squeeze | pending |
51 |`: each unit's risk score, from the risky C patterns (**hazards**) found in it.

## Your own C project

Your project has no settings file for the harness, so the harness first **maps** it: it finds each
program (a `.c` file holding `main()`, where a program starts), the files each needs, and the
**held choices**, where two files offer the same function and you must pick one. You say how the
project is built and **accept** a program as a **tool**: the harness writes its settings file,
`harness.toml`, in `migration/tools/<id>/`, and you name it from then on with `--tool <id>`. The
screens quoted are liblzg's, a small compression library; your names will differ. The guide's
[Part 12](docs/TESTING-GUIDE.md#part-12--liblzg-by-map-let-the-harness-find-the-program) walks it
whole: 12A (Steps 12.1–12.5) the map and accept, 12B (Steps 12.6–12.14) the first unit. Work on a
copy of the project, from inside its folder (`cd` there first): without `--target`, a command
works on the folder you are in.

**1. Map it:** `harness project map`. *You should see* a long screen with `programs: 3`; under each
program `link check: linked` (every function found exactly once) or `not linked while d1 is open`
with a `duplicate set d1 (…): held` line; and at the end `the configuration is a guess, so nothing
can be accepted yet`. *What it means:* it compiled every file in the sandbox under a guess and wrote
`migration/map/`; none of your files changed.

**2. Say how it is built.** The **compiler** turns C text into a program the Mac can run; a
**flag** is an option given to it: `-I` plus a folder says where the **header** files (`.h`) are,
`-D` sets a name, `-O` a speed level. Take them from the project's `Makefile` (the file that tells
the `make` program how to build); leave out warning flags such as `-Wall`, which the harness
refuses. Paste the four lines, a heading and three settings, as one command down to `EOF`:

```bash
cat > migration/map/config.toml <<'EOF'
[[configuration]]
name = "make"
from = "make"
flags = ["-O3", "-Isrc/include"]
EOF
```

Read it back with `cat migration/map/config.toml`: *you should see* those four lines. (Or have a
model propose the file: `harness project ask --build`, answered as in step 4.)

**3. Map again:** `harness project map`. *You should see* the second line now `configuration: make,
from make (stated in config.toml), flags -O3, -Isrc/include; …`, and the last line naming
`harness project accept <id>`. *If not:* `error: migration/map/config.toml: …` names each refused flag.

**4. Optional, a model's advice on a held choice:** `harness project ask --model by-hand`
(`--model` names who will answer: a model, or you). It is **meant to stop** with `error: awaiting
response: …/migration/map/traces/<key>.response.json` and exit code 1: a **hand-off** (see "Using
AI"). The guide's Step 12.4 answers it. The advice decides nothing.

**5. Accept a program:** `harness project accept t-lzg`. *You should see* `project accept: wrote
migration/tools/t-lzg/harness.toml (4 file(s), linked, run as lzg; …)`. Read it with
`cat migration/tools/t-lzg/harness.toml` (the message says `git diff`, which shows nothing for a
new file). A program with a held choice is refused, `error: duplicate set d1 of t-unlzg (…) is not
settled`, until you name the file to keep: `harness project accept t-unlzg --keep d1=d1.2`.

**6. Work on the tool**, naming it each time: `harness scan --tool t-lzg` (*you should see*
`scan: 6 files, 18 symbols, 42 refs -> …`), `harness plan --tool t-lzg` (`plan: execution order:
u-checksum -> u-encode -> u-version -> u-lzg`), then `harness gen-driver u-version --tool t-lzg
--model by-hand`, which is **meant to stop** with `error: awaiting response: …` (exit 1) until you
answer it. Until then `harness migrate u-version --tool t-lzg` refuses (``its generated driver's
validation is `missing` ``), and so does `harness verify u-version --tool t-lzg` (`has no
[unit.oracle] configured`). The guide's Steps 12.7–12.14 answer the driver, translate the unit,
verify it, and turn on the whole-program check (`[oracle.whole_program]` with the program's
arguments, such as `args = ["-c"]`, in the tool's `harness.toml`; until then `verify` shows
`[SKIP] … not run`). When the project changes later, `harness project map` names each tool that
changed and what to do (scan it, or accept it again); the guide's 12C walks it, cockpit included.

## Using AI

**The easy way: the cockpit's chat, on your Claude subscription, no API key.** You need Claude Code
(Anthropic's program for using Claude in a terminal) installed and signed in with your
subscription; the guide's Steps 0.13–0.15 check both. Run `harness-tui --target targets/zopfli`,
press `Tab` until the Chat pane is highlighted, and ask in plain words ("migrate this unit"). The
chat never runs anything itself: what it wants waits on a yellow line until you review and confirm
it. The guide's Parts 4 and 6 walk it.

**On the command line: a hand-off.** The harness writes its question to a file and stops; you, or
any AI you paste it into, write the answer file, and the same command run again reads it and goes
on. Each try at a translation is an **attempt**; `--model` names who answers, recorded with it.

```bash
harness migrate u001-katajainen --target targets/zopfli --model my-test
```

*You should see* `awaiting response: …/u001-katajainen/traces/<key>.response.json`, a line on the
answer's format, then `error: awaiting response: …` and exit code 1: waiting, not failed. The
answer file holds the reply inside this **envelope**:
`{"text": <the reply>, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}`. The
guide's Plan B (after Part 4) does it with Claude Code. To drop the waiting attempt instead:
`git clean -fd targets/zopfli/migration/units/u001-katajainen/attempts` (*you should see*
`Removing …/attempts/a-<code>/`).

**Optional: a live service or a local model.** A **provider** is how the harness reaches a model.
`--provider anthropic` uses your `ANTHROPIC_API_KEY` (billed per use). An OpenAI-compatible
service, or a model on your Mac through Ollama, is a profile in your own providers file:
[providers.example.toml](providers.example.toml) shows one; run
`export RUHARNESS_PROVIDERS=$PWD/providers.example.toml` and add `--provider ollama-openai --model
llama3.2-1b-32k`. A tiny model fails the judge, which is the point. (Not run for this README: it
needs Ollama installed and a model downloaded.)

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

**Reference: look things up here; a newcomer can stop reading here.**

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
harness-tui`); reinstall it with harness-cli, as it uses the `harness` installed beside it.

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
