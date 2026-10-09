# Review of the testing guide for a non-technical reader — 2026-10-09

The person asked: is docs/TESTING-GUIDE.md so easy a non-technical person can follow it, written like a well-designed experiment (a clear goal, what you need, steps that are easy to follow and check)? Two Opus reviewers at high effort read it line by line as that reader, one for Parts 0–5 and one for Parts 6–12 and the closing sections, and ran the commands they could on fresh copies. Their reports follow verbatim; the decisions are at the end.

---

# Review of docs/TESTING-GUIDE.md, top to the end of Part 5 (lines 1–2782)

Read as a careful person with no programming background who can open Terminal and paste.
Guide state: commit fabd9f0. Line numbers are the guide's.

## What I ran, and where today's output differs from the guide

I built the three programs (`cargo build --locked --release`, nothing installed), copied the
repository and the local liblzg copy into the scratchpad, pointed `RUHARNESS_ADOPTED` at a scratch
file, and ran Part 0's checks, Step 0.8, all of Part 1 except the git commands, all of Part 2 and
all of Part 3 there. I could not run git in the copy (this session's sandbox refuses git outside its
own worktree), and Parts 4–5 need Claude.

| Where | Guide says | Real output today |
|---|---|---|
| Step 0.8, line 561 | the status box | **Refused, exit 1:** `error: …/targets/zopfli: this folder already holds migration results made elsewhere (11 units, 1 verified): to trust them here, add `--adopt` once`. The word "adopt" appears nowhere in the guide or the README. With `--adopt` added, two `adopt:` lines come first, the unit line ends in `made-elsewhere`, and a last line says to run `harness verify`. After the verify the status matches the guide's box exactly. |
| Step 0.8 verify, line 589 | 16 PASS, GREEN, exit 0; "one to a few minutes" | identical lines; took 12 seconds; verdict files unchanged afterwards |
| Step 0.4, line 260 | one line `rustup 1.29.0 …` | that line plus two `info:` lines ("This is the version for the rustup toolchain manager…") |
| Steps 0.2, 0.5, 0.6 | example versions | Xcode path (second option), `jq-1.8.1`, `2.1.293 (Claude Code)`: all covered by "something like" |
| Step 0.7 builds, line 489 | "a few minutes" each | all three together: 48 seconds on this Mac |
| Step 1.3, lines 776–800 | each folder on one line | `src/lib` wrapped onto two rows in a narrower window; same names |
| Steps 1.6, 1.8, 2.1, 2.3–2.6 | counts, 808, header bytes, plan | all identical (1592 lines; `20 symbols, 47 refs`; 13 findings; same order and plan block) |
| Step 2.2, line 1458 | ``status: no plan — run `harness plan` `` | ``status: no plan — run `harness plan --target targets/lzg` `` |
| Step 2.5 commit, line 1640 | `3 files changed`, three `create mode` lines | should now be **4**: the first scan also writes `targets/lzg/migration/.gitignore` (seen in the copy; reasoned, not run) |
| Step 1.11, lines 1321–1335 | the lines must be added | today they duplicate that new `migration/.gitignore`; harmless but no longer needed |
| Step 3.1, 3.4, 3.5 | three lines and exit 1; five lines and exit 0; seven PASS; mutation `killed 12/13 compiled (16 sampled of 16 sites, 3 TCE-equivalent discarded; needs ≥ 0.600)`; the oracle table | all identical; the key was 8 characters (`aaa26e46`); validation took 12.5 seconds, not "a minute or two" |
| Step 3.2, line 1819 | sections `[UNIT]`, `[ABI CONTRACT]`, `[C SOURCE]` | correct, but each C file prints as **one huge line** of `\n`-escaped text (all of `lzg.h` in one line), so `head -n 60` fills several screens |

## Whole-guide findings (before Part 0, lines 1–116)

Stops a non-technical reader
1. **Nothing tells the reader how to get RuHarness.** Line 41 assumes it is at `~/code/RuHarness`,
   and Step 0.4 (line 288) and 0.7 (line 426) `cd` into it. There is no `git clone`, no URL and no
   "if you do not have this folder". A newcomer stops at line 288 with `cd: no such file or directory`.
2. **23 words are defined before anything happens** (lines 76–115), and many definitions use
   other unexplained words: "compiled function" (103), "`.c` files", "static" (106), "memory error",
   "undefined" (109), "`Cargo.toml`" (102), "commit it to git" (85). Never explained anywhere in
   Parts 0–5: *compiler*, *header* (`.h`), *build*, *git / commit / branch / repository*, *sandbox*
   (137, 255), *the `~` folder*, *flag*, *library*, *checksum* (first at 667), *Homebrew* (341).
   Better: three words up front (unit, judge, driver), the rest defined where first used.

Slows them
3. Line 7 says "macOS with Apple Silicon" but never how to check it (Apple menu → About This Mac →
   "Chip: Apple M…") or what an Intel Mac would change (Step 0.4's `stable-aarch64-apple-darwin`
   would read `x86_64`).
4. Lines 37–71 teach heredocs, `>` and `<`, exit codes and id formats before the reader has typed
   anything. Keep the two rules (copy a Run box whole; wait for `%`) and move the rest to the first
   step that needs it.
5. No list of steps per part: Parts 0, 1 and 4 (8–11 steps, 60–110 commands each) need one at
   their top. Polish: line 38's `heredoc>` is zsh only (bash, allowed at 131, shows `>`).

## Part 0 — Prepare your Mac (lines 119–656)

Goal: none stated. Natural question: "Is my Mac ready, and does RuHarness work on its own example?"
The checkpoint (651) answers it, and is the best "answer" in the guide, but the last step before
it ends in an optional `git checkout`.

Stops a non-technical reader
1. **Step 0.8's first command fails today** (561): the adoption refusal above. There is no
   "If it looks different" for that box at all (561–584), and Troubleshooting has no row for it.
   Fix: run `harness verify u001-katajainen --target targets/zopfli --adopt` first (or `state status
   … --adopt`), explain in one sentence that the harness asks once before trusting results it did not
   make on this Mac, and that it remembers the answer in
   `~/Library/Application Support/ruharness/adopted.toml`.
2. **No step gets RuHarness** (see whole-guide 1). Step 0.7's Why (415) says "update", not "get".
3. Step 0.6, 361: "follow Anthropic's setup instructions" — no link, no expected screen, and no
   word that a paid Claude subscription is needed before Part 4.
4. Step 0.6, 371–385: editing the shell's start-up file in nano to remove an API key, then
   "ask for help" (385) — help from whom? A non-technical reader should not be editing `~/.zshrc`
   from a description; give the exact `sed` line or say "stop and ask the person who sent you".
5. Step 0.7, 447: "commit them or set them aside" — no command, and a newcomer cannot tell which
   files are theirs.

Slows them
6. Step 0.2 (135–205) is four checks in one step (`xcode-select`, `cc`, `nm`, `sandbox-exec`);
   the `which nm` check has no advice if it differs. Its Why (137) is three tools the reader has
   never heard of; say "the Mac's own build tools" and move names to a footnote.
7. Step 0.3 (227–247): "You should see nothing" sits after the second optional box, so the reader
   who skipped both does not know what the first command should print besides "your name".
8. Step 0.4: the two extra `info:` lines (table above); `source "$HOME/.cargo/env"` (280) is
   unexplained; "overridden by" (300) reads like a warning. 303: "may download the stable toolchain"
   — no idea what that looks like or how long.
9. Step 0.4, 255: Rust must live in `~/.cargo`/`~/.rustup`, but nothing checks it or says what to
   do if it came from Homebrew. 10. Step 0.5, 341: no jq and no Homebrew has no route.
11. **Step 0.7 is nine commands doing three jobs** (update, check the commit, build three programs).
    Split: 0.7 Update, 0.8 Build and install, 0.9 Smoke test. The `merge-base … && echo ok` line
    (478) is opaque; say "this prints `ok` if your copy is new enough".
12. Step 0.7, 495: on a Mac that has never built RuHarness, cargo first downloads the libraries it
    needs (`Downloading`/`Downloaded` lines). Not mentioned; also no "it is finished when `%` comes
    back".
13. Step 0.8: 16 lines of dense text to compare by eye (594–612). Tell the reader what to check:
    the last line says `GREEN` and `exit=0`; optionally a counted check.
14. Step 0.8, 629–635: one "You should see" depends on comparing two versions from earlier steps.
    Make the comparison its own numbered step with both possible outcomes.
15. Step 0.8, 556, 582, 615: trivia (the unit's hand-chosen name, `openai-compat`, `truncated`,
    "older docs say eight") pulls attention from the one question. Polish: drop 129's "Nothing yet."

True only for the author's machine, not said: times "30–60 min" (15), "5–15 minutes" (155), "a few
minutes" per build (489; 48 s total here), "one to a few minutes" (586; 12 s here) — over-estimates
are kind, but label them; `stable-aarch64-apple-darwin` (300) is Apple Silicon only. Step 0.8's need
for rustc 1.94.1 and clang 21.0.0 *is* said (629). Good.

"If it looks different": present and mostly action-giving for 0.1–0.7. Missing for `which nm`,
`rustup show active-toolchain` (no "overridden by" = wrong folder), the sign-in (405), and the
Step 0.8 status box — the one that breaks today.

## Part 1 — Get liblzg and turn it into a target (lines 660–1373)

Goal: "About liblzg" (662) explains why liblzg, not what this part proves. Natural question:
"Do I have liblzg at the right version, does it build and behave the same every time, and is it
set up where the harness can find it?" Step 1.8 answers the middle; the checkpoint (1366) answers
the rest, but its last item (`git status` clean, 1373) is never run in the part.

Stops a non-technical reader
1. Step 1.7's Why (957–964) is compiler flags, `-I<source_dir>`, `[oracle] extra_link_args`,
   `[target] include_dirs`. Mark it "for the curious" or cut to one sentence.
2. Step 1.8 (994–1225) is **19 commands** in one step, with a byte-by-byte hex table (1150–1158).
   Split: 1.8 "Does it run?" (`-V`, no arguments, missing file, empty file), 1.9 "Is its output the
   same every time?" (sample, 808, `cmp`), 1.10 "Where is the checksum?" (`xxd`, optional).
3. 1132: "If you see another number, the copy or the edit in Steps 1.4–1.5 went wrong" — says what
   went wrong, not what to do. Give the action: "delete `targets/lzg` and redo 1.4–1.7".
4. Step 1.11 (1319–1335): seven `.gitignore` lines nobody can understand, and today they are
   redundant (the harness writes its own `migration/.gitignore`). Drop the step's first half.
5. Words without explanation: *LZ77*, *zlib license* (662), *exit code 0* (666), *checksum* (667),
   *branch* (673), *commit id*, *pin*, *archived* (709), *`--detach`* (738), *headers* (765, 809),
   *source_dir* (819), *UTF-8* (909), *`main()`* (911), *include* (877), *optimisation*,
   *floating-point* (959–960), *pangram* (1092), *vendored* (1229), *schema*, *allowlist* (1301–1306).

Slows them
6. Step 1.4 (816–871) uses paths relative to `~/code/RuHarness` but does not say so; a reader who
   opened a new window since Step 1.1 is in their home folder. No "If it looks different" for a
   failed `cp` ("No such file or directory").
7. Step 1.3: "the spacing may differ" (773) — also the line breaks and the order on screen (seen).
   Say "check that these names appear".
8. Step 1.6: two checks whose success is "nothing" (919, 939). Add `&& echo ok`-style endings so
   success prints something.
9. Step 1.8, 1021: "It goes to stderr, but on screen it looks the same" — the reader cannot check
   item 1 of the Why (998). Drop it from the list or say it is checked later by the harness.
10. Step 1.9 (VENDORED.md) has no check that the file exists; Step 1.2's "If gitlab.com fails"
    (726) does not say what failing looks like. Polish: checkpoint 1370 says `lzg -V`, not the
    command that was run, `~/lzg-practice/lzg -V`.

Author's machine: only "20 min" (16); the counts and 808 hold on any Mac (verified). "If it looks
different": good in 1.1, 1.2, 1.5–1.7, 1.9–1.11; missing in 1.3, 1.4 and most of 1.8.

## Part 2 — Let the harness read the C and make a plan (lines 1377–1693)

Goal: none stated. Natural question: "What did the harness find in the C, and which pieces can
move to Rust?" Step 2.6 answers it plainly (`u-checksum` and `u-version`). The closest part to the
experiment shape; the opening "Make sure you are in the RuHarness folder" (1379) is a real
"Before you start" — keep it and name it so.

Stops a non-technical reader
1. Step 2.5's commit (1640) says 3 files; today it should be 4 (the ledger's own `.gitignore`).
   A careful reader will think something is wrong.
2. Step 2.5 invites editing `plan.toml` in nano (1620–1626). The repair advice (1644) is good,
   but it is a second experiment inside a step. Make it a clearly optional side box after the commit.

Slows them
3. Step 2.2's box (1458) differs from today's text (table above).
4. Words: *symbols / refs* (explained, 1410), *signature* (1417), *static*, *public* (1433),
   *callback*, *sort comparator* (1502), *malloc/free* (1504), *fingerprint* (1597, defined in the
   glossary), *interface*, *linked* (1652), *struct* (1681). The Step 2.3 word table (1473) is the
   right pattern: words defined where they are used.
5. Step 2.4 "how the order is chosen" (1554–1560) and Step 2.3 overall are optional; say so up front
   (2.3 does, at 1469).

"If it looks different": present and specific in every step (2.1's three errors are a model).

## Part 3 — Give `u-checksum` a test program (lines 1697–2079)

Goal: "The idea" (1713–1724) is the best goal statement in the guide, but it is a paragraph, not
a question. Natural question: "Is there a test for `u-checksum` that the harness has proven can
catch bugs?" Step 3.5 answers it (seven PASS); the last step is a commit.

Stops a non-technical reader
1. Step 3.1's box (1738–1742) is three lines, one of them ~600 characters, and the step's success
   is an `error:` and `exit=1`. It is explained right after (1756), which is good, but the reader
   meets the word "error" first. Say before running: "this step is meant to stop and say error".
2. Step 3.2, `jq -r .user "$REQ" | head -n 60` (1819): the C files print as single huge lines of
   escaped text; the reader will think the command broke. Say so, or show only the section names.
3. Step 3.3 does four things (write the driver, set `RESP`, build the answer with `jq`, check it).
   The `jq -n --rawfile …` line (1919) is the most fragile paste in Parts 0–5 (nested backticks
   inside quotes). Split into numbered steps; keep the check right after it.
4. Step 3.4's failure path (1983–2004) sends the reader into `attempt.json`, a result table and
   Troubleshooting. There is no "start this part over" recipe (put the ledger back with git and
   remove the unit's new folders), which is what a non-technical reader needs.
5. Shell variables `REQ` / `RESP` (1772–1801) vanish with the window; the warning is there (1801)
   but buried after the first use. Put it in "Before you start": keep one window open for the part.

Slows them
6. Words: *hand-off*, *key* (1730), *shell variable* (1772), `$( … )`, *envelope*, *reply layout*,
   *backticks* (1832), *seed*, *buffer*, *null pointer*, *undefined behaviour*, *16-bit … wrap
   around* (1894–1897), *candidate*, *sites*, *TCE*, *kill rate*, *small-n rule* (2024–2032),
   *oracle table*, *crate*, *c-abi-differential* (2044).
7. "allow a minute or two" (1948): 12.5 seconds here. Give "about 15 seconds on the author's Mac;
   finished when the prompt returns".
8. Step 3.5's mutation explanation (2026–2032) is good teaching but optional; label it.
   Polish: 1724 mentions the cockpit's chat, which the reader has not met yet.

"If it looks different": present for 3.1, 3.2, 3.3, 3.4, 3.5; 3.4's is a diagnosis tree, not an
action. Missing for 3.6 (commit).

## Part 4 — Translate `u-checksum` to Rust (lines 2083–2475; not run: needs Claude)

Goal: none stated. Natural question: "Can Claude write Rust that the judge accepts as the same as
the C, and do I accept it?" Step 4.7 answers it; the part ends with leaving the cockpit (4.8) and
then Plan B, so the answer is not at the end.

Stops a non-technical reader
1. Step 4.1 (2107–2113): three window-width rules in one paragraph (156, 80–155, under 80) and
   "Cmd and -". Give one rule: "make the window as wide as your screen; if `tput cols` is under 156,
   the chat shows as a tab".
2. Step 4.4 (2194–2206): four sub-steps with timing ("wait one second"), and the "You should see"
   placed *before* sub-step 4. Reorder: see the dialog → then press `→` and `Enter`.
3. Step 4.5, 2242: "**Do not press keys now.**" is hidden inside a bullet about chat lines. It must
   come before the run starts, as its own warning.
4. Step 4.5: no "how long is too long" ("a few minutes", 2245). Say what a stuck run looks like and
   what to do after, say, 15 minutes.
5. Plan B (2316–2469) is a second part of ~20 commands, two windows, a long pasted message. Make it
   an appendix with its own goal, before-you-start and checkpoint.

Slows them
6. 4.5 and 4.7 list transient messages ("pass briefly through", 2293) the reader cannot check.
   Name the one line that matters: `Ready. Last: … — GREEN — all 8 checks passed`.
7. Checkpoint 2474: "Accept … ended in `promoted and verified`" — the cockpit never shows that
   phrase; only Plan B's `harness promote` prints it.
8. Words: *pane, focus, fold, ▾/▸, ◇* (2123–2150), *harness-mcp*, *provider external*, *`--json`*
   (2214), *turn / repair turn* (2224), *candidate crate*, *Cargo.lock*, *`*c`* (2293).

Author's machine: the 156-column threshold depends on font size; "15–30 min" (19). "If it looks
different": present in 4.1–4.3 and 4.5–4.7; missing in 4.4 and 4.8.

## Part 5 — Check it yourself (lines 2479–2780; not run: needs Part 4)

Goal: the title is close. Natural question: "Does the judge really catch a wrong translation?"
Steps 5.5–5.6 answer it, and the "read the result like a detective" passage (2693) is the guide at
its best: a prediction (bytes 11–14), a test, an observation. Keep this as the model for others.

Stops a non-technical reader
1. Step 5.5 (2641–2657): the reader must find, in Rust a model wrote, "the line where the Rust sets
   `a` to `1`", jump to it in nano with Ctrl-W Ctrl-T, and edit one character. The wording "depends
   on what the model wrote" (2647). Give a `grep -n` that shows the candidate line(s), say what to do
   if it shows none or several, and keep nano as the fallback.
2. Step 5.5's expected box (2667–2677) holds for one of the two bug choices only; the other is "the
   byte numbers are different". Say plainly which lines must be FAIL and which PASS.

Slows them
3. Step 5.2 (2542–2567) is a reference table with no action and a "Cockpit name" column that means
   nothing until Part 9. Move under "What it means" of 5.1.
4. "This takes a minute or two" (2499): the zopfli verify took 12 s here.
5. Words: *exports* (2557), *asm*, *lint* (2559), *asan+ubsan*, *bound to current source* (2607),
   *demoted* (2699), *shifts `b` left* (2657), *index* (`Updated … from the index`, 2753).

"If it looks different": present in 5.1 (by pointer only), 5.3 (only "two attempts"; add the
`STALE` case), 5.5, 5.6; 5.6's is specific and actionable — a model.

## The template every part should follow

```
## Part N — <short name>

**The question.** One sentence a person can hold in their head, ending in "?".
**You will know the answer when** <the one line or number that answers it>.
**Takes** <time on the author's Mac>. **Uses Claude:** yes/no.

### Before you start
- Where: which folder (one `cd` box) and which branch (one `git switch` box), each with its output.
- What must already be true: the earlier checkpoint that must be ticked.
- What to keep open: e.g. "use one Terminal window for the whole part".
- New words in this part: a small table, only words this part uses for the first time.

### Step N.1 — <verb + object: one thing>
**Run.** one box.
**You should see.** the exact text, or one named line to look for ("the last line ends in GREEN").
  Say when it is finished ("the % prompt comes back, after about 15 seconds on the author's Mac").
**What it means.** one or two sentences, in the words of the question.
**If you do not see that.** a table: what you see → what to do (an action, never only a cause);
  the last row is always "anything else → <the reset recipe for this part> or Troubleshooting".

### Answer
Restate the question and point at the step whose output answers it.

### Checkpoint
Only things the reader actually ran in this part, quoting their exact output.

### If you need to start this part again
One recipe that puts the folder back to the end of the previous part.

### For the curious (optional)
How the harness decides, flags, file formats, history — everything a person can skip.
```

Rules for the steps: one command per step unless two commands only make sense together; every
"You should see nothing" gets a follow-up that prints something; every time, path or version from
the author's Mac is labelled as such; every placeholder `<…>` is listed under the box it appears in.

## Example: Part 3 rewritten in the template

```markdown
## Part 3 — Give `u-checksum` a tested test program

**The question.** Is there a test for `u-checksum` that the harness has proven can catch bugs?
**You will know the answer when** the harness lists seven checks of your test, all PASS (Step 3.9).
**Takes** about 15 minutes. **Uses Claude:** no. You paste a test program this guide provides.

### Before you start

- Part 2's checkpoint is ticked: `u-checksum` has `depends_on = []`.
- Use **one Terminal window** for the whole part. Steps 3.3 and 3.5 store two names, `REQ` and
  `RESP`, that exist only in that window. If you close it, redo Steps 3.3 and 3.5 before going on.
- New words:

| Word | Plain meaning |
|---|---|
| Driver | A small C program that calls `u-checksum`'s one function with fixed inputs and prints every result. The judge runs it once with the original C and once with the Rust, and compares what they print. |
| Hand-off | The harness writes a question into a file and stops until an answer file appears next to it. |
| Mutant | A copy of `checksum.c` with one small bug planted on purpose. A good driver prints something different for most mutants. |

**Run.** Go to the RuHarness folder.

    cd ~/code/RuHarness

**Run.** Make sure you are on the practice branch.

    git switch practice-lzg

**You should see** `Already on 'practice-lzg'` (or `Switched to branch 'practice-lzg'`).

### Step 3.1 — Ask the harness for a driver

This step is **meant to stop with the word `error`**. That is how a hand-off looks.

**Run.**

    harness gen-driver u-checksum --target targets/lzg --model guide-written

**You should see** three lines. The first starts with `awaiting response:`, the second is a long
line starting `gen-driver: external provider mode`, the third is:

    error: awaiting response: /Users/<you>/code/RuHarness/targets/lzg/migration/units/u-checksum/driver-traces/<key>.response.json

`<key>` is 8 characters from 0–9 and a–f; it is different on your Mac.

**What it means.** The harness wrote a question for a model and is waiting for the answer file
named on that line. You will write that answer yourself.

**If you do not see that.**

| You see | Do this |
|---|---|
| ``unit `u-checksum` is stale: …`` | Run `harness scan --target targets/lzg`, then `harness plan --target targets/lzg`, then this step again. |
| `command not found: harness` | Open a new Terminal window, `cd ~/code/RuHarness`, and try again. |

### Step 3.2 — Confirm it stopped on purpose

**Run.**

    echo "exit=$?"

**You should see** `exit=1`. Here 1 means "waiting for an answer", not "broken".

### Step 3.3 — Remember where the question is

**Run.** This stores the newest question file's path under the name `REQ`.

    REQ=$(ls -t targets/lzg/migration/units/u-checksum/driver-traces/*.request.json | head -n 1)

**Run.**

    echo "$REQ"

**You should see** `targets/lzg/migration/units/u-checksum/driver-traces/<key>.request.json`, with
the same `<key>` as in Step 3.1.

**If you do not see that.** An empty line or `no matches found`: run `cd ~/code/RuHarness`, then
the `REQ=` line again.

### Step 3.4 — Read the question (optional, read only)

**Run.**

    jq -r .system "$REQ" | head -n 30

**You should see** text beginning `You write the differential test driver for one C unit for
RuHarness…`, then `DRIVER CONTRACT` and a list of rules (for example `int main(void)` only; never
print a memory address).

**Run.**

    jq -r .user "$REQ" | head -n 60

**You should see** `[UNIT]`, `[ABI CONTRACT]` (naming `_LZG_CalcChecksum`) and `[C SOURCE]`.
Each C file then prints as **one very long line full of `\n`**; it fills several screens. That is
normal: the C is wrapped as quoted data. Scroll back up to see the section names.

### Step 3.5 — Write the driver into your scratch folder

**Run.** One command down to the line `EOF`; copy it whole.

    cat > ~/lzg-practice/checksum-driver.c <<'EOF'
    #include <stdio.h>
    #include <string.h>
    #include "internal.h"

    #define BUF_LEN 70000

    static unsigned char buf[BUF_LEN];
    static unsigned int seed = 12345u;

    static unsigned char next_byte(void)
    {
        seed = seed * 1103515245u + 12345u;
        return (unsigned char)(seed >> 16);
    }

    static void run(const char *label, unsigned int size)
    {
        printf("%s size=%u sum=%08x\n", label, size, _LZG_CalcChecksum(buf, size));
    }

    int main(void)
    {
        static const unsigned int sizes[] = {
            0, 1, 2, 3, 7, 8, 9, 15, 16, 17, 63, 64, 65, 100, 127, 128, 129,
            255, 256, 257, 1000, 4095, 4096, 4097, 65535, 65536, 65537, 70000
        };
        size_t i;
        size_t n = sizeof sizes / sizeof sizes[0];

        for (i = 0; i < BUF_LEN; i++) buf[i] = next_byte();
        for (i = 0; i < n; i++) run("rand", sizes[i]);

        memset(buf, 0xff, BUF_LEN);
        for (i = 0; i < n; i++) run("ff", sizes[i]);

        memset(buf, 0, BUF_LEN);
        for (i = 0; i < n; i++) run("zero", sizes[i]);

        for (i = 0; i < BUF_LEN; i++) buf[i] = (unsigned char)(i & 0xff);
        for (i = 0; i < n; i++) run("ramp", sizes[i]);

        return 0;
    }
    EOF

**You should see** `heredoc>` at the start of each line while it pastes, then the `%` prompt.
**If you do not see that.** Stuck at `heredoc>`: type `EOF`, press Return, paste the box again.

### Step 3.6 — Name the answer file

**Run.**

    RESP="${REQ%.request.json}.response.json"

**Run.**

    echo "$RESP"

**You should see** the Step 3.3 path with `.request.json` replaced by `.response.json`.

### Step 3.7 — Put the driver into the answer file

**Run.** This wraps the driver in the layout the harness expects. Copy it whole.

    jq -n --rawfile d ~/lzg-practice/checksum-driver.c '{text: ("driver.c\n```c\n" + $d + "```\nRUHARNESS_END_OF_OUTPUT\n"), input_tokens: 0, output_tokens: 0, stop_reason: "end_turn"}' > "$RESP"

**Run.** Check the start of the answer.

    jq -r .text "$RESP" | head -n 3

**You should see** exactly three lines: `driver.c`, then three backticks and `c`, then
`#include <stdio.h>`.
**If you do not see that.** The `jq` line was cut while pasting. Paste it again; it overwrites.

### Step 3.8 — Run the same command again, so the harness reads your answer

**Run.** Exactly the Step 3.1 command, including `--model guide-written`.

    harness gen-driver u-checksum --target targets/lzg --model guide-written

**You should see**, after about 15 seconds on the author's Mac:

    gen-driver: checking the driver against the original C (it is built and run several times; this can take a minute) …
    gen-driver: turn 1 generate -> green
    gen-driver: u-checksum attempt d-<12hex> via `external` (external) model `guide-written` -> GREEN
    gen-driver: checking it once more where it now lives …
    gen-driver: promoted migration/units/u-checksum/driver.c and recorded /Users/<you>/code/RuHarness/targets/lzg/migration/units/u-checksum/driver-validation.json

**Run.**

    echo "exit=$?"

**You should see** `exit=0`.

**What it means.** The harness accepted your driver as `u-checksum`'s test and wrote down how it
was checked.

**If you do not see that.**

| You see | Do this |
|---|---|
| a new `awaiting response:` line | Do not answer it yet. Run `jq -r '.turns[] \| "\(.kind) -> \(.result)"' targets/lzg/migration/units/u-checksum/driver-attempts/d-*/attempt.json`. If it says `generate -> format`, redo Steps 3.3, 3.6, 3.7 and this step. For anything else, see "List why a driver failed" in Troubleshooting. |
| `-> RED` and `exit=10` | See "gen-driver ends RED" in Troubleshooting. |

### Step 3.9 — Read the answer to the question

**Run.**

    jq -r '.checks[] | "\(.name): \(if .passed then "PASS" else "FAIL" end) - \(.detail)"' targets/lzg/migration/units/u-checksum/driver-validation.json

**You should see** seven lines, each with `PASS`, in this order: `driver-build`, `driver-shape`,
`symbols-called`, `determinism`, `opt-levels`, `sanitizers`, `mutation`. On the author's Mac the
last one read exactly:

    mutation: PASS - killed 12/13 compiled (16 sampled of 16 sites, 3 TCE-equivalent discarded; needs ≥ 0.600)

Your numbers may differ with another compiler; what matters is `PASS`.

**What it means.** This answers the question: yes. The driver builds, follows the rules, calls
the function, prints the same thing every time and at every optimisation level, runs clean under
the memory checkers, and noticed 12 of the 13 planted bugs (it needed 60%).

**If you do not see that.** A `FAIL` cannot follow a GREEN in Step 3.8: you are reading an old
file. Run Step 3.8 again.

### Step 3.10 — See what the harness added to the plan

**Run.**

    grep -A 4 '^\[unit.oracle\]' targets/lzg/migration/plan.toml

**You should see** exactly:

    [unit.oracle]
    kind = "c-abi-differential"
    driver = "migration/units/u-checksum/driver.c"
    rust_crate = "u_checksum_rs"
    replaces = ["src/lzg/checksum.c"]

**What it means.** The judge will test `u-checksum` by running this driver against the C and
against the Rust, which will live in a crate named `u_checksum_rs`.

### Step 3.11 — Save your work

**Run.**

    git add targets/lzg

**Run.**

    git commit -m "lzg: validated driver for u-checksum"

**You should see** `[practice-lzg <hash>] lzg: validated driver for u-checksum` and
`8 files changed, <number> insertions(+)`.

### Answer

Is there a proven test for `u-checksum`? Yes: Step 3.9 printed seven PASS lines, including the
mutation check.

### Checkpoint

- [ ] Step 3.2 printed `exit=1` and Step 3.8 printed `-> GREEN` and `exit=0`.
- [ ] Step 3.9 printed seven `PASS` lines.
- [ ] Step 3.10 printed the five `[unit.oracle]` lines.

### For the curious

- Why these 28 sizes (around 8, 64, 128, 256, 4096 and 65535–65537), why no null pointer, why
  `"internal.h"` is spelled that way: today's lines 1892–1898.
- How to read the mutation line (sites, TCE-equivalent, the one expected survivor `size / 9`, the
  small-n rule): today's lines 2024–2032.
- Why the key stays the same when you re-run (1762), and why a different `--model` gets a new one
  (1730).
```


---

# TESTING-GUIDE.md, Parts 6–12 and the closing sections, read as a non-technical person

Read: lines 1–118 (the promises), then 2784–4871 line by line. Judged against the shape the owner wants: a question
per part, what you need first, one-action steps, an exact "you should see", what it means, and what to do if not. Line
numbers are those of the file at `fabd9f0`. The big picture: Parts 6, 8, 10 and 11 mostly keep the experiment shape.
Part 7 is fine. Part 9 is a tour, not an experiment. Part 12 is where a newcomer stops: 850 lines, about twenty new
ideas with five defined, "Parts 2–11 are not needed" and then leans on them, and its expected errors are not called
normal.

The opening (lines 1–118) never defines these words that Parts 6–12 lean on constantly: **compile / compiler, build,
link / linking, header, flag, function, `main()`, library, sandbox, branch, commit, hash/digest, Makefile**. Part 1
explains `-I` in passing (line 964); nothing else does. Every part below inherits this gap; I list it once here, not
in each part.

## Part 6 — the second unit (lines 2784–3011)

Goal: line 2805 says it "repeats Parts 3–5 quickly, and teaches something about what the whole-program check can
miss". That is a summary, not a question. Step 6.3's lesson (2984–2988) does answer it well. A question to hold: "Does
checking the whole program really test my new Rust?" (answer: no, not for `u-version` — and Part 8 fixes it).

**Stops a non-technical reader**
1. 2817: the first `gen-driver` ends with `error: awaiting response` and exit 1, and the step only says "as in Step
   3.1". It never says again "this error is normal: it is waiting for your answer". Someone returning after a break
   reads `error:` and stops.
2. 2896–2904: "You should see seven PASS lines. On the guide's run the mutation line was the first of these two" —
   then two boxes. The reader cannot tell whether either is fine or which they should get, and only one of the seven
   lines is shown. Say: "seven lines ending PASS; the `mutation` line will be one of these two, and both are correct".
3. 2949–2953: "Run. Accept it:" has no command box (the opening, line 37, says Run always means a box to paste). Item
   2 asks the reader to judge Rust ("a function that returns a pointer to a fixed text ending in a zero byte") —
   impossible for this reader. Item 3 has no "you should see", yet the checkpoint (3009) asks whether Accept "said
   `promoted`".

**Slows them**
4. 2846–2869: `REQ`, `RESP` and the `jq` line are not re-explained ("As in Part 3"), and the check Step 3.3 had (`jq
   -r .text "$RESP" | head -n 3`) is missing here, so a bad paste is only found one command later.
5. 2899, 2906: "TCE-equivalent", "sites", "mutation gate", "machine code" are unexplained. The plain reason is in 2906
   but arrives after the confusing line.
6. 2945: "Do not press keys while the 'Continues …' line is waiting" — for how long? Part 4's table says 15–30
   minutes; say it here so the reader knows the screen is not frozen.
7. 2958: "use Plan B, with `u-version` in place of `u-checksum` everywhere" — Plan B is ~150 lines with paths, crate
   names and file names; asking a newcomer to substitute by hand invites a wrong path. Either give the three or four
   lines that change, or a short list.

**Polish**
8. 2800–2803: `0x0100000a` and "returns" need one plain sentence; 2986 squeezes a second lesson (only one unit's Rust
   at a time) into the first.

## Part 7 — why three units stay in C (lines 3014–3048)

Goal is clear and answered (3047: two of five is the expected finish).

**Slows them**
1. 3026–3029: "links", "link would fail", "function pointers", "sort comparator", "progress callback", `depends_on`,
   `_LZG_CalcChecksum`. The table at 3018 is enough for this reader; the bullets should say it in plain words ("the
   judge builds each piece alone, and these pieces cannot be built without another piece").
2. 3036–3043: the optional refusal is expected, but there is no `echo "exit=$?"` and the message starts "error:
   invalid plan", which sounds like the reader broke the plan. One line: "this error is what you want to see; nothing
   changed".

## Part 8 — features (lines 3051–3358)

Goal: "The idea" (3067) is good and the map in 8.4 answers it. The question is implicit; make it explicit: "Which of
the program's real uses actually run my Rust?"

**Stops a non-technical reader**
1. 3168 onward: Part 8 calls its result "the map" (`features map`, `map.json`); Part 12 calls a different thing "the
   map" (`project map`). A newcomer will merge them. Rename one in the prose ("the feature map" / "the project map")
   every time.
2. 3198: "run the same command by hand twice, in an empty folder, and compare the outputs (as in Step 1.8)" — which
   command? The program is in `~/lzg-practice/lzg`, not on the PATH, and "compare" is not a command. Give the two or
   three exact lines.

**Slows them**
3. 3129–3131: the rules table uses "flag", "path", "KiB", "pangram", `{input}` without explanation. A newcomer needs
   only: "copy the box; the table is for when you write your own".
4. 3179: the output line says `source_dir` — never explained in this part.
5. 3193: "compiled it one `.c` file at a time: a note the compiler rejects is taken out of just that function" is too
   dense; "round 1" in the output is unexplained.
6. 3162: "Edit it directly with `nano -w …`" — nano's keys are explained in Part 0 and Part 10, not here.
7. 3336: "You should see `features=current` at the end of both verified units' lines" — no box; give the two lines as
   in 8.5.

**Polish**
8. 3209, 3225: two long `jq` lines with no "this only reads; it changes nothing".

## Part 9 — cockpit tour (lines 3361–3415)

Goal: none as a question; it is a tour. Suggest "Does the cockpit show the same truth as the command line?" and end
with item 6 (the re-check) as the answer.

**Stops a non-technical reader**
1. 3387–3405: eleven items, most doing two to four things (item 2: open, open, select, press →, read a header; item 6:
   Enter, choose, read, wait, →, Enter, read). With no "you should see" box per item, a reader who gets lost cannot
   find where. Split into numbered steps with one key each, or at least one expected line per item.
2. 3407: "so `git status --short` still prints nothing" — the reader was never told to run it. Make it a Run step.

**Slows them**
3. 3388–3403: symbols `✓2/5`, `◇`, `◉`, `◌`, "View", "Files", "a cyan line" assume Part 4 was read closely; colours
   depend on the Terminal theme. Point at `?` (item 10) first, not last.
4. No "if it looks different" except `g` (3409); nothing for "I pressed a key and I am somewhere else" (Esc? Tab?).

## Part 10 — status and resuming (lines 3419–3607)

Goal is clear ("Where am I?") and the status table (3468–3476) is the best "what to do" table in the guide.

**Slows them**
1. 3502–3540: three `cargo install` steps with no time ("several minutes each") and no "you should see" beyond "the
   same as Step 0.7".
2. 3556–3578: resolving a merge conflict in nano is real editing of markers; the instructions are clear but this is
   the hardest task in the guide for this reader. Offer "or run `git merge --abort` and ask for help" first, not only
   for other files.
3. 3492: "Answer it, then run the same command again" — how to answer is in Part 3; give the step numbers (3.2–3.4).
4. 3586: "If the verdicts show STALE, re-verify the units" — give the two commands. (Polish: 3423 "Run these" is a
   Run label with no single box.)

## Part 11 — speed (lines 3611–3863)

Goal: the title is a question and 11.3's answers answer it. Good shape: "Before you start" is the only real
prerequisite check in Parts 6–12; copy it to Part 12.

**Slows them**
1. 3640: "half a second or more of the C's CPU time, or at least a billion instructions" — neither is checkable or
   needed; "the sample files are too small to time" is enough.
2. 3727: "a minute or two (the cockpit's estimate says about 3 minutes)" — two estimates.
3. 3735–3755: "shortened here", with `…` lines and `<n.nn>`, `<nnn>`, `<n.nn>e<n>`: the reader cannot check this box;
   say which three lines to look for (the first, any "about as fast", the last `exit=0`).
4. 3780: one sentence on the launcher, `cc --version`, `rustc -V`, "the sandbox", `--no-check`; the reader needs
   "this only reads". 3786: "the same rows as at the end of Step 11.3" — 11.3 never showed all rows.
5. 3776: "naming a command that holds the writer lock" — say "another harness command is running (maybe the cockpit in
   another window)".

**Polish**
6. 3849: placeholder `<7hex>`; the opening table (line 51) calls it `<hash>`. `<n.nn>`, `<nnn>`, `<n.nn>e<n>` are not
   in the table (3636 explains only `<n.nn>`).
7. Author's machine: line 7 says every output is from commit `a154870`; that commit has neither `harness perf` nor
   `harness project`. Parts 11–12 come from later commits and line 7 should say which and when they were walked.

## Part 12 — the project map (lines 3867–4712)

Goal: lines 3869–3879 say what happens, not what question is answered. The question to hold: "Can the harness find the
program inside an untouched download and set it up for me?" The checkpoint (4704–4712) answers it, but 12.10 and 12.11
come after the answer and dilute it.

**Words used before they are explained** (beyond the opening's gaps listed at the top): `main()` as "a program" (3874;
Part 1 hinted it at 665), "configuration" (3876, defined at 3892 only as "flags that matter (`-I` folders, `-D`
defines)" — `-D` is never explained), "tool" vs "target" (3893: how does a tool relate to the `targets/lzg` of Parts
1–11? never said), "linking cannot tell" (3891), "duplicate set" (3891), "map" (3889, clashes with Part 8's map),
"outside symbols", "compiler or runtime names", "guessed libraries", "kind guess", "link check", "incomplete",
"dot-folder" (all in the 12.2 box, 3960–3999), "sandbox" (4001), "advice", "hand-off", "key" (4123–4124), "slice"
(4134), "definer" (4206), "envelope" (first named at 4157, after it is used in the box at 4137), "Output contract"
(4156: the reader never read the request), "alternative-implementation" (4161: one of three reasons, the others never
listed), `root_hash`, `inputs_hash`, `schema_version`, `[llm]` (4246–4250), "ledger" (4312), "OUTPUT FORMAT" (4408),
"tokens in/out" (4468), "closure" (4568, 4574 — only in output), "SOURCE-STALE", "CONTRADICTION" (4592), "hash
updated" (4625), the ids `p1`, `t-`, `d1`, `d1.1`, `u-` (four naming schemes at once).

**Stops a non-technical reader**
1. 3881–3882: "Parts 2–11 are not needed" — but 12.4 relies on `REQ`/`RESP` and the envelope from Part 3, 12.7 on "the
   same one as Step 6.1" and "its layout (Step 3.3)", 12.9 on "As Step 6.3 explains", 12.10 on the status words of
   Part 10. A newcomer who starts here meets unexplained machinery at every hand-off. Either say "do Parts 3 and 6
   first" or explain the hand-off again in Part 12.
2. 3883: "docs/TUTORIAL.md … explains the words used here" sends the reader to another file without saying how to open
   it. The words must be explained here.
3. 4136–4141: the first `project ask` ends with `error: awaiting response` and exit 1. The text says "with exit code
   1" and calls only the slice lines "expected". It never says "this error is normal: the harness is waiting for your
   answer". Same at 4329–4331 and 4404–4406.
4. 3960–3999: a 40-line screen of dense output, then a "read it top down" that explains five of its ideas. The reader
   cannot check it line by line; tell them the three lines to look for (`programs: 3`, `link check: linked` under
   `t-lzg`, `duplicate set d1 … the choice is yours`) and that the rest is for later.
5. 4057–4065: "What to keep" asks the reader to judge compiler flags and explains `-I../include` becoming
   `-Isrc/include` through "the Makefile runs from `src/tools`". The reader cannot make this judgement; say plainly
   "here is the answer for liblzg; for your own project, the harness can propose it (`harness project ask --build`)",
   and keep the reasoning as an aside.
6. 4505: `cat >> …` appends; every other file box uses `cat >`. A reader who types one `>` wipes the harness-written
   `harness.toml`. Warn: "two arrows: this adds to the end".
7. 4553–4633: Step 12.10 is ten actions (change, map, read three lines, status, read a prose description of a 3-part
   line, accept, read, scan, plan, verify) with the expected status given in prose (4591–4593). It also tells the
   reader to accept again "to clear the note" right after the output said "accepting it again only clears this note" —
   so why do it?
8. 4699: "first write `~/lzg-cockpit/migration/map/config.toml` as in Step 12.3 (from another Terminal window)" —
   12.3's box writes `migration/map/config.toml` relative to the current folder, so the reader must change the path or
   `cd` first; the guide does not say which.

**Slows them**
9. 3874–3879: the four-step overview mentions `--tool` and `harness.toml` before either is explained; it is good as a
   map of the part, but needs a picture (see the proposal).
10. 3917: "Every command from here on runs from inside this folder" — but nothing says that `harness` commands without
    `--target` now act on the current folder, unlike Parts 1–11.
11. 4123–4124: "the model's name is part of the question's key, so changing it later asks a new question" — jargon;
    say "use the same `--model` word both times".
12. 4137 and 4330: the harness's own re-run line (`--target=. --provider=external …`) differs from the guide's
    command; say both work and to use the guide's.
13. 4146–4161: no "You should see nothing" after the `REQ=`, `RESP=` and `jq` lines, no `echo "$RESP"` check (Part 3
    and Part 6 had one).
14. 4220, 4236: accept says "the whole-program check is off until you fill in …" — a newcomer thinks they must act
    now; add "you do that in Step 12.9".
15. 4246–4250: the "You should see" for `cat harness.toml` is a prose paragraph of TOML terms; show the file, or say
    the two lines worth spotting.
16. 4258–4262 and 4537–4543: `git commit -q` prints nothing; no "You should see nothing".
17. 4398–4471: in Part 4 the reader accepted a GREEN attempt; here `migrate` promotes on its own ("promoted and
    verified"). Say why there is no Accept step.
18. 4655–4661: the cockpit here is a numbered text menu, not the full-screen cockpit of Parts 4 and 9 — warn the
    reader it looks different. "Do." (4663) is a new label not in "How to read this guide".
19. No "If it looks different" at 12.1 (e.g. `~/lzg-map` already exists from an earlier try: rsync merges into it and
    `migration/` survives), 12.6, 12.7, 12.8 (a RED translation), 12.10 or 12.11.

**Polish**
20. 3957: "in about 2 seconds" is the author's machine; say "a few seconds".
21. Line 27 calls Part 12 "30 min"; at 850 lines it is closer to an hour for a newcomer.

Where to split it: see the proposal at the end (three experiments: 12.1–12.5, 12.6–12.9, 12.10–12.11).

### Today's real output for 12.1–12.4 (built from this worktree, run on a copy of liblzg)

Run on a copy of `~/code/ruharness-test-downloads/liblzg` (commit 182b56c, the guide's pin), `RUHARNESS_ADOPTED`
pointed at a scratch file, Apple clang 21.0.0.
- 12.1: as described: none of the commands printed anything. (The copy was made with `cp` of the listed files instead
  of `rsync`, a limit of this review's sandbox; the result is the same files.)
- 12.2: the screen matches the box line for line, **except one extra line** the box lacks, just before `skipped
  folder: migration …`: `skipped folder: .git (a dot-folder, 0 C files)` — it appears because 12.1 made the copy a git
  repository. Add it to the box (and explain "dot-folder" in a word or drop the line). `git status --short` prints `??
  migration/` as shown.
- 12.3: the `grep` output, the second line `configuration: make, from make (stated in config.toml), flags -O3,
  -Isrc/include; -O3 is recorded only …` and the new last line all match. The refusal for pasted Makefile flags
  matches the box at 4109 word for word (exit 1).
- 12.4: the first `ask` matches (four lines, then the `error:` line, exit 1). The bare-array refusal matches 4185, but
  the real output prints the three `project ask:` lines first; the box shows only the `error:` line. The envelope
  answer and the second `ask` match the box exactly. The request's rules confirm the reply format and the three
  reasons (`platform`, `alternative-implementation`, `cannot-tell`), which the guide should list.

## Known quirks, Troubleshooting, What to try next (lines 4716–4871)

**Stops a non-technical reader**
1. The table (4729–4765) is ordered by no visible rule: general, then driver, chat, cockpit, verify, git, then Part
   12. A reader with a problem must read all 35 rows. Group it under small headings by where you are (installing,
   writing answers, the chat, the cockpit, verify, Part 12) or by the step number.
2. The most frequent scary line in the whole guide, `error: awaiting response: …` with exit 1, has no row. Add one:
   "Normal: the harness is waiting for an answer file. Go on to the next step."
3. Missing rows a reader can hit in Parts 6–12: `unrecognized subcommand` for `project` or `perf` (old install: Part
   10 "After updating"), a refused ledger "made elsewhere" (the adoption check; nothing in the guide mentions it), the
   perf errors (they live only in 11.3).
4. 4739, 4753, and the "When you ask someone for help" list (4848) give only `targets/lzg` paths and `--target
   targets/lzg`; in Part 12 the paths are `migration/tools/t-lzg/…` and the flag is `--tool t-lzg`.

**Slows them**
5. Rows with more than one action: 4737 (scan, plan, read diff, retry, or undo), 4739 (same model, `ls`, compare
   names, change unit), 4740 (check, then answer again via two steps), 4748 (a paragraph about BLIND hand-offs and
   `targets/tractor`), 4764 (three kinds of flag). Each should lead with the one thing to do and put the rest under
   it.
6. 4841: "When you ask someone for help" — who, and where? Say where to ask.

**Polish**
7. What to try next, item 2 (4863): "Change `1.0.10` in its Rust" — give the file path
   (`targets/lzg/migration/units/u-version/u_version_rs/src/logic.rs`, by the ledger layout at 3603 and the file names
   of 12.8).
8. Item 9 (4870) needs a download with no commands; item 10 (4871) lists scratch folders to delete but forgets
   `~/lzg-map` and `~/lzg-cockpit` from Part 12.

## Proposal: how to reshape Part 12 (keep every command and output)

**Put the question first.** "Your own project will not be laid out like Part 1's folder. Can the harness find the
program inside an untouched download and set it up for you?"

**Say what you need, and check it.** Part 0, Step 1.2's download, and Parts 3 and 6 (for how a hand-off is answered
and why whole-program runs miss `u-version`). Add a "Before you start" like Part 11's: `harness project --help`; if it
says `unrecognized subcommand`, update.

**Give a picture in words before any command** (about 12 lines, replacing the five-row table):
- "A download is a box of `.c` files. Some of them start a program — they hold `main()`, the place a program begins.
  liblzg has three: `lzg` (compress), `unlzg` (decompress), `benchmark` (timing)."
- "A program needs other files. Building means turning `.c` files into a runnable program; **linking** is the last
  part, where every function a file asks for must be found in exactly one file. The **map** lists, for each program,
  the files it needs, and whether they link."
- "Sometimes two files offer the same function. Here the library's decoder and a small stand-alone decoder both offer
  `LZG_Decode`. Both would work, so the harness will not guess: it **holds the choice** (`d1`) and you pick one
  (`d1.1` or `d1.2`)."
- "The harness also needs to know how the project is normally built: which folders to search for **header** files
  (`-I`). That is the **configuration**, three lines you write."
- "When you **accept** a program, the harness writes the `harness.toml` you wrote by hand in Part 1. An accepted
  program is called a **tool**; from then on you name it with `--tool t-lzg` where Parts 1–11 used `--target
  targets/lzg`."
- One line on names: "`t-` = a program, `u-` = a unit, `d1` = a held choice, `p1` = a program's row number on the map
  screen." Also rename Part 8's result "the feature map" so "the map" here means only one thing.

**Split into three experiments, each with its own checkpoint:**
- **12A Find and accept (12.1–12.5).** Question: "Which programs are in liblzg, and can I make one my target?" For
  12.2 tell the reader the three lines to find, then keep the full box as "the whole screen, for reference". Move
  12.3's flag reasoning into an aside; lead with "here is liblzg's answer". Keep 12.4 but mark it optional and repeat,
  in two sentences, how a hand-off works and that `error: awaiting response` is normal; list the three reasons.
  Checkpoint: two `harness.toml` files exist.
- **12B Migrate one unit of the tool (12.6–12.9).** Question: "Does everything from Parts 3–6 work the same on an
  accepted tool?" Say up front why there is no Accept step this time, warn about the `>>` in 12.9, and say in 12.5
  that the whole-program check is switched on in 12.9.
- **12C When the project changes; the cockpit way (12.10–12.11).** Question: "If I edit the C, does the harness
  notice, and what do I do?" Break 12.10 into: change the file → map (look for one line) → status (show the real lines
  as a box, not prose) → accept again (say why: it records that you have seen the change) → scan → plan → verify. For
  12.11, warn that this cockpit is a numbered text menu, and give the `cd ~/lzg-cockpit` line before the config box.

**Add "If it looks different" to every step**, the shortest being "if you see `error: awaiting response`, that is
expected — continue". Add the `.git` line to the 12.2 box.


---

# Decisions (the session, 2026-10-09)

The guide is accurate (almost every box still matches today's output) and in places excellent (Part 5's
"read the result like a detective"), but it is written for someone who already knows what a compiler,
a header, a build and a commit are, it has no shape a newcomer can hold, and three things stop a
non-technical reader outright: nothing says how to get RuHarness onto the Mac, Step 0.8 fails today
with the adoption question the guide never mentions, and expected errors (`error: awaiting response`)
are never called normal where they appear. The guide is rewritten to one shape, part by part, keeping
every real command and every real output.

1. **Every part follows the template in the Parts 0–5 report**: the question; you will know the
   answer when; takes / uses Claude; Before you start (where, what must be true, what to keep open,
   new words in this part); numbered steps that each do one thing with Run / You should see (and
   when it is finished) / What it means / If you do not see that (an action, never only a cause; the
   last row always the part's reset recipe); Answer; Checkpoint (only what was run, quoting its
   output); If you need to start this part again; For the curious (optional).
2. **Three words up front** (unit, judge, driver); every other word is explained in the step where
   it is first used, in one sentence a non-programmer can hold. Compiler, build, link, header, flag,
   function, `main()`, library, sandbox, git/commit/branch, hash, checksum, Makefile each get that
   sentence at first use. "How to read this guide" keeps two rules (copy a Run box whole; wait for
   the prompt) and moves the rest to the first step that needs it.
3. **A step that gets RuHarness** (clone, with the URL, and "if the folder already exists") before
   anything `cd`s into it; one line on how to check the Mac's chip; an Intel note where the
   toolchain name differs.
4. **The adoption question is a step**: Step 0.8 runs the first command with `--adopt`, says in one
   sentence why the harness asks once, and the troubleshooting table gets the row.
5. **Every expected `error:` is announced before it appears** ("this step is meant to stop with an
   error: the harness is waiting for your answer"), at every hand-off, in Part 7's refusal, and as
   the first troubleshooting row.
6. **One thing per step.** Steps 0.2, 0.7, 1.8, 3.3, 4.4, 6.2, 9's tour, 12.10 are split; every
   "You should see nothing" is followed by a line that prints something; every box the reader must
   compare by eye names the one or three lines to look for; every `cat >>` says "two arrows: this
   adds to the end".
7. **Times, versions and paths from the author's Mac are labelled as such** everywhere; line 7
   names the commits and dates each part was walked (Parts 11 and 12 came later than `a154870`).
8. **Part 12 is reshaped** as the Parts 6–12 report proposes: the question first, a Before-you-start
   check (`harness project --help`), the picture in words (programs and `main()`, building and
   linking, a held choice, the configuration, tool vs target, the four kinds of names), then three
   experiments 12A (find and accept), 12B (migrate one unit of the tool), 12C (when the project
   changes; the cockpit way), each with its own checkpoint; the hand-off explained again in two
   sentences; the three reasons listed; the `.git` line added to the 12.2 box; Part 8's result called
   "the feature map" everywhere so "the map" means one thing.
9. **Part 4's Plan B becomes an appendix** with its own question, before-you-start and checkpoint;
   Part 6's "use Plan B with u-version in place" names the lines that change.
10. **The troubleshooting table is grouped by where you are** (getting ready, answering a hand-off,
    the chat, the cockpit, verify, the map), each row one thing to do, Part 12's `--tool` paths
    beside the `targets/lzg` ones; "when you ask someone for help" says where.
11. **Boxes that drifted are refreshed from real runs**: Step 2.2's status line, Step 2.5's file
    count (4, with the ledger's own `.gitignore`; Step 1.11's hand-added lines dropped), rustup's
    extra lines, Step 3.2's one-line C dump (show the section names only), the 12.4 refusal's three
    preceding lines.

## The rewrite

Three writers in parallel, one per third (the opening through Part 2; Parts 3–8 with Plan B as an
appendix; Parts 9–12 and the closing sections), each running every command on a fresh copy and
pasting the real lines (the parts that need Claude keep the recorded outputs and are restructured
only), then one reader who has not seen the old guide reads the whole new one as a non-technical
person and runs Parts 0–3 and 12A.
