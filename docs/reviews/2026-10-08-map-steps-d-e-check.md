# Check round over the map's steps (d) and (e) — 2026-10-08

Three Opus checkers at high effort over the branch at 0682103: correctness of `project ask` and `project accept`; security of the prompts, the validators and the written tool; a newcomer's cold start on liblzg and zopfli by the docs alone. Their reports follow verbatim; the triage is at the end.

---

# Check of steps (d) and (e): are `project ask` and `project accept` right?

Worktree `agent-a60a39ead386edd36`, fast-forwarded to 9ebca98 (holds 0682103). Nothing committed;
every source change made for the reverts was restored (`git status` clean). Every run used
`RUHARNESS_ADOPTED` pointed at a scratch file; no model or network was called — every hand-off
answer was a file I wrote. Scratch scripts and copies are under
`…/scratchpad/check3/de/` (`forge.sh`, `build.sh`, `reply.sh`, `refuse*.sh`, `changed*.sh`,
`lzg*.sh`, `zop.sh`, `cockpit*.sh`, `mutate.sh`).

Both test files pass as merged: `project_ask` 16 passed (1 ignored, the regenerator),
`project_accept` 9 passed.

## Findings

1. **Medium — `ask`'s screen says "it did not link" for a choice the map never tried to link.**
   When a held set is over the limit (more than 4 definers, or more than 16 combinations), the
   map holds the program without linking anything (`closure.rs:820`, `links` left empty), and
   `linked_split` (`project_ask.rs:454`) reads an empty `links` as "this choice failed".
   Ran it: a real map of `main.c` + five files defining `f` (`overlimit_real.sh`), then an
   answer `keep d1.3` printed
   `d1 (f; held by t-main): the model's advice (…): keep d1.3 src/f3.c, reason …; in t-main it did not link`.
   The person reads a link failure that never happened and may throw away a good file. (The
   map's own screen says "linking cannot tell d1.1 … from d1.5 apart" for the same set, also
   untrue: it never linked.) Fix: carry "linked or not tried" per program into `SetLinks`
   (the closure's `linked` is absent and the set is held with no `links`), and say "the map did
   not link these choices (too many to try)".

2. **Medium — the cockpit's Ask fails on a guessed map with held choices, and its heading
   points the person at it.** `argv` (`harness-tui/src/project.rs:180`) runs the questions
   whenever something is held and never passes `--allow-guessed`; the heading says "Its
   configuration is a guess … (write it in migration/map/config.toml, or Ask for a proposal)"
   (`project.rs:291`). Ran it headless (`cockpit.sh`, the ask fixture, which is guessed and
   holds d1, d2): typed 2, y → `error: the configuration is a guess, so the questions may be
   wrong: … or pass --allow-guessed`, exit 1, with no way to pass it from the cockpit. On a
   stated map (the real liblzg copy) the same Ask ran and stopped at the hand-off as it should.
   Fix: while the configuration is a guess, Ask runs `--build` (the question that is open), and
   only a stated map asks the held choices.

3. **Medium — a library's id moves when a new file sorts first, and the "what changed" line
   then gives a command that is refused.** `accepted_ids` (`mapfile.rs:586`) keeps an `l-` id
   only for the library whose *first* file the tool lists. Ran it (`libid.sh`): accept `l-crc`
   (`lib/crc.c`, `lib/step.c`), add `lib/aa.c` that `crc.c` calls, map again → `library l-aa:
   lib/ aa.c, crc.c, step.c` and `accepted tool l-crc changed since it was accepted: it is no
   longer a library in the map; accept it again with harness project accept l-crc`; that command
   answers `the map has no program or library l-crc (its ids are l-a0, l-aa)`. The tool and its
   ledger are orphaned; accepting `l-aa` starts a second ledger. Fix: keep the id for the library
   that holds most of the tool's listed files (or any of them), and when the id still moves, name
   the new id in the sentence.

4. **Medium — `accept` takes a library whose code calls into a file that did not compile.**
   Ran it (`refuse.sh` R12): `lib/crc.c` calls `crc_step`, defined in `lib/step.c`, which has a
   syntax error. The map lists `library l-crc: lib/ crc.c`, `needs_from_outside: []`, step.c
   "unreached"; `accept l-crc` exits 0 and writes a one-file tool. The library check
   (`accept.rs:682`) only asks whether the library's own files compiled; nothing asks whether a
   symbol it needs may be defined in a file that did not compile, which is exactly the
   "incomplete" refusal a program gets. `crc_step` will read as an outside call later. Fix: run
   the same `Gaps::why` over the library's undefined names and refuse as "incomplete".

5. **Medium-Low — "what changed" blames the tool's files when they did not change, and names a
   new program only once.** The fallback line (`mapfile.rs:812`) says "its files changed since it
   was accepted (same closure, same configuration…)" whenever the digests differ and nothing else
   was found. Ran (`changed.sh`, `changed2.sh`): a different `inputs_hash` alone (as after a
   compiler update), adding `system_headers` to `config.toml`, and adding an unrelated
   `tools/new.c` each print exactly that line. "New programs since the last map" compares with the
   previous map, not with the acceptance (`project.rs:191`): on the next map the new program is no
   longer named while the "changed" line stays, with no cause given. Fix: say which digest moved
   ("the project's files changed" / "the configuration or the compiler changed"), and compute new
   programs against the map the tool was accepted under (or keep naming them until it is accepted
   again).

6. **Medium-Low — a truncated answer has no way forward.** `check_stop` (`projectask.rs:1293`)
   says "response truncated …: ask again with a larger budget", but `project ask` has no budget
   flag (`max_tokens` is fixed at the default, `project_ask.rs:106`), and under `external` the
   sentence does not name the response file or say to delete it, so every re-run repeats the
   error. Ran it (`replay_missing.sh`): `error: external: response truncated (stop_reason
   max_tokens): ask again with a larger budget`. Fix: under `external`, the same sentence as a
   contract failure (name the file, "delete it and answer again"); otherwise say how to raise the
   budget, or ask fewer items per call.

7. **Low — both validators take a JSON object with a duplicate key (the last copy wins), and
   neither checks field order.** Ran (`forge.sh`, `build.sh`): `{"item":"d1","keep":"d1.1",
   "keep":"d1.2",…}` is accepted as d1.2; `{"item":"d2","item":"d1",…}` answers d1;
   `{"flags":[{"flag":"-fplugin=x.so",…}],"flags":[],…}` writes a proposal with no flags and the
   forbidden flag never refused. The fields reversed (`purpose, name, kind, item`) are accepted
   although §3.4 says "in a fixed field order". Harm is small (labels and advice, and the written
   file holds only what validated), but "refused in full" is not strict and two readers of the
   same response file can disagree. Fix: parse with a map visitor that refuses a repeated key;
   either check the order or drop "fixed order" from the design.

8. **Low — a name made only of combining marks passes.** `check_line` (`projectask.rs:1045`)
   counts characters and trims whitespace; 40 U+0301 marks pass as a 1–40 character name and print
   as accents piled on the opening quote (`name "́́́…"`). Fix: refuse a name or purpose that
   holds no base character, or count only non-mark characters.

9. **Low — `--run-name` on a library is written unchecked.** `accept.rs:759` takes the given name
   for a library and the check at `accept.rs:763` runs only for programs. Ran: `accept l-crc
   --run-name '../../escape'` and `'.hidden'` both exit 0 and write `name = "../../escape"`.
   Downstream `program_name` filters it, so nothing escapes, but SCHEMAS says a library's `name`
   is its id. Fix: refuse `--run-name` for a library (or check it the same way).

10. **Low — wrong or circular sentences on the way to the open question.** (a) Guessed map with
    nothing held: `ask` → "pass --allow-guessed"; with it → `nothing is open: every program linked
    and every duplicate set is settled` (`guessopen.sh`). The configuration *is* the open question;
    the refusal should name `--build`. (b) The same "every program linked" sentence prints when a
    program did not link (none of its choices links). (c) `ask --programs t-driver` says
    "`t-driver` is a driver program: only a program with its own main() is asked about" — a
    driver has its own `main()`; say "only a main program". (d) The cockpit's Ask dialog says "one
    model call" though more than 10 items make several (`project.rs:231`).

11. **Low — a `--keep` for a set the final closure does not reach is dropped without a word,
    and the cockpit asks it anyway.** Ran (`nested.sh`, d2 reached only under d1.1):
    `--keep d1=d1.2 --keep d2=d2.1` writes only the d1 pick and says nothing about d2. The cockpit
    asks for every held set in turn, so it asks d2 even after the person chose d1.2. Fix: print
    "d2 is not reached with these picks; not recorded", and have the cockpit skip a set whose
    `under` excludes the choice just made.

## Tests

Reverted eight small pieces for real, ran the matching tests, restored each (`mutate.sh`):

| Reverted | Caught by |
|---|---|
| `keep` must be one of the set's own definers | `forged_replies_are_refused_in_full_naming_the_index_and_the_rule` |
| reply file bound to its digests (always merge) | `a_reply_for_another_map_is_replaced_unread` |
| every batch written (`continue` → `break` on awaiting) | `every_batch_is_written_in_one_run_and_merged_as_it_validates` |
| `accept`'s re-link result ignored | `accept_links_again_before_writing` |
| system-header folder not moved to `-idirafter` | `a_system_header_folder_becomes_an_idirafter_flag` |
| "linked"/"did not link" swapped | `a_recorded_answer_is_read_as_advice_and_shown_with_its_link_results` |
| "what changed" silence when digests match | `a_later_map_reports_what_changed_for_each_accepted_tool` |
| `picks.retain` (record only reached sets) | **nothing** — no test has a nested set at `accept` |

Each test that caught its revert proves its name. Gaps: no CLI test for the refusals after "map
changed" other than those in `accept_refuses_in_one_sentence_and_writes_nothing` — the limit hit,
the fuzzer and fuzz driver, two definers for one set, a library file that did not compile, an
ambiguous include, a file missing from `compile_commands.json`, flags that differ, a bad run name.
I triggered every one with its own small project (`refuse.sh`, `refuse2.sh`, `driver.sh`): each
fires alone, one line, exit 1, nothing written. Worth adding: those, a nested-set accept (would
catch the `retain` revert), an over-limit set at `ask` (finding 1), and a duplicate-key reply.

## Holds

- **`ask`'s open-question rule.** Only open items are sent: programs only when `--programs`
  names them (refused for a fuzzer or driver, and for an id not in the map, naming the ids), then
  every held set in index order, at most 10 per call (14 items → 2 calls). A program sends `{id,
  path, folder, bytes, functions, includes}`; a set its symbol union, holder ids and per definer
  the facts and a slice capped at 120 lines (checked on liblzg: `decode.c` and `lzgmini.c` each
  sent 120 lines starting at `LZG_DecodedSize`). The trusted part holds only checked indexes;
  facts sit in nonce blocks with `<` escaped.
- **Both validators against the rules**: another set's definer (`d2.2` for d1), `D1`, `D1.2`,
  `"d1 "`, an object instead of an array, prose after the array, null, blank or NBSP-only name,
  U+200D, U+2028, U+001B — all refused in full naming the index and rule, nothing written. A
  Markdown fence with a language tag is stripped. `--build`: a cite past the end (`Makefile:9` of
  8 lines), line 0, `./Makefile`, a file not sent, `-lm`, `-Wall`, `-fplugin=`, `-Imigration/map`,
  `-I…/../..`, an uppercase name, `from: autotools`, a 201-character assumption — all refused.
  A flag naming a folder no sent file mentions (`-Isecret`, `-includesecret/s.h`) or a missing
  folder passes: the design asks only that the path be inside the project, as `config.toml`'s are.
- **The reply file across a map change and back** (`reply.sh`): merged under the same digests
  (p2 kept, d1/d2 refreshed), replaced unread for map B, untouched by an awaiting run, replaced
  again on the way back to A, whose answers return from the stored responses.
- **Error paths**: external contract failure names the response file and says "delete it and
  answer again"; replay says "record a live run" (or "record a live run first or use the external
  provider" when missing); no map → "no map written yet: run `harness project map`"; the resume
  command is exact (`--target=… [--build] [--programs=…] [--allow-guessed] --provider=… --model=…`,
  no `--adopt`/`--json`).
- **`--build` caps**: 64 KiB each, 128 KiB total with the largest left out first, links and
  non-build names left out and each named with why (the test proves it; read the code).
- **`accept` on the real liblzg** (`lzg1`–`lzg6.sh`): map holds t-unlzg and t-benchmark on d1;
  no keep → the one-sentence refusal naming both definers; `--keep d1=d1.1` → 2 files
  (`lzgmini.c`, `unlzg.c`); `--keep d1=src/lib/decode.c` → 3 files (`checksum.c` comes back only
  with `decode.c`), printed in words before writing, "alternative not kept", `replaced: true`,
  ledger kept. Then `scan`, `plan` (u-checksum, u-decode, u-unlzg) and, with a hand-written
  `checksum_rs` and the practice driver under the tool, `verify u-checksum` **GREEN** (symbol set,
  capabilities, driver shape, differential 2 936 bytes identical, sanitizers).
- **zopfli identity, re-verified** (`zop.sh`): a copy without the root `harness.toml`, `flags =
  []`; map → one program, 13 files, `-lm`; `accept t-zopfli_bin --run-name zopfli` → 13 files with
  no include folders, no flags, `extra_link_args = ["-lm"]`; `scan --tool` gives `facts.jsonl`
  byte-identical to the committed one; a fresh plan's 11 `source_hash` values equal the committed
  plan's; with the committed plan and crate copied under the tool, `plan: no changes (11 units)`
  and `verify u001-katajainen` GREEN (differential 183 832 bytes identical, sanitizers clean).
- **Re-link both ways**: a map file claiming `ok` for a program that does not link is refused;
  one claiming a failure for a program that links is accepted; a stored linking choice edited to
  the definer that does not link is refused; a held set deleted from the map file is refused
  ("the map holds no set for them"); definer paths swapped in the map file are followed, and the
  printed sentence names the file actually kept.
- **Every refusal fires alone**, in SCHEMAS' order, one line, nothing written (`refuse*.sh`).
- **Written `harness.toml`** loads through every command (scan/plan/verify ran on it): files in
  path order with their folders, the configuration's own copy, `-idirafter` from
  `system_headers` (test), `extra_link_args` as linked, run name, map stamp, picks by person or by
  links. "What changed" is silent when nothing changed (and for a README edit).
- **The cockpit headless** (`script` pty): heading, menu, the Accept picker listing each held set's
  definers with none suggested, the dialog's command line identical to the CLI's, Accept writing
  the tool and opening it; the "project changed" notice shows in `state status`.


---

# Security check of steps (d) and (e): `project ask`, `project accept`, the cockpit's project mode

Worktree fast-forwarded to 9ebca98 (head of `claude/ruharness-resume-ee312a`, after 0682103).
Scope: what an untrusted download or a hostile model reply can do through `project ask`,
`project accept`, the accepted-id reading and the cockpit's project dialogs. Experiments used a
release build against a crafted project under the scratchpad (`scratchpad/check3/sec/e*.sh`),
`RUHARNESS_ADOPTED` pointed at a scratch file; `--provider external` only (requests written,
responses written by hand), so no model or network was called. Nothing downloaded, installed or
committed; the worktree is clean.

One fact frames most of what follows: a shipped map file, reply file, traces folder or
`config.proposed.toml` counts as "results" (`adopt.rs:302-330`), so the download reaches them only
after the person adopts the folder. Adoption is a single yes, and after it every file in the
download counts as the person's own, so the findings below assume the person adopted.

## High

None found.

## Medium

1. **`accept` trusts the map file for which files and which choices go into the tool. The
   digests tie the map to the files on disk, not to what the map says about them.**
   Evidence: `accept.rs:550-560` checks only that `root_hash`/`inputs_hash` still match the
   files. After that, the map file decides: the program's path for an id (`accept.rs:630`), a
   library's whole file list (`accept.rs:698`, each file only checked to have compiled), the
   duplicate sets' definers (`accept.rs:620`), and a set's `choice`, which is honoured as
   "settled by linking" (`accept.rs:612`). The re-link checks only the one choice it was given.
   The root hash can be computed from the files the download ships. The inputs hash depends only
   on the configuration and the compiler, so it can be guessed for a common Xcode.
   Ran it, yes (`e8.sh`): a project whose set d1 is really held (both `src/a.c` and `src/b.c`
   link). The honest map makes `accept t-main` refuse until `--keep`. I edited the map file to
   add `"choice": {"keep": "d1.2"}` and empty `questions`. `accept t-main` with no `--keep` then
   wrote the tool with `src/b.c` and recorded
   `picks = [{ definers = [...], keep = "src/b.c", by = "links" }]`. It printed "(settled by
   linking)", which is untrue.
   Effect: the gate "a held choice is the person's" (§3.6) is passed by the download. The written
   `by = "links"` hides that from `git diff`. In the same way, a library can be written with any
   compiled files the download names, and an id can be pointed at another main program. Every
   path still has to be a walked file inside the root, so nothing escapes the root.
   Fix: after re-mapping, compute the closures and duplicate sets from the fresh map (this needs
   no linking). Use the map file only to turn the person's indexes into paths, and refuse when
   its ids, paths, file lists or definer sets differ from the fresh ones. Never take `choice` from
   the file: treat the set as held unless re-linking the other definers shows that only this
   choice links.

2. **The cockpit's Accept dialog prints a set's name and its definers' indexes from the map file
   with no filtering: terminal escape codes and newlines reach the terminal.**
   Evidence: `harness-tui/src/project.rs:355-360` uses `h.set` and `{index}` without
   `safe_line`. `read_map` (`project.rs:94-150`) reads the map with no shape checks, unlike
   `ask`'s `check_map_shapes`.
   Ran it, yes (`e10.sh`, under `script` to get a terminal): a map with set
   `"d1\x1b]0;TITLE-SET\x07\nFAKE LINE"` and index `"d1.1\x1b[31mRED\x1b[0m"` printed both raw. The
   title-setting code, the colour codes and a line of the download's choosing all reached the
   terminal.
   Effect: the review gate can be forged. The design's §3.9 rule "newlines and tabs are filtered
   in every printed project string" is broken. Some terminals also act on other escape codes,
   such as writing the clipboard or making a hidden link. The `It runs:` line is filtered, so the
   real `--keep` stays visible there.
   Fix: put `h.set` and `index` through `safe_line`, and have the cockpit's `read_map` refuse
   ids, sets and indexes outside their shapes, as `projectask::check_map_shapes` does. That check
   also stops an id that starts with `-` from reaching the argv.

3. **A duplicate's slice is limited to 120 lines but not in bytes, and finding the definition
   slows down with the square of the line length: one project file can make `ask` send a
   multi-megabyte prompt or run for hours.**
   Evidence: `projectask.rs:822` cuts the line count only. A definer is read up to 8 MiB
   (`SOURCE_READ_MAX`). There is no limit on how many definers a set has, and no limit on bytes
   per request. `find_definition` (`projectask.rs:712`) calls `definition_end` for every whole-word
   hit of the name at file scope, and each call scans to the end of the line, and up to 4 000
   lines (`projectask.rs:742`).
   Ran it, yes (`e3.sh`, `e4.sh`). A definition holding a 7 MiB comment gave a 7.3 MB
   `*.request.json` from 5 lines. A file-scope table `{f,f,f,…}` before `f`'s definition took
   0.44 s with 20 000 entries and 6.3 s with 80 000 (four times the entries, sixteen times the
   time). At the 8 MiB cap (about 4 million entries) that comes to roughly 4 hours per definer,
   all under the project lock. Reasoned, not run: ten sets per batch with several 8 MiB slices
   each, and any number of batches written before the run exits, multiply the disk and memory. A
   request over 16 MiB cannot even be read back for replay (`MAX_TRACE_BYTES`). With a live
   provider, each such prompt is paid for, or refused by the API.
   Fix: cap each slice in bytes (for example 16 KiB per definer, and each line cut to a few
   hundred characters, saying it was cut), cap the definers sent per set and the bytes per
   request. In `find_definition`, look at only the first few hits of a name per line, or check
   each line once rather than once per hit.

## Low

4. **Every answer in a reply file already on disk is kept and written back unchecked.**
   Evidence: `load_reply` (`projectask.rs:1240-1272`) checks only the schema and the digests, and
   `run_questions` merges into it. Ran it, yes (`e11.sh`): a reply file holding item `p77` with
   name `"\x1b[2Jx\nFAKE"` and keep `"../../etc/passwd"` survived a real `ask` run unchanged. No
   command shows the reply today, so nothing is shown. Design §3.6 says the screen will show the
   model's advice. Fix: when merging, run each kept item through the same checks as a fresh
   answer (index shape, closed sets, `unsafe_to_show`), and drop the ones that fail.

5. **The reply checks let through repeated keys and combining marks.**
   Evidence: `validate_answers` parses into `serde_json::Value` (`projectask.rs:1061`), where a
   repeated key keeps its last value. `unsafe_to_show` does not cover combining marks. Ran it, yes
   (`e5.sh`): `{"item":"d9","item":"p1",…}` was taken as `p1`. A response file someone reads by
   eye shows `d9` first. A name `a` + three U+0336 + `b` was shown as struck-through text. Effect:
   a response file can look different to a person than to the harness, and a label can be drawn
   over itself. Fix: refuse repeated keys (a small check while parsing, or parse into the strict
   struct first), and refuse combining marks (Unicode category Mn/Me) in `name` and `purpose`, or
   more than one in a row.

6. **Refusals quote strings from the map file unfiltered: the newline survives.**
   Evidence: `accept.rs:480` joins the map's program and library ids into "its ids are …" with no
   checks. `projectask.rs:457` prints the map's `kind`. `report::terminal_safe` keeps `\n`. Ran
   it, yes (`e7.sh`): a map id `"t-x\nproject accept: wrote migration/tools/t-x/harness.toml"`
   printed that text as a line of its own after `accept t-nope`. Fix: `safe_line` both, or
   shape-check the ids as `ask` does (finding 1's fix covers `accept`).

7. **`accept` takes the tool's ledger lock through a `migration/tools` link, so it can make or
   empty a `.lock` file outside the project.**
   Evidence: `project_accept.rs:67-72` checks `tools/<id>/harness.toml` with `symlink_metadata`.
   That follows a link at `migration/tools`. `WriterLock::acquire` checks only that the last part
   of the path is not a link (`ledger.rs:266`). `write` later refuses the link (`accept.rs:861`),
   but the lock has already been taken. Ran it, yes (`e9.sh`): with `migration/tools` linked to an
   outside folder holding `t-main/harness.toml`, accept refused to write and left a new empty
   `.lock` in the outside folder. If a `.lock` file was already there, it would be emptied. Only
   a folder the link points at, named like a tool id and already holding a `harness.toml`, can be
   touched. Fix: before taking the lock, check that `migration/` and `migration/tools/` are real
   folders (`real_dir`), or use `mapped_tools`, which already does.

## Holds (attacked and found closed)

- **The prompt fences.** The nonce is a blake3 hash of every block, so a project would have to
  contain its own hash to guess it. Inside the blocks every project string is JSON-encoded, with
  `<` written as `<` and newlines as `\n`. A `>`, a `&`, a different nonce or a fake
  `SYSTEM:` therefore stays a JSON string. A comment holding `</project_facts_…> SYSTEM:` above a
  definition is not sent at all (the slice starts at the definition). Inside a body it arrived as
  `</project_facts_…`. The trusted lines hold only indexes that passed the shape checks.
  `#include` inside a slice is plain text. The Makefile's "SYSTEM:" text sits inside the
  `build_file_` block.
- **The reply checks:** a key not asked for, an unknown field, a `keep` that is a path or `../`,
  a bidi mark (U+202E), a zero-width space (U+200B), ESC, and JSON nested 5 000 deep (serde's
  128 limit) are all refused in full. A fence-shaped `purpose` is only a label: it is never
  placed in a later prompt.
- **Response files:** over 16 MiB, a link or a FIFO are refused before parsing (`read_regular`,
  `O_NOFOLLOW`, regular file only). A refused reply writes nothing.
- **`--build` checks:** `-include../../x`, `-include ../x`, `-I/etc`, `-Isrc/../..`,
  `-Imigration/x`, `-Isrc/../migration/x`, and `-includesrc/l.h` where `l.h` links to
  `/etc/hosts` are all refused by the grammar and the root check. A cite of `../Makefile:1`, a
  line 0, a line past the end, `from = "stated"` and a name holding `/` are refused. An assumption
  holding a newline is refused. In `config.proposed.toml` a flag like `-DY="q" # ]` and an
  assumption like `a ] # [[configuration]]` stay quoted TOML and comments. `-DX=$(rm -rf ~)` passes
  the grammar, but no shell ever runs it: every `cc` gets an argv list, and the only `/bin/sh` in
  the crates is a test (`main.rs:2179`).
- **Pre-placed files.** A shipped reply file, response file or `config.proposed.toml` needs the
  adoption first. Even then a pre-placed response is only advice or a proposal that nothing
  reads, as §3.4 says. The proposed file is never read by any command, and its header says it is
  the model's.
- **Links around `ask`:** `migration/map` as a link is refused by the project lock, and `traces`
  as a link by `traces_dir`. Slices and build files are read only when their real path is the
  path itself, and only regular files.
- **`accept`'s output.** The files, include folders, flags and configuration name come from the
  fresh map. `extra_link_args` are only `-lm`, `-lz` and `-lpthread`, as the fresh link used them.
  `[oracle] allowlist` and `[llm]` are constants, never copied from anywhere. The written text is
  loaded as a `TargetConfig` (grammar included) before it is written. It is written with a
  temporary file and a rename, refusing a link at `tools`, `tools/<id>` or the file.
- **`--keep` and `--run-name`:** `--keep d1=src/../src/b.c` is refused, since a value must equal
  a definer's index or path exactly. `--run-name` values `a/b`, a newline, `../x`, `x y` and
  `.hidden` are refused. A library's name is its checked id.
- **The re-map and the link** run under the map profile (`Runner::map`). `require_sandbox` is
  called. Locks are taken in the order project, then tool, everywhere (`sync-runtime` too), and
  they are non-blocking, so two commands cannot wait on each other forever.
- **Ids from tool files.** `accepted_ids` and `what_changed` read a tool's `harness.toml` only
  through `mapped_tools`: a tool-id-shaped name, a real folder and a real file, at most 1 MiB. A
  listed path is only compared with walked paths. A shipped id keeps a program's id, but only if
  it passes `is_tool_id` and is not already taken. Every "what changed" line quotes the shipped
  file's strings through `safe_line`.
- **The cockpit:** the menu, the heading, the ids, the paths and the `It runs:` line go through
  `safe_line` (finding 2 is the exception). The adoption gate runs before project mode. Only a
  typed `y` runs anything, and with an argv list, not a shell.


---

# A newcomer's cold start on a real project, by the docs alone (steps d and e)

Checker: Opus, worktree agent-a409073976acdc0f8 fast-forwarded to 9ebca98 (contains 0682103).
Binaries built from that tree (`cargo build --workspace --bins`, 54 s). Every run used a scratch
`RUHARNESS_ADOPTED`; no model, no network, nothing downloaded. liblzg was present under
~/code/ruharness-test-downloads/liblzg and was copied (no `.git`) to my scratchpad. Logs of every
command: `check3/de/logs/` (one `.out`/`.err` pair per step named below).

Result: **a verified unit on both projects** — liblzg's tool `t-lzg`, unit `u-version` (GREEN,
6 checks, then 8 with a hand-added whole-program check), and zopfli's tool `t-zopfli_bin`, unit
`u-katajainen` (GREEN, 6 checks). What I read as the newcomer: README, docs/TESTING-GUIDE.md,
docs/SCHEMAS.md, `harness --help`, `harness project --help` and each subcommand's help. README,
the tutorial and the testing guide say nothing at all about `project map/ask/accept` or `--tool`.

## The walk on liblzg

1. `harness --help`, `harness project --help`, `… map|ask|accept --help`. The help names the three
   commands but not their order, and points at docs/PROJECT-MAP-DESIGN.md. `--configuration` says
   "of migration/map/config.toml" without its shape.
2. `harness project map --target .` (exit 0, 2.0 s, 34 lines). Readable: one block per program,
   the duplicate set in plain words ("held, linking cannot tell d1.1 src/extra/lzgmini.c from
   d1.2 src/lib/decode.c apart, so the choice is yours"). Key lines:
   `configuration: a guess (no migration/map/config.toml), flags none; failed compiles are expected until that file states the build`
   `p2 t-lzg — src/tools/lzg.c … link check: linked`
   last line: `… next, make a program or library a tool with \`harness project accept <id>\`; the held choices (d1) are yours to make …`
3. Followed that hint: `harness project accept t-lzg --target .` (exit 1, 1.0 s):
   `error: the configuration is a guess, and a tool is built under a stated one: write the build's name, from and flags in migration/map/config.toml (or use \`harness project ask --build\`), map again, then accept`
4. Writing `config.toml` with nothing to copy from — three tries:
   - `name = "make"` / `from = "make"` / `flags = []` → `parse error in …/config.toml: line 3: unknown field \`flags\`, expected \`configuration\`; fix the file`
   - `[configuration]` table → `line 1: invalid type: map, expected a sequence; fix the file`
   - `[[configuration]]` → accepted; `configuration: make, from make (stated in config.toml), flags none`.
5. Pasting the Makefile's real flags (`-O3 -funroll-loops -W -Wall -I../include`): refused one
   flag per run — `the flag \`-funroll-loops\` is not one the harness passes …`, then `-W`, then
   `the flag \`-I../include\` names a path outside the project …`. Then
   `flags = ["-O3", "-Isrc/include", "-DLZG_UNSAFE"]` → map exit 0, 1.2 s, the flags shown.
   (Side tries: `-I src/include` with a space is reported as "names a path outside the project";
   `from = "Makefile"` lists the allowed words — good; two `[[configuration]]` entries →
   `parse error in …: … holds several configurations (make, unsafe); pick one with --configuration NAME`.)
6. `harness project ask --target .` (exit 1, 0.0 s):
   `project ask: asking external (claude-sonnet-5) about 1 item(s) in 1 call(s): d1`
   `awaiting response: …/migration/map/traces/59d0b955.response.json`
   `project ask: external provider mode — write each response beside its request under migration/map/traces and re-run: harness project ask --target=. --provider=external --model=claude-sonnet-5`
7. Wrote the reply SCHEMAS' "reply contract" describes, a bare JSON array, into that file →
   `error: parse error in …/59d0b955.response.json: invalid type: map, expected a string at line 1 column 1`.
   Only the testing guide's Step 3.1 `jq` line shows the envelope
   (`{text, input_tokens, output_tokens, stop_reason}`). With it: exit 0,
   `d1 (…; held by t-benchmark, t-unlzg): the model's advice (claude-sonnet-5): keep d1.2 src/lib/decode.c, reason alternative-implementation; in t-benchmark, t-unlzg that choice linked`
   and `wrote migration/map/project-map.reply.json`. No next step after it.
8. Wrong replies: `keep: "d1.3"` → `… does not follow the contract: item \`d1\`: keep \`d1.3\` is not one of its definers (d1.1, d1.2) nor \`undecided\`; nothing was written from it: delete it and answer again`;
   prose before the array → `the reply is not JSON …; delete it and answer again`. Both clear.
9. `harness project ask --target . --build` → awaiting as above; answered with a proposal →
   `wrote migration/map/config.proposed.toml … copy what you accept into migration/map/config.toml and run \`harness project map\` again`.
   The proposal file is the first place the `[[configuration]]` shape appears.
10. `harness project accept t-unlzg --target .` (no pick) → `duplicate set d1 of t-unlzg (LZG_Decode, LZG_DecodedSize) is not settled: pick its definer yourself with --keep d1=<index or path> (its definers: …)`.
    Mistakes are caught well: `--keep d1=decode.c` and `--keep d1=2` → "names no definer of d1: its
    definers are …"; `accept t-lzg --keep d1=d1.2` → "names no duplicate set of t-lzg (t-lzg has none)".
11. `harness project accept t-unlzg --target . --keep d1=d1.2` (exit 0, 1.5 s):
    `project accept t-unlzg: keeping \`src/lib/decode.c\` over \`src/extra/lzgmini.c\` …`
    `project accept: wrote migration/tools/t-unlzg/harness.toml (2 file(s), linked, run as unlzg; configuration make, flags -O3 -Isrc/include -DLZG_UNSAFE); review it with \`git diff\`, then scan it: \`harness scan --target . --tool t-unlzg\``
12. `harness project accept t-lzg --target .` (exit 0, 1.6 s). Then `harness scan --target .` →
    `… has 2 mapped tools and no harness.toml of its own; pick one with --tool (t-lzg, t-unlzg)`.
13. `harness scan --target . --tool t-lzg` (0.0 s, `scan: 6 files, 18 symbols, 42 refs`);
    `harness plan … --tool t-lzg` (4 units). Neither prints a next step.
14. `harness gen-driver u-version --target . --tool t-lzg` → awaiting; answered with the testing
    guide's own version driver through the guide's `jq` envelope → GREEN, promoted (5.5 s).
15. `harness migrate u-version --target . --tool t-lzg` → awaiting; answered by hand (safe
    `logic.rs`, `ffi.rs` per the request's emission contract) → `GREEN … promoted and verified` (3.6 s).
16. `harness verify u-version --target . --tool t-lzg` (exit 0, 1.7 s): six lines, among them
    `verify: [PASS] whole-program — not configured for this target`.
    After adding `[oracle.whole_program] args = ["-9"]` by hand to the tool's harness.toml: eight
    PASS lines (three samples), 3.3 s.
17. `harness state status --target . --tool t-lzg`: `u-version [verified] … verdict=green (fresh)`.
18. Re-accept with nothing changed: `harness project accept t-lzg --target .` → exit 0, "its ledger
    (plan, units, verdicts) is kept" — and `diff` shows my `[oracle.whole_program]` lines gone.
19. Notice: appended a comment to `src/tools/benchmark.c` (not in t-lzg). `state status` before a
    new map: no line. `harness project map` then printed
    `accepted tool t-lzg changed since it was accepted: its files changed since it was accepted (same closure, same configuration, it still links); accept it again with \`harness project accept t-lzg\``
    and `state status` now leads with
    `status: the project changed since this tool was accepted: run \`harness project map\`, then \`accept\` again`.
20. Cockpit, headless (devtools/cockpit-drive/cockpit.py in a pty, 40×140) on a fresh copy:
    `harness-tui: … holds no harness.toml and no tool yet: it is a C project to map. / No map yet. / 1. Map the project`.
    Map dialog: "It runs: harness project map --target …", "It takes: a few seconds …", "It
    writes: …"; `y` ran it (map screen shown inline). Heading then: "Its configuration is a guess: a
    program is accepted under a stated one (write it in migration/map/config.toml, or Ask for a
    proposal)", acts 1–3. **Ask** ran `harness project ask --target …` (no `--build`, since a set is
    held) → `error: the configuration is a guess, so the questions may be wrong: … or pass --allow-guessed`.
    **Accept** let me pick t-unlzg and d1.2 ("1. d1.1 src/extra/lzgmini.c / 2. d1.2 src/lib/decode.c"),
    then was refused for the guess. After writing config.toml from a shell, "Map the project again"
    and Accept t-lzg → the cockpit opened the tool: "Next step: Nothing is scanned yet — press Enter
    and choose Scan the project"; files outside the tool marked ⊖.
21. Cockpit on the two-tool liblzg: chooser `1. t-lzg / 2. t-unlzg` (ids only); picking 1 shows the
    notice in the detail pane and the status bar. The Enter menu on the root has no Map/Accept acts.
22. `harness-mcp --target liblzg` → exit 1, `… has 2 mapped tools …; pick one with --tool`. With
    `--tool t-lzg`: `harness_status` returns `"tool":"t-lzg"`, u-version verified — and `"note": null`:
    no project-changed notice, though state status and the cockpit show one.

## The walk on the zopfli copy (no root harness.toml, `flags = []`)

23. Copied targets/zopfli without `harness.toml` and `migration/`. `harness project map --target .`
    (exit 0, 3.1 s, 16 lines): `p1 t-zopfli_bin — src/zopfli/zopfli_bin.c … guessed libraries: -lm … link check: linked`;
    C++ and Go folders "set aside … not read". Same guess line, same "next … accept" line.
24. `config.toml` with `[[configuration]] name="make" from="make" flags=[]` → map 4.1 s;
    `harness project accept t-zopfli_bin --target . --run-name zopfli` (3.3 s) →
    `wrote … (13 file(s), linked with -lm, run as zopfli; configuration make, no flags)`.
25. `scan` 0.1 s (26 files); `plan` 11 units; `gen-driver u-katajainen --tool t-zopfli_bin` answered
    with the committed M0 driver → GREEN after **35.8 s with no output while it ran**;
    `migrate` answered with the recorded green candidate a-ef81857896e5 → GREEN (5.8 s).
26. `harness verify u-katajainen --target .` (one tool, so `--tool` may be left out; 3.4 s): GREEN,
    again `[PASS] whole-program — not configured for this target` — the committed folder-form zopfli
    has `[oracle.whole_program] args = ["-c"]`, the accepted tool does not.
27. `harness state status --target .` picks the only tool silently (no line says which).
    Cockpit on it opens the tool directly.
28. Variant: the committed copy with its old `migration/` but no harness.toml → `project map`
    refuses (made elsewhere, 11 units, 1 verified; add `--adopt`); with `--adopt` the map, accept
    and `state status` (`no facts — run \`harness scan --tool t-zopfli_bin\``) proceed, and nothing
    says the old 11-unit ledger in `migration/` is now read by nothing.

## Findings

1. **The map's closing hint sends you to `accept`, which refuses a guessed configuration.** (High)
   On both projects the first screen ends "next, … `harness project accept <id>`"; the person
   runs it and is refused. The guess line says only "until that file states the build", with no
   shape and no `ask --build`. Fix: while the configuration is a guess, the closing line names the
   first step — write `migration/map/config.toml` (show the three lines) or run `harness project
   ask --build` — and mentions accept only after that.

2. **`config.toml`'s shape is written down nowhere a newcomer reads.** (High)
   SCHEMAS describes the map file, accept and ask, but never `config.toml`; README and the guide
   do not mention it. It took three tries, led by serde's words ("unknown field `flags`, expected
   `configuration`", "invalid type: map, expected a sequence"). Only `config.proposed.toml`, after a
   model call, shows `[[configuration]]`. Fix: a SCHEMAS section for `config.toml` (fields, the
   `from` words, `stated` for a hand-written one, the flag grammar link), and the first map writes a
   commented example (or prints it) beside the guess line.

3. **Accepting a tool again silently drops what the person added to its `harness.toml`.** (High)
   The accepted file has no whole-program check, so the person adds `[oracle.whole_program]` (the
   only way to get the guide's eight checks); a re-accept — which the notice asks for — rewrites the
   file and the section is gone, with "its ledger is kept" as the only words. Without git (a
   downloaded tarball) "review it with `git diff`" shows nothing. Fix: keep the sections accept does
   not own (`[oracle.whole_program]`, `[llm] model`, `[driver]`, extra allowlist), or refuse naming
   the lines it would drop.

4. **A mapped tool verifies with no whole-program run, and verify calls that a PASS.** (High)
   Both tools printed `[PASS] whole-program — not configured for this target`; a newcomer reads six
   PASS lines as "fully checked". Nothing in accept's output or the written file says the check is
   off or how to turn it on (`args`). Fix: accept writes a commented `[oracle.whole_program]` with a
   sentence, its closing line says the check is off; verify prints it as "not run", not PASS.

5. **The notice asks for a re-accept when only a file outside the tool changed.** (Medium)
   Editing `src/tools/benchmark.c` (not in t-lzg) gave "accepted tool t-lzg changed … its files
   changed since it was accepted (same closure, same configuration, it still links); accept it
   again". t-lzg's files did not change; combined with finding 3 the advice costs the person their
   edits. `state status` then says "run `harness project map`, then `accept` again" although the map
   was just run. Fix: when closure, configuration and link are unchanged, say "a file elsewhere in
   the project changed; nothing to do for this tool" (or re-bind silently), and have state status
   name the exact accept command when the map is current.

6. **The hand-off response's envelope is not on screen nor in SCHEMAS' ask section.** (Medium)
   The awaiting lines say "write each response beside its request"; SCHEMAS' reply contract shows
   the bare JSON array. Writing that array gives "invalid type: map, expected a string at line 1
   column 1" (it was an array, and the sentence names no fix). The envelope is only in the testing
   guide's Step 3.1 `jq` line. Fix: the awaiting line (all three commands) names the envelope
   `{"text": <the reply>, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}`, and a
   non-envelope file is refused with that sentence.

7. **In the cockpit, a guessed configuration with a held set is a dead end.** (Medium)
   The heading says "or Ask for a proposal", but Ask runs the held-set question, which is refused
   ("pass --allow-guessed" — a flag the cockpit cannot pass). Accept then walks through two pickers
   and is refused for the guess the heading already knew. Fix: while the configuration is a guess,
   Ask runs `--build` (or offers both questions) and Accept says before the pickers that a stated
   configuration is needed.

8. **The cockpit's Ask dialog promises advice "shown beside the choice"; Accept never shows it.** (Medium)
   project.rs reads no reply file; the definer list is bare. Fix: show the reply's advice, labelled
   as the model's, beside the definers (none preselected), or change the dialog's sentence.

9. **Once one tool exists the cockpit has no project acts.** (Medium)
   The notice says "accept again", the person wants a second program, but the root's menu offers
   only the tool's acts and the chooser lists bare ids. Fix: "Map the project again" and "Accept a
   program" in the root's menu once any map exists; the chooser shows each tool's program path.

10. **`config.toml` errors mislead and come one at a time.** (Medium)
    Every error starts "parse error in <full path>:", also for "holds several configurations; pick
    one with --configuration" and a name rule. `-I src/include` (a space) is called "a path outside
    the project". The Makefile's `-funroll-loops -W -Wall` cost one run each. Fix: start with
    "migration/map/config.toml:", list every refused flag in one message, say warning and tuning
    flags can be dropped, and name a space after `-I` as "write it joined: -Isrc/include".

11. **Hand answers are labelled `claude-sonnet-5`, authored by the pipeline.** (Low)
    The accepted harness.toml has no `model`, so the default lands in every attempt and in the
    ask reply, and the resume line prints `--model=claude-sonnet-5`. The testing guide's folder form
    sets `model = "my-claude-code"` for this reason. Fix: accept writes a commented `model =` line,
    and the external awaiting line says to pass `--model` naming whoever answers.

12. **harness-mcp's status carries no project-changed notice; README's `.mcp.json` lacks `--tool`.** (Low)
    A chat agent on a tool never learns the project changed. Fix: the notice in `harness_status`'s
    `note`, and the README example shows `"--tool", "t-…"`.

13. **Small silences.** (Low) `harness project --help` points at the design document and gives no
    order; scan and plan print no next step; gen-driver ran 36 s with no line; `state status` on a
    one-tool project never says which tool it read; `-O3` is listed among the flags with no word
    that `-O` is recorded only; after `map --adopt` on a folder holding an old folder-form ledger,
    nothing says that ledger is no longer read. Fix: one line each.

## Docs for step (f)

- **README**: a "Start from your own C project" section before the command reference: map →
  state the configuration (three-line `config.toml`, or `ask --build`) → map again → accept (with
  `--keep` when a choice is held, `--run-name`) → scan/plan/gen-driver/migrate/verify with
  `--tool`. The flags table's `--target` row ("the folder containing harness.toml") gains
  `--tool`. The harness-mcp `.mcp.json` example with `--tool`. Exit codes unchanged.
- **Tutorial**: one plain chapter: what a program, a shared file, a held choice and a
  configuration are; why the link check does not prove the right file; where things live
  (`migration/map/`, `migration/tools/<id>/`); that a tool's `harness.toml` is the acceptance and
  what re-accepting rewrites; the cockpit's three dialogs.
- **Testing guide**: a Part 1 alternative (or Part 12) on liblzg by map: the exact first screen;
  writing `config.toml` (with `-Isrc/include`, why `-W`/`-Wall` are left out); `ask` answered by
  hand with the envelope; `accept t-unlzg --keep d1=d1.2` and `accept t-lzg`; adding
  `[oracle.whole_program] args = ["-9"]` so verify shows eight checks; the notice after a change;
  the cockpit on a folder with no tool. Troubleshooting rows: "the configuration is a guess",
  "has 2 mapped tools", "names no definer of d1", the envelope error.
- **SCHEMAS**: a `config.toml` section; the hand-off envelope stated once under Global rules and
  linked from ask, gen-driver and migrate.

## Holds

- The map is fast (1.2–4.1 s here) and its words are plain: the held set, the link check's limits
  once per screen, set-aside folders, shared files and "defined in two programs' files that never
  meet".
- accept's refusals are one clear sentence each, with the fix (`--keep` errors list the definers;
  the guess refusal names `ask --build`); the pick is printed in words before writing.
- ask's contract errors are precise and end "delete it and answer again"; `ask --build` writes a
  proposal file that explains how to use it.
- `--tool` works across scan, plan, gen-driver, migrate, verify, state status, the cockpit and
  harness-mcp; several tools are refused naming them, a single tool is picked.
- A real `-D` (`-DLZG_UNSAFE`) and an include folder (`-Isrc/include`) flow from `config.toml` into
  the tool's `harness.toml` and through driver validation, migrate and verify.
- The cockpit's dialogs show the exact command, how long it takes and what it writes, and run only
  on `y`; after accept it opens the tool with a correct next step and greys files outside it; the
  notice shows in the detail pane and the status bar.


---

# Triage (the session, 2026-10-08)

The headline is evidence: a checker who had never seen the design reached a verified Rust unit on
liblzg and on zopfli from a cold start, by the docs and the screens alone, and the zopfli tool
accepted from the map is byte-identical to the hand-written target. Nothing high was found by the
security lens. What follows decides every finding and groups the work into two fix passes by file
ownership; step (f)'s docs are folded into the second, since the flow report wrote their outline.

## Decisions

1. **`accept` never takes a choice, a file list or a program path from the map file.** After the
   re-map it recomputes the closures and the duplicate sets from the fresh map; the map file only
   turns the person's indexes into paths, and `accept` refuses when its ids, paths or definer
   sets differ from the fresh ones. A set is held unless re-linking shows exactly one choice links.
2. **Re-accepting keeps what the person added.** `accept` owns `[target]`'s files, folders,
   configuration, map stamp, picks and run name, `[oracle] extra_link_args` and the allowlist's
   fixed entries; every other key or section (`[oracle.whole_program]`, `[llm] model`, `[driver]`,
   extra allowlist entries) is carried over unchanged, and the closing line names what was kept.
3. **A check that did not run is never a PASS on screen.** `verify` prints an unconfigured
   whole-program check as "not run: not configured for this target (add [oracle.whole_program]
   args = [...] to harness.toml)"; the recorded verdict's bytes do not change (committed verdicts
   and the bench replay stay identical). `accept` writes a commented `[oracle.whole_program]`
   example and says the check is off until it is filled in.
4. **While the configuration is a guess, the map's closing line shows the three lines of
   `config.toml` and names `ask --build`; `accept` is named only once the configuration is
   stated.** The map writes no example file; SCHEMAS gets a `config.toml` section.
5. **The hand-off envelope is stated once** in SCHEMAS' global rules and named on every awaiting
   line (`ask`, `gen-driver`, `migrate`, `observe`); a response that is not the envelope is
   refused with that sentence; a truncated answer under `external` names the file and says
   "delete it and answer again".
6. **`ask` says what the map did**: a set the map did not link (over the limit) reads "the map did
   not link these choices (too many to try)" on both screens, never "did not link"; a guessed map
   with nothing held is told `--build`; "every program linked" only when true; a driver is "only a
   main program is asked about".
7. **The cockpit's acts follow the open question**: while the configuration is a guess, Ask runs
   `--build` and Accept says before any picker that a stated configuration is needed; once any
   map exists the root's menu keeps "Map the project again" and "Accept a program"; the chooser
   shows each tool's program path; Accept's picker shows the reply's advice beside the definers,
   labelled as the model's, none preselected, and skips a set the chosen pick does not reach;
   its set and index strings go through `safe_line` after the shape checks `ask` already has.
8. **Library ids follow the files**: an accepted library keeps its id for the library that holds
   any of its listed files (the most when several), and when the id still moves the line names
   the new id; a library whose needed symbol may be defined in a file that did not compile is
   incomplete and refused; `--run-name` is refused for a library.
9. **"What changed" says which digest moved** ("the project's files changed" / "the configuration
   or the compiler changed"), says "a file elsewhere in the project changed; nothing to do for
   this tool" when closure, configuration and link are unchanged, and the map file records per
   accepted tool what changed so `state status`, the cockpit and harness-mcp's note read it from
   there instead of comparing digests; programs not accepted as tools are listed on every map
   while any tool exists.
10. **Bounds on what `ask` sends**: 16 KiB per definer slice, lines cut at 400 characters and
    said so, 256 KiB per request, a definition search that reads each line once; a kept reply
    item is re-validated when merged; repeated JSON keys and names without a base character are
    refused (the "fixed field order" sentence leaves the design: order is not checked).
11. **The person's `config.toml` errors**: lead with the relative path, every refused flag in one
    message, warning and tuning flags named as droppable, `-I src/include` told to write it
    joined.
12. **Hand answers are named**: `accept` writes a commented `model =` line; the external
    awaiting line says to pass `--model` naming who answers.
13. **The small silences**: `harness project --help` gives the order; scan and plan print a next
    step; gen-driver prints a progress line; `state status` names the tool it read when it picked
    the only one; the map notes `-O` is recorded only; `map --adopt` over an old folder-form ledger
    says that ledger is no longer read; harness-mcp's status note carries the notice; the README's
    `.mcp.json` example shows `--tool`; `accept` dropped picks say so.
14. **`accept` checks that `migration/` and `migration/tools/` are real folders before any lock.**

## Fix pass G — the map, ask and accept (harness-oracle `projectmap/*`, harness-llm `projectask.rs`, harness-cli `project.rs`/`project_ask.rs`/`project_accept.rs`, harness-tui `project.rs`)

Decisions 1, 2, 3 (the `accept` half), 4, 6, 7, 8, 9, 10, 11, 12 (the `accept` and `ask` halves),
13's map and accept lines, 14. Tests the correctness lens named as missing: each `accept` refusal
through the binary, a nested-set accept, an over-limit set at `ask`, a duplicate-key reply, and a
re-accept keeping a hand-added section.

## Fix pass H — the person's side and step (f)'s docs (harness-cli except the project files, harness-oracle's verify screen, harness-mcp, harness-llm's awaiting lines, README, docs/TUTORIAL*, docs/TESTING-GUIDE.md, docs/SCHEMAS.md, docs/PROJECT-MAP-DESIGN.md §9)

Decisions 3 (the `verify` half), 5, 12 (the gen-driver/migrate/observe half), 13's other lines;
the docs outline in the flow report ("Docs for step (f)"): README's "Start from your own C
project", the tutorial's chapter, the testing guide's Part 12 on liblzg by map, SCHEMAS'
`config.toml` section and the envelope; the design's §9 entry for this round.

## Recorded, not fixed now

- The whole-program check's absence in an accepted tool is a wording and a hint, not an
  automatic configuration: the harness cannot guess a program's arguments.
- Adoption of a shipped map, reply or response file is one yes; after it the download's files
  are the person's (the security lens's framing). The design says so already.
