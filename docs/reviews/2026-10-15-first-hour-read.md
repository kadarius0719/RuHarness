# The docs reshaped for a person in a hurry — 2026-10-15

The person read the rewritten testing guide and judged it right for an agent and too much for a human. Decided: docs/FIRST-HOUR.md (one page), docs/topics/ (seven short pages), the README cut to its first screen and four doors, the guide and tutorial kept as the reference. A reader in a hurry followed FIRST-HOUR on a fresh copy; their report follows, and its findings were fixed.

---

# First hour, read by a person in a hurry

Who: someone who has never used RuHarness and does not want to read. Opened README.md, took the
first door ("Get it running in an hour"), followed docs/FIRST-HOUR.md on a fresh copy, pasting
each box exactly and looking only for the one thing the line above it names.

How it was run (so the times make sense): Xcode tools, Rust, jq and Homebrew were already on this
Mac, so the three install boxes were checked, not run. RuHarness was copied instead of cloned and
built into a scratch folder; liblzg was copied from the local test copy (already at commit 182b56c)
instead of cloned, so the `git checkout --detach` line could not be seen. `~/lzg-map` and
`~/lzg-practice` were scratch folders. The two editor boxes were driven by typing the page's exact
keystrokes into the real editor (it is UW PICO 5.09 on this Mac; `nano` points to it). No model
was called; every answer was the page's own.

## Run log (clock time, did the screen match, did I stop to think)

README first screen, chose door 1 .............................. 12:14:31  fine, 2 seconds
1.1 xcode-select --install ...................................... not run (installed)
1.2 rustup .............................................. not run (installed)
1.3 brew install jq ............................................. not run (installed)
1.4 git clone RuHarness (copied instead) ........................ 12:14:55  n/a, no stop
1.5 cargo install (44 s) ........................................ 12:15:49  matched `(executable `harness`)`, no stop
1.6 zopfli state status + verify (13 s) ......................... 12:16:33  matched last line; stopped briefly on the 17 lines about "adopt", "trusted" and "made elsewhere" before it
2.1 clone liblzg + checkout (copied instead) .................... 12:16:41  not seen
2.2 rsync + mkdir + cd .......................................... 12:17:02  matched (nothing printed), no stop
2.3 harness project map ......................................... 12:17:03  matched `programs: 3` and `link check: linked`; stopped on "the choice is yours" and "nothing can be accepted yet"
2.4 config.toml heredoc ......................................... 12:17:13  matched, no stop
2.5 map again + accept .......................................... 12:17:15  matched last line; stopped on "the whole-program check is off" just above it
3.1 scan + plan + gen-driver .................................... 12:17:26  matched `error: awaiting response: …`; small pause: the plan says next is `u-checksum`, the page does `u-version`
3.2 driver heredoc .............................................. 12:17:41  matched, no stop
3.3 the long answer line (jq) + gen-driver ...................... 12:17:44  matched `-> GREEN`; box too long to trust pasting
3.4 migrate ..................................................... 12:17:54  matched `error: awaiting response: …`, no stop
3.5 Rust heredoc ................................................ 12:18:07  matched, no stop
3.6 the second long answer line + migrate ....................... 12:18:11  matched `u-version promoted and verified — status set to verified`; box too long to trust
3.7 verify ...................................................... 12:18:20  matched, the one [SKIP] as promised, no stop
4.1 editor edit to `b` + verify ................................. 12:19:11  matched `u-version RED` and `[FAIL] differential-driver`; stopped twice inside the editor (see below)
4.2 editor edit back to `a` + verify ............................ 12:19:28  matched `GREEN — status set to verified`, no stop

Total: about 5 minutes from opening the page to the last GREEN, with every tool already installed
and a 44-second build. On a fresh Mac add the Apple tools download (10 to 30 minutes), Rust (2),
Homebrew (5) and a slower first build (2 to 3): roughly 30 to 45 minutes. Under the hour.

Every one of the 13 boxes I could run showed the exact thing it promised. The "one thing to look
for" line above each box is the best idea on the page: I never had to read output to know whether
to go on.

## Findings

### Stopped me

1. Line 138 (planting the bug). Inside the editor there is nothing to check: after the keystrokes I
   could not tell whether I had changed the right character until the judge ran. Also, after
   Ctrl-W the editor says `Search:`, which looks like the wrong thing. Fix: add "after Ctrl-W it
   says Search:, that is right; line 3 should now read `0x0100_000b`".
2. Line 138 and 142 (the editor name). On this Mac `nano` opens a program whose title bar says
   `UW PICO 5.09`, not nano, so a person checking they are in the right place hesitates. The
   keystrokes do work in it (tested). Fix: one clause, "the editor may call itself PICO; the keys
   are the same".
3. Lines 40 to 144 (which folder am I in). Every box from 2.3 on only works inside `~/lzg-map`, and
   only box 2.2 goes there. Anyone who takes a break, opens a new window, or closes Terminal
   between sections gets confusing errors; worse, in section 4 `nano` on a relative path in the
   wrong folder opens an empty new file, you type into it, save it, and the judge then fails
   for a reason that has nothing to do with the bug. Fix: start the first box of sections 3 and 4
   with `cd ~/lzg-map && `.
4. Line 17 (Homebrew). If `brew` is missing, the page sends you to brew.sh; on an Apple-silicon Mac
   Homebrew's installer ends by asking you to run two more lines to put `brew` on your path, and
   until you do, `brew install jq` still says "command not found". Fix: add "and run the two
   lines it prints under Next steps".

### Slowed me

5. Lines 91 and 129 (the two answer boxes). Each is a single line of about 250 to 330 characters
   with `$( )`, jq and backtick fences; in a normal window it wraps over five lines and I could not
   tell whether my paste was whole. Fix: break each into three short lines (find the request,
   write the answer, ask again) inside the same box.
6. Line 44 (first map). The page names two things to find in about 40 lines, but the output also
   says "the choice is yours", "failed compiles are expected" and "nothing can be accepted yet",
   which read like orders. Fix: add "the rest, including the held choice d1, is about the other
   two programs; you can ignore it".
7. Line 57 (accept). Just above the promised last line it prints "the whole-program check is off
   until you fill in [oracle.whole_program]", which sounds like something I must do now. Fix: say
   here that this line is expected and is the same thing as the [SKIP] at line 131.
8. Line 31 (zopfli proof). Before the verify lines come 17 lines about "adopt", "trusted on this
   computer" and "made elsewhere"; `--adopt` is never explained. Fix: one clause, "`--adopt` tells
   the harness to trust this folder that came with the download; the lines about it are expected".
9. Line 9 (Apple tools). If they are already installed the command prints an `error:` saying so;
   a hurried reader thinks something broke. Fix: "if it says they are already installed, go on".
10. Line 64 (first gen-driver). The plan's own last line says next is `gen-driver u-checksum`;
    the page runs `u-version`. The page does say "the smallest piece", but the clash makes you
    wonder if you skipped something. Fix: "the plan suggests u-checksum; we take the smallest
    one instead, any order is fine".

### Polish

11. Line 66 and on: `--model guide-written` is an odd word that is never explained. Fix: "this is
    only the name your hand-written answers are filed under".
12. Line 138: the RED line reads `outputs differ (lens 284 vs 284, first diff at byte 26)`; "lens"
    is jargon and nothing shows that the difference is the planted `a` to `b`. Fix: say "byte 26 is
    where the version number is printed: the C says 0100000a, your Rust 0100000b".
13. Line 54: the page has you type `-O3` and the next map says "-O3 is recorded only, never
    applied". Fix: drop `-O3`, or add "the harness keeps its own speed setting".
14. Line 59: the first map already showed `link check: linked` for t-lzg, so it is not clear what
    writing config.toml changed. Fix: "without this file, accept refuses: the build was a guess".
15. Line 23: someone who already downloaded RuHarness to read this README gets "destination path
    already exists". Fix: "if you already have it, just `cd` into it".
16. Line 7 heading "(the build about 2 minutes on an M3 Mac, the installs longer)" reads oddly, and
    the Apple tools alone can take half an hour. Fix: "(installs up to 30 minutes; the build about
    2 minutes on an M3 Mac)".
17. Line 147 to 149: the page ends on links with no recap. Fix: three lines saying what just
    happened (the map found the programs; scan and plan cut lzg into pieces; you wrote a small C
    test program the judge checked against the real C; you wrote the Rust; the judge ran both
    and compared every byte, and caught one changed digit), then point at judge.md and drivers.md.
18. Line 129 output: `(tokens in/out: ?/?)` means nothing to this reader. Fix in the harness: leave
    it out when nobody counted.

Could not check here: that the RuHarness GitHub address in line 23 is public, and that line 38
really ends `HEAD is now at 182b56c Bump version to 1.0.10` (the copy was already at that commit,
and with `--detach` git should print no long "detached HEAD" warning).

## Judgement

- About an hour? Yes, less: about 5 minutes with tools already installed, 30 to 45 minutes on a
  fresh Mac, nearly all of it the installs.
- Did I understand what happened? Half. I know the judge ran the C and my Rust, compared what they
  printed, and caught one changed digit; that landed well. I do not know what scan, plan, a
  "driver" or the map really are, or why I wrote a C program before the Rust. The page explains
  almost nothing, by design; finding 17 (a short recap) would close most of the gap.
- Would I read a topic page now? Yes: judge.md first (what GREEN and RED checked, what
  "differential-driver" means), then drivers.md (what that C program I pasted was for).

## The README's four doors, read again

The first door is right and easy to take: bold, first, says "an hour" and lists what you will do.
The other three are less clear for this reader. "Understand one part" (topics), "The full walk and
troubleshooting" (the guide) and "The ideas in full" (the tutorial) are three ways to learn more,
and the README itself calls the last two "the reference", so a newcomer cannot tell the tutorial
from the topics. Missing is the door this person wants next: "move my own C program", which today
hides as `map.md` in the topic list. Suggested four: get it running in an hour; move your own C
program (map.md, then the guide's matching part); understand one part (the seven topics, with the
tutorial named there as the long version); something went wrong (the troubleshooting section and
the exit-code table).
