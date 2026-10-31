# The scale run on lz4: what to do, what you should see, what it means

A testing script for the person (2026-10-22). lz4 is the first project bigger than the examples:
a library of five files, a command-line program of seven, about 14 000 lines, with real
dependency chains and a test suite. The goal of the first session is **not** Rust. It is the list
of walls, in order: the first thing that stops you, written down with the piece, the check and the
words on screen. That list decides what gets built next. Expect to stop two or three times.

Every step: what to do, what you should see, what it means, and what to do if you do not see it.
Keep one terminal at a fixed size (160×50 or more). Screenshot every surprise.

## Before you start (about 3 minutes)

**Do.** Pull, rebuild the three programs, copy lz4 into a folder of its own, open the cockpit.
```bash
cd ~/code/RuHarness && git pull -q && cargo build -q -p harness-cli -p harness-tui -p harness-mcp
```
```bash
rsync -a --exclude .git ~/code/ruharness-test-downloads/lz4/ ~/lz4-map/ && cd ~/lz4-map && ~/code/RuHarness/target/debug/harness-tui --target ~/lz4-map
```
**You should see.** The Project view with one row, `Map the project`, and `Next: Map the project — Enter`.
**It means.** The cockpit sees a C project with no record folder yet. Nothing has been read.

## 1. Map (about 1 minute)

**Do.** Enter on the row, then `y`.
**You should see.** `Programs (N)` with `t-lz4cli` and about ten rows from `tests/` and `examples/`
whose card says "looks like a test (it is in a tests/ folder)"; `Libraries` with `lib/`'s files;
`Next: Write the build configuration — Enter, it is the first item`.
**It means.** The map found every `main` and followed what each links. The tests are programs
too; you will not accept them. The configuration is a guess until you write it.
**If not.** A row `✗ did not compile`: press `i` on it and read the first error line; a wall if
it is `lz4cli.c`, fine if it is a test.

## 2. Write the build configuration (about 2 minutes)

**Do.** Enter, choose Write the build configuration, `y`; in the editor make the file read exactly:
```toml
[[configuration]]
name = "make"
from = "make"
flags = ["-O3", "-Ilib", "-DXXH_NAMESPACE=LZ4_"]
```
Save (nano: Ctrl-O, Enter, Ctrl-X), answer `y`. Then Enter, Map the project again, `y`.
**You should see.** `t-lz4cli … links` and `Next: Accept t-lz4cli — Enter on its row`.
**It means.** With lz4's own flags the program compiles and every function it calls is defined
exactly once. The `-D` matters: lz4 renames the hash functions with it.
**If not.** `needs a decision`: two files define the same function; the card lists both; pick the
one under `lib/` unless the card says otherwise. `did not link`: screenshot the card's first error
line; this is a wall worth a session.

## 3. Accept, open, scan, plan (about 2 minutes)

**Do.** Enter on `t-lz4cli`, Accept this program, `y`. Enter again, Open tool. Then Enter on the
tool row: Scan, `y`; Enter: Make the plan, `y`.
**You should see.** `Pieces (N)` with about twelve rows, most `◇ no test program`, one
`· holds main` (lz4cli.c); the journey line `Map ✓ · Configure ✓ · Accept program ✓ · Scan ✓ ·
Plan ✓ · Test program ○ …`; `Next: Make the test program — Enter, then ask in chat` on the first
piece in order (a leaf: expect `lorem`, `timefn`, `util`, `xxhash` or `lz4` first).
**It means.** The plan cut the program into pieces, one per file, in dependency order. A piece
moves after the pieces it calls. Only the piece holding `main` waits for a later version.
**If not.** A piece `⚠ needs attention`: press `i`. Fewer pieces than files: some files have no
functions of their own (headers, or a file folded into another); fine.

## 4. The first piece (10–20 minutes, the first real test)

Take the piece the Next line names. Write its name and its size (the card says how many functions
and lines) on your log sheet.

**Do.** Enter, Make the test program — ask in chat, Enter in the chat to send, then `y` on the
dialog. Watch the chat: it reads the request, writes a C test program, the judge runs it against
the C several times.
**You should see.** After one to three turns: `u-…'s test program is ready — next, its Rust`;
the row `◈ test program ready`.
**It means.** A C program now calls every function of this piece with fixed inputs and prints what
comes back, and it was strong enough to notice planted bugs in the C (the mutation check). That
program is the judge's ruler for the Rust.
**If not.** `red` after four turns with the mutation check named: the test program could not tell
broken C from real C — a wall for pieces whose output depends on the environment or on huge
inputs. Log it, press `i` on the failed try, screenshot, and move to the next piece.

**Do.** Enter, Migrate — ask in chat, send, `y`. Watch the turns.
**You should see.** `GREEN — N of M checks passed`, then `✓? … Rust green — accept it`. Enter,
Accept, `y`. The row `✓ … done`, the Project card and `state status` one piece further.
**It means.** The judge built the program's Rust in place of this piece's C, ran the test program
on both, and the output matched byte for byte; the sanitizers found no memory error; the Rust
exports exactly the C's functions and nothing more.
**If not.** `RED` after four turns: press `i` on the try and read which check failed. Log the
piece, the check and its first line. The likely first walls, in order of probability:
- `symbol-set`: the Rust's exported names differ from the C's. On `xxhash.c` expect exactly this
  if the `-D` rename is not seen by the scan (`LZ4_XXH32` against `XXH32`). A wall for the
  harness, not for you.
- `rust-build`: the Rust does not compile or link. For `lz4.c` (the biggest piece, about 2 800
  lines) the model may run out of room in one answer. A wall of size.
- `capabilities`: the Rust reaches for files, threads or the clock where the C did not. On
  `threadpool.c` and `lz4io.c` the C does, so it is allowed; elsewhere a model's shortcut.
- `differential-driver`: the output differs. Read the first differing byte in the details (`c`).
  This is the judge doing its job; ask the chat to try again (Migrate with a note), once.

## 5. The next pieces (repeat 4, in the order the Next line gives)

**Do.** Each time, take the piece the Next line names. When a piece calls one you already moved,
its card says "It calls …: checked with … as the program stands".
**You should see.** The journey's counts climb: `Test program 3/11 · Rust 2/11 · Accept Rust 2/11`;
`state status` on the command line ends `the program as it stands: N of M pieces in Rust`.
**It means.** Each piece is tested with the Rust of the pieces it calls, so the program really
runs with that much Rust. A piece whose callee you accept later goes `out of date: a piece it
calls changed` — re-check it, that is all.
**Helpers written in headers (from 2026-10-30).** `util.h` writes its helpers in full; it is a
header, not a piece, and after a Refresh of the plan it has no row under Pieces (an older
`u-util` row reads "set aside"). `u-lz4io`, `u-bench` and `u-lz4cli` use those helpers: their
card says `Uses N helper functions written in programs/util.h, translated as part of this
piece`, and the chat is told to translate them as private Rust functions. If a try on one of
them goes red at `symbol-set` naming a `UTIL_` function, or at the build with "undefined
symbol UTIL_…", that is this fix failing: screenshot it.
**A long reply (from 2026-10-30).** A test program for a big piece (`u-lz4`, seventy functions)
can be longer than one reply. The chat pane then says `Claude Code says: the reply hit the
output limit; the chat goes on from where it was cut`, and the chat keeps writing: wait. If
instead it says `the reply was cut off at the output limit three times over`, the turn failed:
Ctrl-N for a new chat, and ask it to make the test program in parts (a file written in several
steps). Before this fix the same moment read `a message the cockpit did not send reached the
chat` — that wording on an output-limit line is the old build.
**Stop rule.** Stop after the third wall, or after `lz4.c` either way (green or not). Everything
past that is the same lesson again.

## 6. Prove the program (10 minutes, once two or more pieces are in Rust)

**Do.** Features row: Propose features — ask in chat, send; then Write your features file, save,
`y`; then Map the features, `y`. Speed row: Propose a workload — ask in chat; put a real file of
a few megabytes into `~/lz4-map/bench/` when the chat asks for one; Write your workloads file,
save; then Measure.
**You should see.** Feature rows `◉ holds so far · k of N pieces`; one or two `a scenario cannot
run` is normal (compression levels that print timings). Speed: `the program as it stands — …`
with a per-piece line, most "can't tell" or "about the same".
**It means.** The whole program, with its Rust inside, still does what a person uses it for, and
runs at about the same speed. That is the "program works" proof this version can give.
**If not.** `does not link` on Speed: screenshot the details; a wall for the harness.

## 7. Write up (5 minutes)

For each wall: the piece, its size, the stage (test program / Rust), the check, the first line of
the detail, and what you expected. Paste the list into the chat with the screenshots. That list is
the next design.

## What this run cannot show

Shared code: lz4's library is used by its tests and examples too, and today each accepted program
has its own record folder, so a shared piece would move once per program. Accept only `t-lz4cli`
in this run. The piece holding `main` stays C. Linux is unchecked.
