# Check round over the map's step (b) and the fix passes of steps (a) and (c) — 2026-10-08

Four Opus checkers at high effort over main at 1cded59, each in its own worktree with experiments: fix passes A and B re-verified; fix pass C and the whole flow; step (b) for correctness; step (b) for security. Their reports follow verbatim; the triage is at the end.

---

# Fix passes A and B: did they fix what the review found, and what did they bring?

Worktree at 1cded59 (the fast-forward was already up to date; the head 0a1d2e8 named in the brief
does not exist in this repository, so everything below is at 1cded59). Read: the briefing §10–§12,
the review's oracle and readers reports with the triage, the design's §3.7 and §9, DECISIONS' last
entry, and `git diff 0d8e250..HEAD` over the named crates (the project-map files left out).
Experiments ran under the scratchpad with `RUHARNESS_ADOPTED` pointed at a scratch file. Temporary
test files and the reverts were removed; `git status` is clean. Nothing was committed, downloaded
or installed (an older binary was built from `git archive 0d8e250` in the scratchpad, offline).
The whole workspace test suite passes (42 test binaries), and `cargo fmt --check` is clean.

## Medium

**1. The include reader's new `<` rule takes time that grows with the square of a line's length,
and every command on a file-list tool pays it.**
Evidence: `crates/harness-core/src/sources.rs:669-670`. For every `<` byte, `strip_comments`
searches back through everything written so far for the line's start (`out.iter().rposition`). A
long line with many `<` therefore costs about the line's length squared. The scanner reads every
file with it (`harness-scan/src/lib.rs:274`), and so does the staleness walk run by `plan`,
`detect`, `state status` and the cockpit's read model (`names_on_disk`, `sources.rs:583`).
Ran it: a tool whose one file is a single 420 KB line of `x=a<b;` took 4.1 s to scan. At 840 KB,
`scan` took 16.0 s and `plan` took 15.6 s. The binary built from 0d8e250 runs the same `plan`
instantly. At the 8 MiB cap, that rate gives about 25 minutes for every command.
Failure: one generated or hostile file makes every command on the tool hang for minutes. The
target is untrusted input.
Fix: keep the current line's start (or an "after `#include`" flag) as a running value while
copying, instead of searching back for it. Add a timing test with a 1 MiB line.

**2. The features mirror now refuses any program that holds a file over 8 MiB.**
Evidence: `crates/harness-oracle/src/featuremap.rs:1347` reads every mirrored file through
`read_regular(path, MAX_SOURCE_BYTES)`. The scanner records such a file (hashed, not parsed), and
the compile builds it. Before fix pass B the mirror read it whole and checked only the 256 MiB
total.
Ran it: a tool whose `-include` chain reaches a header of 8 MiB + 1 byte. `map_features` fails with
"the features map copies only regular files of at most 8 MiB … longer than 8388608 bytes".
Failure: a program that includes a big generated table, or SQLite's single-file `sqlite3.c`
(about 9 MB), can never have its features mapped.
Fix: keep the regular-file check, and give the mirror a per-file cap of its own well above the
scanner's, such as the map's 64 MiB, under the existing 256 MiB total.

**3. A manifest written with `[project]` instead of `[package]` gets a build script past the
unit-crate check.**
Evidence: `crates/harness-oracle/src/unit_crate.rs:146` checks `build`, `links` and `workspace`
only under `package`. Cargo 1.94 still reads the old `[project]` table, with a deprecation
warning. The folder rule allows `src/*.rs`.
Ran it: `[project] build = "src/b.rs"` with an empty `[workspace]`. `manifest_problem` returns
`None` and `check_unit_crate` returns `Ok(())`. A real offline `cargo build` of that crate printed
"BUILD SCRIPT RAN". The gap predates fix pass B, but closing this check's gaps was that pass's job.
Fix: refuse a top-level `project` key, or check it exactly as `package` is checked. Add the case to
`a_manifest_that_could_run_or_fetch_code_is_refused_by_key`.

**4. A header that an ambiguous include lands on is in no unit's closure. For a hand-written tool,
editing it never makes a verdict stale.**
Evidence: `sources.rs:562` records no edge for an ambiguous name. `compute_inputs`
(`harness-oracle/src/lib.rs:203`), the planner (`planner.rs:111`) and status (`status.rs:318`)
hash only the facts' closure. The oracle's own compile closure (`include_rule::closure`) does
include that header.
Ran it: `inc/common.h` includes `"cfg.h"` and is reached from `a.c` (folders `inc, d1`) and `b.c`
(folders `inc, d2`). `cc -M` reads `d1/cfg.h` for `a.c`. The facts give `a.c` only
`[inc/common.h]`. Editing `d1/cfg.h` makes the facts stale. After a rescan, though, the unit's
`unit_source` is unchanged, so the verdict reads "green (fresh)".
DECISIONS records this as not fixed because `accept` refuses an unsettled ambiguity. A
hand-written tool never goes through `accept`, and only `scan` mentions the ambiguity, once.
Failure: a verdict built from `d1/cfg.h` stays fresh after `d1/cfg.h` changes a layout.
Fix: hash each unit's compile closure from the resolver (move `include_rule::closure` into
harness-core and use it in `compute_inputs`, `source_hash` and status). Or have `verify` and
`plan` refuse a unit whose closure crosses an ambiguous include, naming it.

## Low

**5. A folder given both as `-I` and as `-isystem` is searched where the resolver does not
search it.**
Evidence: `search_order` (`sources.rs:435-455`) keeps both entries. clang and gcc drop the `-I`
form of a folder that is also a system folder, and search it at the `-isystem` position.
Ran it: configuration `-Ia -isystema`, file folders `b`, with both `a/x.h` and `b/x.h`. The scan
records `a/x.h` and `a/y.h`, while `cc -M` in the oracle's argument order reads `b/x.h` and `b/y.h`.
The same happens when a file's `include_dirs` repeats a `-isystem` folder.
Fix: in `search_order`, leave out of the `-I` and `include_dirs` lists any folder that `-isystem`
names. Add the case to `the_rule_reads_what_cc_reads`.

**6. Five forms the compiler honours are read by no reader.**
Ran it: crafted files were checked against `cc -M` and against the reader. For each of these, cc
reads the header and `include_names` returns nothing: `#import "x.h"`, `#include_next <x.h>`
(reaching a later project folder), the digraph `%:include`, lines that end in a lone `\r` (old Mac
files), and a backslash followed by a space before the newline. The module doc says the reader
reads "as the preprocessor does".
Fix: read `import` and `include_next` as includes (`include_next` resolved as an ordinary include
over-approximates, which is safe), treat a lone `\r` as a line end, and treat `\` plus blanks plus
a newline as a splice. Or name the remaining ones as limits in the module doc.

**7. A header in a dot-folder is scanned and compiled, but every prompt refuses it, with a wrong
instruction.**
Evidence: `find` (`sources.rs:459`) lands in a dot-folder, while `read_sources`
(`harness-llm/src/trajectory.rs:1972`) refuses any path in a dot-folder.
Ran it: `src/main.c` includes `"../.gen/config.h"`. The scan records `.gen/config.h`, and `plan`
and `detect` pass. `gen-driver` refuses with "lies in a folder whose name starts with a dot, which
no scan reads … (re-run `harness scan`)". A rescan cannot help, so the unit can never be migrated.
Fix: let the prompt accept what the resolver reaches, since the scope check already confines it.
Or have `find` treat a dot-folder landing as outside the project, with a scan note.

**8. The configuration entry moves more than the configuration does.**
Evidence: `configuration_entry` (`status.rs:52`) hashes the raw flags and every listed file with
its folders.
By reading the code: adding one file to a tool makes every unit's verdict stale. A trailing `/` on
`-Ix/` does too, although the program digest treats it as cosmetic (the readers' finding 10).
Fix: hash the resolved forms (`Resolver::resolved_flags`, `Resolver::listed`), and only the
folders of the unit's own files plus the flags.

**9. The shared include rule has no `-idirafter`, which the map uses for `system_headers`.**
Evidence: the map compiles a folder that holds a name listed in `system_headers` with
`-idirafter` (`harness-oracle/src/projectmap.rs:1088-1096`). The tool grammar and `Resolver` know
only `-I`, `-iquote`, `-isystem` and `-include`. Today a change of `system_headers` correctly never
stales a tool, because no tool can carry it.
Failure (when `accept` is built): a tool written from such a map compiles `compat/unistd.h` where
the map settled on the system's header.
Fix: before `accept`, add `-idirafter` to the grammar and to the resolver (after `-isystem` and
after the system), or have `accept` refuse such a folder.

**10. Detect's skip note is not the scan's.**
Ran it (e5): the scan says "cannot be read: Permission denied (os error 13); recorded as
unreadable". Detect says "cannot be read: io error at /full/machine/path: Permission denied".
Fix: build both notes from one helper with the root-relative path.

**11. (Older than these passes, seen in passing.) A deeply nested expression aborts the scanner.**
Ran it: a 400 KB line `int x = 0 < 1 < 1 …;` overflows the stack ("fatal runtime error: stack
overflow"), with the 0d8e250 binary too. Fix: walk tree-sitter's tree with a stack instead of
recursion, or cap the depth.

## Closed (each review experiment re-run)

- Readers e1 (the configuration's `-I` and `-include`): the scan records `src/main.c ->
  a/forced.h, a/x.h, a/y.h` and `a/forced.h -> a/fh.h`, matching `cc -M`. An edit to each of the
  four files makes `plan` refuse as stale.
- Readers e3 (X-macro array): after a scan, `plan` and `detect` run, and the facts are fresh.
- Readers e5 (unreadable file): the folder form and the file list both record `unreadable`, plan
  proceeds, and detect skips the file with a note and exits 0. Once the file is readable again,
  plan honestly says stale.
- Readers e6 (a file's folders change): `plan` refuses as stale, and a scan clears it.
- Readers 7 (an include outside the project or under `migration/`): two scan notes, nothing read.
  Readers 11 (a linked `source_dir`): fresh after a scan. Readers 9 and 10: checked by reading the
  code and by the revert below.
- Oracle `-std=c89`: `tests/file_list.rs` is green with it. Reverting the runtimes' own arguments
  brings back the guard's "redefinition of 'j'".
- Oracle, a configuration `-I` against the facts: a temporary tool used only `-Iinc
  -includecfg/forced.h -std=c89` (no file folders). `verify` was green on all 8 checks;
  `unit_source` moved when `cfg/wide.h` (reached through the `-include`) was edited; the features
  map built, and the mirror held `cfg/wide.h`. So the `configuration` entry rightly leaves
  `-include` contents out: `unit_source` covers them.
- Oracle mutant header, absolute symbol and FIFO `-include`: each test passes, and each revert
  below fails. The loader refuses a FIFO `-include`, and the mirror refuses one that appears after
  load.
- Oracle 3 (configuration in the verdict), 6 (64 MiB driver object), 7 (name budget), 8 (except
  finding 3 here) and 10 (no `.obj` folders were left after the zopfli verify).

## Tests (ten real reverts, each restored)

Each revert below made the named tests fail:
- The mutant's `-iquote` back to `-I`: `a_mutant_reads_its_originals_headers`.
- The `<name>` rule removed: `include_names_reads_what_the_preprocessor_reads` and
  `the_scanner_and_staleness_read_the_same_includes`.
- Status no longer reporting `configuration`: `a_changed_configuration_makes_a_file_list_verdict_stale`.
- The runtimes given the unit's arguments: `the_configuration_reaches_the_driver_build_and_the_boundary_check`.
- Mach-O absolute symbols not reported: `the_object_is_read_without_nm_and_odd_names_are_refused`.
- The two-way staleness check removed: `staleness_compares_where_each_include_lands_with_the_record`.
- A missing `[workspace]` accepted: `a_manifest_that_could_run_or_fetch_code_is_refused_by_key`.
- Ambiguity detection removed: `a_header_resolving_differently_under_two_files_is_an_ambiguous_include`.
- Raw forms hashed in the digest: `the_v2_digest_hashes_resolved_forms_and_moves_with_the_link_arguments`.
- `-I` and file folders swapped in `search_order`: `the_rule_reads_what_cc_reads` and
  `the_configurations_path_flags_reach_the_scan_in_the_compilers_order`.

Weak spots: the `cc -M` half of the last test is skipped silently when `cc` fails
(`harness-scan/src/lib.rs:2841`); it should fail instead. No test covers findings 1–3 or 5–7.

## Holds

- The 101 committed `facts.jsonl`: a real `harness scan --adopt` of a fresh copy of each root
  produced all 101 byte for byte.
- zopfli: `state status` says `features=current`, so its v1 program digest holds. A real
  `harness verify u001-katajainen` on an adopted copy wrote an `oracle-latest.json` byte-identical
  to the committed one (zopfli opts out of the boundary check, so the wrapper's version 4 does not
  touch it).
- The lexical reader against tree-sitter on 494 files (the 415 under `targets/`, plus lz4 and
  liblzg): no difference.
- Crafted cases where both readers and cc agree: an include inside a string, inside `#if 0`
  (read, which over-approximates safely), a macro-built name (read by neither), and a trailing
  `\r`.
- The resolver against `cc -M`: a header reached through `../` and through a folder link; a
  `-include` whose own folder holds a same-named header (`f/x.h` is taken over `-Ia`'s);
  `<stdio.h>` with `-Iinc`, `<stdlib.h>` with `-isystem`, `<string.h>` not taken from `-iquote`,
  and `"errno.h"` taken from `-iquote`; on the case-insensitive disk, `"X.H"` is recorded as
  `inc/x.h`, the on-disk name.
- Staleness: a header added to an `-iquote` folder stales a quoted include and leaves an angle
  include alone. Edits behind `-include` stale.
- objsyms: 453 real objects were read with no refusal from the name budget or the section count
  (lz4, liblzg and zopfli C at `-O2` and at `-g -O0`, and every member of katajainen's Rust
  staticlib).
- The unit-crate check: the 194 committed unit manifests and zopfli's crate pass.
- The loader: a 3.0 MB `harness.toml` (12 000 files, 12 folders each) loads in 3 s. A linked or
  FIFO root file is refused before it is read.
- The cost of the include walk is not new: 2 000 files, each with its own folder list, take 35 s
  per `plan` both before and after the passes.


---

# Fix pass C, end to end: what a person meets now (check, 2026-10-08)

Checked at 1cded59, the head of `claude/ruharness-resume-ee312a` (the fast-forward found nothing new; the
0a1d2e8 the brief names does not exist in this repository). Binaries built from the worktree. Every
run used `RUHARNESS_ADOPTED` pointed at a scratch file (tests and mutants at another), so the
person's adoption file was never touched. Fixtures under the scratchpad's `check2/flow/`: `z1`, `z2`,
`z3` (copies of targets/zopfli, no token file), `lzg` (the liblzg shape from
crates/harness-cli/tests/file_list.rs, one tool), `two` (the same plus `t-other`, which lists only
`src/other/decode.c`), `pair` (the `t-pair` tool of file_list.rs, wired and verified green, plus a
`t-other`), and small folders (`own`, `empty`, `ff`, `tok`). Nothing downloaded, nothing committed;
the worktree is clean (`git status` empty) after every revert.

## Findings

1. **High — `project map`'s closing next step names two commands that do not exist.**
   `harness project map --target lzg` ends: "next, `harness project ask --target …/lzg` asks a model
   for advice on held choices, or `harness project accept <id> --target …/lzg` makes one of them a
   tool". Run as printed: "error: unrecognized subcommand 'ask'" and "… 'accept'", exit 2
   (`harness project --help` lists only `map`). The same dead end sits in project.rs:475 ("pick one
   with `harness project accept …`") and in the notice (ledger.rs:415, "…then `accept` again"). It
   also offers `ask` "for held choices" on a map that holds none. What the person meets: the first
   screen of the map, the new front door, ends on two errors. Fix: until steps (d)/(e) land, end on
   what the map is for today ("nothing to accept yet; a tool is written by hand under
   migration/tools/<id>/harness.toml"), and name `ask` only when a choice is held. (project.rs is
   the other checker's file; it is listed here because it is the flow's first hint.)

2. **Medium — harness-mcp's adoption sentence leads the person into an error in a two-tool project.**
   harness-mcp `--target twoy --tool t-lzg` (unadopted): "…ask the person to adopt it (`harness
   state status --adopt --target …/twoy`, or the cockpit's question); an agent never adopts". Run as
   printed: "adopt: … is now trusted…", then "error: … has 2 mapped tools and no harness.toml of its
   own; pick one with --tool (t-lzg, t-other)", exit 1. The sentence is built without the tool
   (crates/harness-core/src/adopt.rs:190). What the person meets: the adoption worked, but the last
   line says error. Fix: spell the server's `--tool <id>` in the sentence (through
   `runtime_view::command_line`).

3. **Medium — harness-mcp's status `note` still hands an agent bare commands.** `harness_status` on
   `pair --tool t-other` (not scanned): `"note":{"text":"no facts — run \`harness scan\`"}`; on `two
   --tool t-other` with its plan removed: `"no plan — run \`harness plan\`"`. Both refuse in that
   project ("has 2 mapped tools … pick one with --tool"). Source: crates/harness-tui/src/model.rs:262
   and :336 (`NO_PLAN`), which harness-mcp reads. An agent copies commands literally (the triage's
   own reason for decision 9). Fix: build both notes with `command_line` and the snapshot's tool.

4. **Medium — the hints fix pass C left without `--tool`, exactly.** Each is a next step a person or
   agent can see while a tool is open; none goes through `runtime_view::command_line`:
   - cockpit: app.rs:4831 "facts predate {file} — run \`harness scan\`"; files.rs:170 "Re-check it
     (\`harness state status\` explains)"; main.rs:1161 "(\`harness state status\` shows it)";
     model.rs:262 and :336 (finding 3); speed.rs:168 "then run harness verify {id} in a terminal";
     speed.rs:393 "harness perf run --unit {id} --workload {w}"; speed.rs:944 and :947 "no facts — run
     harness scan"; the help's view.rs:3046 "run harness verify <unit>" and :3120 "harness migrate
     <unit>" (general help; acceptable if it adds "and --tool <id> on a tool").
   - harness-core: perf/mod.rs:68 "Re-check {unit} (or run harness verify {unit})" (also the
     cockpit's interrupted-Accept cause); perf/words.rs:684 and :716 "run harness verify {first}";
     observer.rs:531 "run \`harness review\`" (in a tool's observations.md).
   - harness-llm: trajectory.rs:1946, 1964, 2000 "re-run \`harness scan\`"; migrate.rs:880
     "(\`harness gen-driver\`)"; migrate.rs:1039 and triage.rs:103, 309 "\`harness detect\`".
   - harness-oracle: lib.rs:1726 "run \`harness scan\` first"; perf/measure.rs:773 "run harness scan
     first, then measure".
   Fix: one pass that hands the open tool (or the ledger's relative path, from which `tool_of`
   reads it) to these and spells every one through `command_line`.

5. **Medium — `verify` and `promote` name the folder-form path for a tool's broken features file.**
   `harness verify u-pair --tool t-pair` with `bogus = 3` in the tool's features.toml: "verify: your
   features file has an error — no feature scenario runs (migration/features/features.toml: unknown
   key "bogus" …)". The file to fix is migration/tools/t-pair/features/features.toml; `features map`,
   the cockpit's Features view and `perf` all say so (through `error::in_ledger`), but
   `announce_features` (crates/harness-cli/src/main.rs:1052, also promote.rs:459) prints
   `FeatureSnapshot::Invalid` raw, built by `features::invalid()` (harness-core features.rs:245) and
   features.rs:755-764 from `MIGRATION_DIR`. Fix: `invalid()` and `FeatureSnapshot::load` take the
   ledger's relative path (`load` already holds the context); then `in_ledger` at the edges can go.

6. **Medium — "the verdicts are claims" is said, then nothing shows a claim.** After `harness state
   status --target z1 --adopt`: "adopt: deleted 0 build folder(s) made elsewhere; the verdicts are
   claims made elsewhere until `harness verify` runs them here", and two lines below
   "u001-katajainen [verified] plan=fresh verdict=green (fresh)". The cockpit's `✓?` means "origin
   not recorded", not "made elsewhere". On `twoy` and `two2` (4 units, 0 verified, nothing deleted)
   the same line still speaks of verdicts and "deleted 0", because the condition is `units > 0`
   (adopt.rs:155). Fix: print the claims half only when `verified > 0` and the deletion half only
   when something was deleted; then either mark adopted verdicts until re-verified or drop "stay
   claims" from the help and dialog (the review's ninth point is half done).

7. **Low — SCHEMAS still describes the old adoption.** docs/SCHEMAS.md:1520 (".ruharness-adopted …
   committed for RuHarness's fixtures"), :1540-1542 ("Adopting a root whose token file already
   holds a well-formed token records that token and writes none" — the code writes a fresh one,
   adopt.rs:639-644), :1545-1549 (the sentence has no folder prefix, and "harness-mcp refuses with
   the same sentence" — it now says "ask the person … an agent never adopts", adopt.rs:182-195),
   :1553-1558 (omits the tools' build folders; "says the verdicts are claims" is now conditional),
   :1576 ("when the root has no well-formed token yet"), and no word of the results-free
   `migration/` (decision 7). The "of its own" sentence at :1507 matches the code. Fix: step (f) as
   planned, with these lines.

8. **Low — the token and the lock show up in `git status` for a project nobody mapped.** `harness
   scan --target ff` (folder form): `ff/migration/` holds `.lock` and `.ruharness-adopted` and no
   `.gitignore`; only `project map` writes one. The design calls the token git-ignored. Fix: write
   the same `.gitignore` when a command first creates a ledger.

9. **Low — hints leave out `--target`.** From `flow/`, `harness state status --target lzg` says "run
   \`harness scan --tool t-lzg\`"; run there: "error: . is not a harness target …". Inside the
   project it works. The resume lines carry `--target=.`. Fix: add `--target <as given>` when it is
   not the current folder.

10. **Low — the tree loses the name of a selected outside file, and marks unscanned headers as the
    tool's.** Selecting `unused.h` under `--tool t-lzg` shows the row "⊖ … not part of this …" (the
    name gone; the View title has it whole). Before `t-other`'s first scan its tree shows "+ lzg.h",
    "+ unused.h", "+ internal.h" (rolled up "+4") though decode.c includes none; after the scan
    they read ⊖. Fix: keep the name on the row (the title already says the rest); show headers
    neutrally until a scan says which are reached.

11. **Low — the cockpit's adoption dialog lists only the folder form's build folders**
    ("migration/build/, each unit crate's target/…", crates/harness-tui/src/adoption.rs:18-20),
    while adoption also deletes each tool's (adopt.rs:757-763). Fix: "…in the project's ledger and
    in each tool's under migration/tools/".

12. **Low — "once per computer" is once per checkout** (adoption is by canonical path, so each
    worktree's zopfli and tractor are asked again); DECISIONS and NEXT-SESSION say "per computer".

13. **Low — devtools/cockpit-drive/cockpit.py:38-39 drops `RUHARNESS_ADOPTED`**, so a checker
    driving the cockpit with it adopts into the person's real file. Fix: pass it through when set.

14. **Low — sentences that are not one plain sentence with a next step:**
    - `harness features map --tool t-lzg` on the starter: "error: your features file has no
      scenario to map" (no path, nothing to do; say "add a [[scenario]] to
      migration/tools/t-lzg/features/features.toml").
    - `harness verify u-checksum --tool t-lzg`: "error: unit \`u-checksum\` has no [unit.oracle]
      configured" (no step; `gen-driver` is it).
    - `harness sync-runtime --tool t-other` before a scan: "error: loading facts (run \`harness scan
      --tool t-other\` first): io error at …/facts.jsonl: No such file or directory (os error 2): No
      such file or directory (os error 2)".
    - `observe` and `gen-driver` in hand-off mode: "supply the response file(s) under … and re-run"
      — no command, while `migrate` prints "re-run: harness migrate u-pair --target=. --tool=t-pair".
    - `harness perf show --tool t-lzg` with a broken workloads file: the error, then "nothing
      measured yet — run harness perf run --tool t-lzg", which refuses with the same error.
    - `project map`: "configuration: guessed (guessed), flags none".
    - `harness scan --target own`: "this project has a migration/ folder of its own; move or rename
      it, or map a copy" — "map a copy" on a scan, and no path (the cockpit names it).
    - the cockpit, declining adoption: "not adopted; nothing was changed" — no way back named;
      `sync-runtime` says "AGENTS.md updated" when `--check` said up to date; "1 files scanned".

## Closed (the review's findings, re-run)

- **A project's own `migration/` first told to adopt** — closed. `own` (migration/001_init.sql):
  `project map`, `project map --adopt`, `scan`, the cockpit and harness-mcp all say "this project has
  a migration/ folder of its own…" first.
- **A hand-written tool refused as results made elsewhere** — closed. `harness scan --target lzg`
  scans; the adoption file records `how = "created"`; no adoption line. A shipped token in such a
  folder (`tok`) is replaced on the first scan.
- **An agent told to adopt** — closed. harness-mcp says "ask the person … an agent never adopts"
  (its wording now has finding 2's gap). Every block carries the line; targets/zopfli/AGENTS.md
  between 3792dd3 and 7e5356d differs by exactly that line and the hash, and a fresh
  `sync-runtime --target z1` is byte-identical to the committed file. The line reads right.
- **The shipped token kept** — closed. `--adopt` on `z1` wrote a new token.
- **Hints without `--tool`** — closed for the CLI's own: `state status` (scan, plan), `features
  init`, `perf init`, `perf show`, the migrate resume, `review`'s "re-run", stale findings,
  `migrate`'s stale-driver refusal and the block's command list, each run as printed in `two`.
  Open elsewhere: findings 2, 3, 4.
- **Workloads and features errors with the folder-form path** — closed in `perf run`/`perf show`,
  `features map`, `features init`, the cockpit's Speed and Features views; open in verify and
  promote (finding 5).
- **Which tool is open** — closed: "two · tool t-lzg · ✓0/4" on the title and tree root;
  harness-mcp `"tool":"t-lzg"` and "serving … · tool t-lzg (".
- **"Not a target" said three ways** — closed: one sentence in the CLI, cockpit and harness-mcp;
  "this project has no mapped tools; drop --tool"; two tools without `--tool` exit 1 in all three.
- **Adoption's words** — mostly closed: the help and dialog say the harness builds and runs the code
  in the sandbox and that the token is replaced; the claims half is finding 6.
- **The refusal without its path; "add --adopt" in the cockpit** — closed: the path leads the
  sentence; the cockpit away from a terminal says "start the cockpit in a terminal and answer its
  question" (but untested, see Tests).
- **`◌` meaning two things** — closed: `⊖` "not part of this tool: a project file the open tool
  does not list" in the help's States (finding 10 for the row).
- **The weak tests** — closed: `a_file_list_target_is_refused_by_no_command` checks each exit code
  and its words; the hand-written-tool tests no longer pass `--adopt` (the `t-pair` test still does
  and says why: it writes results).

## Tests

The review's six reverts, each restored afterwards — all caught now:

| Reverted | Caught by |
|---|---|
| harness-mcp's acts without `--tool` (acts.rs:89) | `every_act_on_the_servers_own_target_carries_its_tool` (argv only) |
| `TargetContext::open` without the adoption check (config.rs:1047) | CLI adopt: `a_ledger_made_elsewhere_is_refused_until_adopted_once`, `the_projects_own_migration_folder_is_not_adopted` |
| the v2 record without `extra_link_args` (features.rs:717) | `the_v2_digest_hashes_resolved_forms_and_moves_with_the_link_arguments` |
| harness-detect's pruned list emptied (lib.rs:1312) | `the_detectors_prune_the_ledger_when_source_dir_is_the_root` |
| adoption skips the tools' build folders (adopt.rs:760) | `adopting_deletes_a_tools_build_folders` |
| the cockpit's background re-read without `--tool` (main.rs:1045) | `the_binarys_background_reread_opens_the_tool_it_was_started_on` (pty) |

Mine, on fix pass C's own rules: a project's `--adopt` keeps a shipped token (killed:
`a_hand_written_tool_is_made_here`, `a_ledger_made_elsewhere_…`); the block without "an agent never
adopts" (killed: `the_folder_forms_body_is_pinned`); a results-free `migration/` needs adoption
(killed: `a_hand_written_tool_is_made_here`); `command_line` drops the tool (killed: three in
tools.rs); the "of its own" refusal moved after the adoption question (killed, CLI and core);
`in_ledger` a no-op (killed: `every_tool_command_opens_the_tool_it_names`); the claims line always,
or only on deletions (killed: `the_projects_own_migration_folder_is_named_before_adoption`); the
title without the tool (killed); harness-mcp's refusal in the CLI's words (killed:
`a_ledger_made_elsewhere_is_refused_never_adopted`); harness-mcp's `tool` field changed (killed);
the cockpit read model's refusal in the CLI's words (killed).
**Survived:** the cockpit binary's no-terminal refusal switched to the CLI's words (main.rs:1016,
`Way::Cockpit` → `Way::Command`): every harness-tui test passes, so "add `--adopt` once" — which
harness-tui refuses as an unknown argument — could come back unseen. Fix: a test that runs the
binary on an unadopted copy with stdin not a terminal and checks the sentence.

**The no-sandbox test** (`the_map_is_refused_without_a_sandbox`): on this Mac `sandbox_mode()` is
"sandbox-exec", so it prints "skipped…" to stderr (hidden by the test harness) and reports ok — a
pass that ran nothing. CI's matrix has ubuntu-latest, where `sandbox_mode()` (sandbox.rs:38-44
knows only sandbox-exec) is "none", so there it asserts the refusal for real; forcing "none" here,
the test passes, so the assertion is live. Honest in CI, misleading in a local "all green". Fix:
`#[cfg_attr(target_os = "macos", ignore = "…")]` so it reads as ignored here.

Also: tools.rs removes its folder in one test of five; 112 `harness-cli-tools-*` sit in `$TMPDIR`.

## Holds

- The zopfli copy: one sentence naming the folder; `--adopt` once; a second says "already trusted".
- The cockpit's dialog adopts only on a typed `y` and says what is trusted, deleted and recorded.
- Two tools and no `--tool`: the cockpit lists them to pick in a terminal; the CLI and harness-mcp
  refuse naming both.
- A tool's writes stay under migration/tools/<id>/; verify on `t-pair` is green with its `-D`.
- The whole-root map writes project-map.json and a `.gitignore` that also covers each tool's folders.
- The "project changed" notice appears after an edit plus a new `project map` for a tool carrying
  a `map` stamp, in text and as a `project-notice` event; a hand-written tool gets none.


---

# Step (b) check: is the whole-root map right?

Checker lens: the algorithm against PROJECT-MAP-DESIGN §3.1 steps 6–11, §3.3, §3.5, the evidence
and configuration, the map file, the screen, the link checks, the lock, and the builders' tests.

Worktree `agent-a8fe6dd726433fc15`, at `1cded59`. The brief asked for `0a1d2e8` or later, but no
such commit exists in this repository: `git cat-file -t 0a1d2e8` says "Not a valid object name". The
branch head is `1cded59`, so that is what I checked. All runs used
`RUHARNESS_ADOPTED=<scratch>/adopted.toml` and copies under `<scratch>/w/`: zopfli without its
`migration/` and `harness.toml`, one benchmark case (`md5_digest_lib`), and liblzg and lz4 from
~/code/ruharness-test-downloads (both present, copied, nothing installed). My adversarial tests are
saved beside this report (`chk.rs`, `chk_b.rs`). I removed them from the worktree afterwards, so its
`git status` is clean.

## Findings

1. **The link check builds every file with different flags from the map's own compile, so it says
   "did not link" for programs that do link (High).**
   - Evidence: `link.rs:144` passes only the configuration's flags, and `mapfile.rs:526` hands it
     `map.configuration.flags`. But the map's compile uses each file's own `compile_commands.json`
     flags (`projectmap.rs:581-592`), and passes `-idirafter` for folders that hold a
     `system_headers` name (`projectmap.rs:1087`). The link compile passes plain `-I` for every
     folder (`link.rs:145`).
   - Ran `<scratch>/w/p1`: a `compile_commands.json` that gives `-DHAVE_CONFIG_H`, with `decode`
     defined in `src/dec.c` and in `src/mini.c`. The map compiles every file fine. The screen then
     says `link check: did not link`, gives no missing and no doubled symbols, and lists the set as
     `d1.1 src/dec.c, d1.2 src/mini.c` with no hold and no next step. The `--json` event is
     `{"k":"project-link","id":"t-tool","missing":[],"doubled":[]}`. The right answer is "both link,
     held".
   - The same happens with `from = "compile_commands"`: my test `chk_from_compile_commands_with_file_not_listed`
     records `t-main` with `flags: ["-DHAVE_CONFIG_H"]` and `linked: Failed{missing: [], doubled: []}`.
   - Ran `<scratch>/w/p4`: `system_headers = ["unistd.h"]` beside a project `compat/unistd.h` that
     holds `#error`. The map compile used the system's header ("the compile used system"), but the
     link check says `did not link`, because it reaches the project's header through `-Icompat`.
   - The failure: on a first run (a guess) of any project whose `compile_commands.json` carries a
     needed `-D` or `-I`, every link and every duplicate settled by linking is wrong. Those programs
     can never pass `accept`. The screen gives a failure with no reason.
   - Fix: build the link objects with the map's own `compile_argv` and each file's `FileFacts.flags`,
     reusing the objects the map already compiled where it can. A failed link whose missing and
     doubled lists are both empty should say why ("a file did not compile for the link").

2. **The probe budget reports libc names as missing (Medium).**
   - Evidence: `link.rs:299`. Once the 64 probes are spent, every group still unsorted is added to
     `missing`.
   - Ran `<scratch>/w/p3`: 70 symbols defined nowhere, beside about 45 libc and libm calls. The
     screen says `did not link; missing miss_1 … miss_70, pow, putchar, puts, qsort, rand, remove,
     rename, strchr, strlen, strrchr, strtod, strtol, strtoul, tan`. The libc names are not missing.
   - The failure: the missing list is the very facts a person or a model will act on, and here it is
     wrong.
   - Fix: keep the leftover names as a separate "not checked" list, or spend the budget on the
     project-shaped names first. Never put an undecided name in `missing`.

3. **A set found under one choice is labelled with the wrong choice (Medium).**
   - Evidence: `closure.rs:599` records `under` for whichever choice first reached the set, and
     `closure.rs:695` copies that label into the settled result.
   - Ran (fake linker): `x` is defined by `a.c` and `b.c`, and both need `w`, which `e.c` and `f.c`
     define. Only `b.c` with `f.c` links. The result is `d1 keep d1.2` and `d2 keep d2.2, under:
     "d1.1"`. So the screen says "reached only when d1.1 is kept" for a set settled under d1.2. When
     both choices link, d2 is still `under d1.1` although every choice reaches it.
   - Fix: record every choice that reaches a set. Mark it `under` only when not every choice does,
     and when settled, use the kept choice.

4. **A weak default can push out the strong override, depending on when it joins (Medium).**
   - Evidence: the round loop at `closure.rs:510` treats a need as met once any closure file
     defines it, weakly or not.
   - Ran: `main` needs `y`; `a.c` defines `y` and a weak `x`, and calls `x`; `b.c` defines a strong
     `x`. The closure is `[a.c, main.c]` and `b.c` becomes library `l-b`. When `main` also needs `x`
     directly, the closure is `[a.c, b.c, main.c]`.
   - The failure: §3.1's own rule says "the strong one defines it". The weak-hook pattern then maps
     to a program that behaves differently from the real build.
   - Fix: when a need is met only weakly, still add a single strong definer. Or record the pair as
     a fact the person sees.

5. **A `.c` that a header pulls in as text is offered as a library with no warning (Medium).**
   - Evidence: `Analysis.included_by` (`closure.rs:1082`) is computed and never used. `project.rs`
     never mentions `included_by` (grep count 0).
   - Ran `<scratch>/w/p5`: `all.h` includes `impl.c`, and `main.c` includes `all.h`. The screen
     shows `library l-impl: ./ impl.c` and "defined in two programs' files that never meet: impl in
     impl.c, main.c". §3.1 step 10 asks for this to be shown beside the closure, with a warning
     before it is offered.
   - Fix: show "textually included by …" on the closure and the library lines. Leave such files
     out of the between-programs list.

6. **Changing `system_headers` moves no digest (Medium, the design's gap too).**
   - Evidence: `config.rs:104` hashes only `{flags, from, name}`.
   - Ran (`hash.sh`): adding `system_headers = ["stdio.h"]` left `inputs_hash` at `7d65818c…` and
     `root_hash` unchanged. Yet `system_headers` changes the compile (`-idirafter`) and the settled
     ambiguous includes.
   - The failure: an accepted tool's "project changed" notice will not fire after this change.
   - Fix: put `system_headers` into the configuration digest, in the design, SCHEMAS and code
     together.

7. **The 30-minute budget does not cover the link checks (Medium).**
   - Evidence: the budget is checked only before each map compile (`projectmap.rs:610`).
     `link::analyze_linked` (`link.rs:353`) takes no deadline. Each program may spend 16 links plus
     64 probes plus its compiles, each allowed 120 s.
   - Read, not run.
   - Fix: pass the remaining budget to `CcLinker`. When it runs out, stop linking and record
     `limits_hit: budget`, or say "not link-checked: the time budget ran out".

8. **Under `from = "compile_commands"`, a file the JSON does not list is built with no flags, and
   nothing says so (Medium).**
   - Ran `chk_from_compile_commands_with_file_not_listed`: `tool2.c`, absent from the JSON, got
     `flags=[]`. Its closure kept `flags: []`, and `configuration.source` stayed `compile_commands`.
     No fact names the file.
   - In the same way, `settled()` (`mapfile.rs:620`) reads only `config.toml`'s flags. A JSON `-I`
     that settles an ambiguous include under this source is not counted (read, not run).
   - Fix: record such files (for example `not_in_compile_commands: [path]`) and keep the source a
     guess for closures that hold one. Let `settled()` read each file's own flags.

9. **A refusal for "no C files" leaves files in the folder (Low).**
   - Ran `<scratch>/w/p7` (a folder with only a README): exit 1. Then `find` shows
     `migration/.ruharness-adopted`, `migration/map/.lock`, and the root recorded in the adoption
     file. No `.gitignore` is written on this path, so all of it shows up in `git status`.
   - Fix: look for C files (or run the walk) before `acquire_project` (`project.rs:55`), or remove
     what this run created when it refuses.

10. **The `.o` names and the CMake/Meson bookkeeping flags flood the screen (Low).**
    - Ran `p1`: one line per entry, `ignored flag -o dec.o (1×): … remove it (the flags allowed are
      listed in docs/SCHEMAS.md …)`. Every entry carries a different `-o` value, so a CMake project
      prints up to 200 such lines (the cap at `evidence.rs:210`). "Remove it" also asks the person
      to edit a generated file.
    - Fix: drop `-o`, `-MF`, `-MT`, `-MQ`, `-MD` and `-MMD` silently, as `-c` already is, and word
      the rest as "not used by the map".

11. **List order in the map file is by text, not by number (Low).**
    - Evidence: `mapfile.rs:943` sorts definers by index string and `mapfile.rs:957` sorts sets by
      set string. So `d1.10` comes before `d1.2` (a set over the limit can have 12 definers, as my
      `chk_many_definers_index_order` shows), and `d10` before `d2`.
    - The file is still deterministic, and §3.3's "sorted by the first field's bytes" allows it,
      but it breaks "definers in path order".
    - Fix: sort by the parsed numbers.

12. **Small rule edges (Low).**
    - A fuzz driver's own needs never join the fuzzer's link. My test shows a driver that needs
      `helper.c`: every fuzzer says `missing helper`, and `helper.c` becomes a library
      (`closure.rs:830`).
    - A file with `LLVMFuzzerTestOneInput` and a data `main` is neither fuzzer nor program: a library.
    - "Duplicates between programs" also lists library files (`x` in `a.c`, in the closure, and in
      `b.c`, a library). Same flags in another order are "flags differ": fine, undocumented.

## The screen, read as a newcomer (zopfli, md5_digest_lib, liblzg, lz4)

- **What works:** zopfli maps to one program of 13 files with `-lm`, linked. The benchmark case maps
  to `library l-lib`. On liblzg, `t-unlzg` and `t-benchmark` are held on `d1`, with one index shared
  across both and each program's own symbols. lz4 shows a driver serving ten fuzzers, all linked,
  with 30 programs in 13 s.
- **"configuration: guessed (guessed)"** reads as a stutter, and `from` is never shown.
- **The outside-symbol lists are noisy:** they lead with compiler internals (`__stack_chk_guard`,
  `__chkstk_darwin`, `__stderrp`), and lz4cli's line is several hundred characters long.
- **"did not link" with nothing after it** (finding 1). A set that no choice links prints as a bare
  list of definers, with no "neither choice links" and no next step.
- **A held set's hint** is `--keep d1=d1.1`, picked arbitrarily, for an `accept` that does not exist
  yet. The closing line offers `project ask` "for advice on held choices" even when nothing is held
  (zopfli, md5). Both are known leftovers for steps (d) and (e).
- **"programs: 30"** counts fuzzers and the driver among the programs.

## Tests: the builders' tests and ten real reverts

I ran 81 projectmap unit tests and the CLI `project_map` tests: all green. Each revert below was
applied with `sed`, the tests run, and the file restored (`mut.sh`).

Caught:
1. A weak definition counted as strong (`closure.rs:347`): caught by `weak_beside_strong…`.
2. A data `main` taken as a program (`closure.rs:734`): caught by `a_data_symbol_main_is_no_program`.
3. The settled closure not recomputed (`closure.rs:699`, base instead of the leaf): caught by three
   tests.
4. The `programs_not_compiled` exclusion removed from libraries (`closure.rs:1038`): caught.
5. No halving in the probes, every failing group reported missing (`link.rs:307`): caught by
   `linked_comes_from_the_facts…`.
6. `from = compile_commands` left trusted although flags differ (`config.rs:276`): caught.
7. The `guessed` downgrade on a closure whose flags differ (`mapfile.rs:742`): caught by the CLI's
   `per_file_flags…`.

Survived:
8. Allowing 17 combinations instead of 16 (`closure.rs:605`): no test sits at the boundary
   (16 → linked, 17 → held).
9. One file listed twice with the **same** flags counted as differing (`evidence.rs:216`).
10. The between-programs list no longer excluding the duplicate-set symbols (`closure.rs:1059`).

Not tested at all:
- That the link check's compile matches the map's compile. `per_file_flags…` uses `-DA`/`-DB`,
  which change nothing, and never asserts `linked`.
- The 64-probe budget.
- That nothing built is run (see Holds for why it holds anyway).
- The label recorded with a settled set found under one choice.
- A `.c` included by a header on the screen.

The test names otherwise prove what they say.

## Holds

- **Programs, kinds and indexes:**
  - A data `main` is no program.
  - A fuzzer that also defines `main` is kind `main`.
  - The driver is found and has no closure.
  - `p1…` follows path order over `main` programs only.
  - Fuzzers are never asked.
- **Closures:**
  - A pending set met later by another symbol is met, even when the definer arrives in round 2.
  - A choice recomputes the closure from scratch: the file the other choice pulled became a
    library.
  - Exactly 16 combinations with nesting were linked (16 calls); 20 were held without linking.
  - Common beside strong is no collision.
  - `needs_from` is recorded and its symbol reported missing.
  - A library can never need another library: groups join on single-definer needs.
- **Ids:**
  - `Main.c` and `MAIN.c` take their folders.
  - Two 70-character folders give `…x` and `…x-2`, at most 62 characters, which passes
    `is_tool_id`.
  - An accepted id is kept by `closure::analyze`. It is not yet wired: `mapfile::analyze` passes
    `&[]`, step (e).
  - Reversed input gives an identical analysis.
- **Evidence:**
  - CMake's `-isystem /usr/local/include` is refused by name as outside the project.
  - A `directory` with a trailing slash, Meson's `../src/./b.c`, its relative `-I` rewritten
    root-relative, and `-MD -MQ x -MF y` refused with their values all work.
  - The splitter: `'a b'` gives `a b`, `"a\"b"` gives `a"b`, `a\ b` gives `a b`, and `$(x)` and
    `` `y` `` stay as text.
- **Map file:**
  - Byte-identical across two runs on zopfli.
  - `root_hash` moves with a header and `compile_commands.json`, not with a README.
  - `inputs_hash` moves with flags and their order.
  - No `/Users` anywhere. The only absolute paths are the toolchain's `system_include_dirs` under
    `/Applications/Xcode.app`, as §3.3 asks.
  - Every §3.3 field is present with the SCHEMAS additions (`compile_commands_path`, `at_least`,
    `flags_differ`).
- **The cap path:** at depth 33 the run exits 1 with "depth limit of 32"; the file holds the facts
  of `main.c` (compiled ok) and no programs, closures or libraries.
- **Nothing built is run:**
  - The map's runner allows only `cc` (`Runner::map(..., &["cc"])`, and `exec.rs:579` refuses
    anything else).
  - Each linked program is deleted at once.
  - `p2`'s constructor and `main` both write a marker, and neither marker appeared.
- **`linked` never comes from text:** it comes only from the exit status and the `objsyms` facts.
- **Locks:** project lock before ledger lock in `project map` and `sync-runtime` (`main.rs:1415`);
  both are try-locks, so the order cannot deadlock; a lock held elsewhere refuses in one sentence.
- **`--json`:** the `project-file`, `project-build`, `project-program` and `project-link` events
  carry what SCHEMAS lists.


---

# Security check of step (b) of the project map

Worktree at 1cded59 (the head of `claude/ruharness-resume-ee312a`; the 0a1d2e8 named in the brief does
not exist yet, so nothing newer was merged). Scope: what step (b) added that an untrusted project can
abuse: the configuration file, `compile_commands.json`, the closures and link checks, ids, the map
file, the screen, the project lock, the `.gitignore` writer and the notice. Experiments ran a release
build against crafted folders under the scratchpad, `RUHARNESS_ADOPTED` pointed at a scratch file.
Nothing was downloaded, installed or committed; the worktree is clean. The scripts are in
`scratchpad/sec/e*.sh`. Timings are from a busy machine shared with other agents.

## High

None found.

## Medium

1. **The 30-minute budget guards only the compile loop: hashing, parsing, `compile_commands.json`
   and the whole link phase run without a time limit.**
   Evidence: the only budget check is `projectmap.rs:610`, inside the step-5 loop. Before it, every
   walked file is hashed with no size limit (`projectmap.rs:729`, `hash::file_hash` streams the whole
   file, `too_large` files included) and parsed (`projectmap.rs:531`); `evidence::gather`
   (`projectmap.rs:572`) dedups each file's flag lists with `Vec::contains` (`evidence.rs:216`), which
   is quadratic. After it, `mapfile::analyze` (`mapfile.rs:526`) links every program, and a program
   with open duplicate sets links up to 16 choices (`closure.rs:679-681`), each failed link followed by
   up to 64 probe links (`link.rs:29`, `link.rs:291-317`). No deadline reaches `CcLinker`.
   Ran it, yes: 10 programs that each call 64 undefined functions took 26 s (2.6 s each); 2 programs
   that also hold four two-way duplicate sets took 87 s (43 s each), so 10 000 such programs, whose
   compiles fit inside the budget, would link for about 5 days. A 16 GiB sparse `big.h` (0 bytes on
   disk) took 12 s to hash, so a 1 TiB one takes about 13 minutes and a tar can carry many. A
   `compile_commands.json` of 200 000 entries for one file (14 MB) took 58 s; the 64 MiB cap allows
   about 20–35 minutes of it.
   Fix: carry one deadline through every phase (the hash loop, the scan loop, `gather`, every link and
   probe), stopping as a `budget` limit; hash a too-large file by its size and first 8 MiB, or not at
   all; dedup flag lists with a set.

2. **Neither memory nor the map file's size has a bound: the 200 000 cap counts distinct names, not
   bytes or repeats, and the parser's names and the JSON tree are never capped.**
   Evidence: `projectmap.rs:620` counts distinct names only; names have no length limit, and every
   file keeps its own copy of each name it defines or needs. `parser` (`projectmap.rs:533`) keeps
   every function and call name of every parsed file, `.h` files too, for the whole run, although the
   closure reads it only for files that did not compile. `evidence.rs:184` parses the whole 64 MiB
   file into a `serde_json::Value` tree. The map file is written with no size limit
   (`mapfile.rs:991-1003`), and `unfound_entries` and `flags_differ` copy from `compile_commands.json`
   whatever it holds.
   Ran it, yes: 200 hard links to one 6 MiB source that needs four 512 KiB names (4 MB on disk) gave
   a 420 MB map and 1.29 GB of memory in 19.5 s. The 20 000-file cap would allow about 42 GB of map
   and well over 100 GB of memory, all inside the budget. 20 hard links to a 7 MiB file of 700 000
   calls used 1.2 GB (60 MB a file) before any compile ran. A 24 MiB `compile_commands.json` of
   `{"":0}` objects used 2.5 GB, so 64 MiB uses about 6.7 GB. A map over 256 MiB also leaves the
   "project changed" notice saying "could not be read" every time (`ledger.rs:387`).
   Fix: cap name bytes (count names over a few KiB as odd names, and add a total-bytes limit beside
   the 200 000 names, as `objsyms` already does per object); keep parser facts only for `.c` files
   that did not compile; read `compile_commands.json` into typed entries with a cap on the entry
   count; treat a map over a set size as a limit hit.

3. **The link checks compile with other flags than the map did, keep every object until the run
   ends, and never check an object's size.**
   Evidence: `mapfile.rs:526` gives `CcLinker` only the configuration's flags. Under a guess or
   `from = "compile_commands"`, the map's own compile used each file's entry flags first
   (`projectmap.rs:580-591`). `link.rs:135-158` checks no size, and `link.rs:208-217` keeps every
   object in `objects` and on disk until the linker is dropped.
   Ran it, yes: 10 programs with 48 MiB objects left 481 MB in the fresh folder at the peak, so about
   9 000 such files would fill this disk's 412 GB of free space. A file whose entry says `-DSMALL`
   compiled small for the map (under the 64 MiB cap), then compiled in the link check to a 200 MiB
   object plus a 200 MiB program (411 MB peak). That link check reported "linked" for code holding a
   function, `only_in_link`, that the map's facts never saw.
   Fix: compile each link object with the file's own `FileFacts::flags`, as the map compile did;
   refuse an object over `MAX_OBJECT_BYTES`; delete objects once no remaining closure needs them, or
   cap the fresh folder's total bytes as a limit.

4. **A compile that fails leaks facts about the person's machine into the map, through the line it
   stopped at and the missing header's name.**
   Evidence: on a failed compile, `compile` returns at `projectmap.rs:1181` before it reads the `-MD`
   list, so `outside_includes` is never set for a failure. `classify` keeps `at` and the header name
   (`projectmap.rs:1353-1410`). The map profile allows reading everything outside `/Users`,
   `/Volumes` and the temporary folders, so `__has_include` can test `/Applications`, `/Library`,
   `/opt/homebrew` and the shared work folder.
   Ran it, yes: a chain of `#if __has_include("/Applications/<App>.app/Contents/Info.plist")` /
   `#error` blocks wrote `"at": "a.c:5"` into `project-map.json`: Xcode is installed. Under the same
   condition, `#include "has-xcode"` stores any text the project chooses as `header`. The map file is
   not in the written `.gitignore` (`mapfile.rs:40-51`), so it is committed with the project; it is
   also read by harness-mcp's agents and, later, by `ask`. One bit a file (did the compile read
   outside, yes or no) leaks even on a successful compile, over up to 20 000 files.
   Fix: read the `-MD` list on a failure too (clang writes it even then: checked), and drop `at` and
   `header` when it names a file outside the root and the toolchain. To close the channel itself,
   switch the map profile's reads to an allow-list (the root, the fresh and work folders, the
   toolchain, `/usr`, `/System`, `/dev`); otherwise say in §6 that file existence outside the home
   folder can reach the map.

5. **A download's own `migration/map/config.toml` is shown and recorded as the person's statement,
   with no question asked.**
   Evidence: `adopt.rs:260-288` counts a `migration/` holding only `map/config.toml` (and tool
   `harness.toml` files) as holding no results, so `adopt::check` passes it as "made here". Then
   `config.rs:273-281` marks the entry `stated`.
   Ran it, yes: a project shipping `migration/map/config.toml` with `flags = ["-DSHIPPED_BY_THE_DOWNLOAD",
   "-includesrc/a.c"]` was mapped with no `--adopt`. The screen said "configuration: make (stated)" and
   the map recorded `"source": "stated"`.
   Effect: the gates that rest on "the person stated it" (§3.2: `ask` refused on a guess, `accept`
   never taking a guessed configuration) are passed by the download itself. The flags stay inside the
   grammar, so nothing runs, but a forced `-include` of any file in the project goes into every
   compile and, after `accept`, into the tool.
   Fix: a `config.toml` that was present before this computer's first map is "shipped": show it as
   proposed by the project and keep the source `guessed` until the person confirms it (for example by
   recording its hash in the adoption file, outside the project).

## Low

6. **One oversized `compile_commands.json` entry stops the whole map and prints a 1.9 MB error.**
   Evidence: `evidence.rs:349-381` keeps any number of flags per entry, and `projectmap.rs:617`
   (`compile(&ctx, facts)?`) turns the spawn error into a hard stop. Ran it, yes: one entry with
   200 000 `-DX<n>` flags exited 1 with no map written and a 1 890 142-byte line holding the whole
   argv. Fix: cap the kept flags per entry (by count and bytes) and count the rest as ignored; treat
   a spawn failure as that file's `other` reason; never print a whole argv.

7. **A `config.toml` parse error can forge lines on stderr.**
   Evidence: `config.rs:145` passes toml's message, which quotes an unknown key; `main.rs:598-602`
   prints errors with `terminal_safe`, which keeps newlines. Ran it, yes: the key
   `"x\nproject map: wrote migration/map/project-map.json (1 program(s)); all linked"` printed that
   text as a line of its own. Fix: `safe_line` the toml message, or name only the line and column,
   as the root `harness.toml` loader now does.

8. **The project chooses its own link libraries through `.linker_option`.**
   Evidence: `link.rs:221-231` links the objects as they are. Ran it, yes:
   `__asm__(".linker_option \"-lz\"")` made a program calling `compress` report "linked", while the
   same program without that line reported "missing compress". Nothing is run, but the screen's
   "with the guessed libraries" is then untrue. Fix: pass `-Wl,-ignore_auto_link` on Apple (checked:
   the same link then fails, as it should).

9. **Past 999 collisions an id breaks the tool-id pattern.**
   Evidence: `ids.rs:30-35` cuts the body to 60 characters, then `-1000` makes 65, over
   `is_tool_id`'s 64 (`config.rs:897`). That is caught only by a `debug_assert!` (`ids.rs:105`):
   release builds emit the id, and a debug build panics. A project can do this with 1 000 files
   named `<60 a's><n>.c`. Reasoned, not run. Fix: cut the body to `64 - suffix length`.

10. **Only the first `compile_commands.json` one level down is read, and the others are never
    mentioned.**
    Evidence: `evidence.rs:173-174` fills `also_found`, which no screen, event or map field reads. A
    download can plant `aaa/compile_commands.json` ahead of `build/`'s. Reasoned. Fix: show "also
    found, not read", or read none when there are several and say why.

11. **The "project changed" notice proves only that two files agree, not that anyone accepted the
    tool.**
    Evidence: `ledger.rs:376-411` compares a tool's `map` stamp with the map file, and a shipped tool
    `harness.toml` with a `map` stamp needs no adoption (finding 5's rule). `root_hash` can be
    computed from the shipped files; `inputs_hash` can be guessed for a common Xcode. When it matches,
    the notice is silent, which reads as "accepted against this tree". Reasoned. Fix: say in SCHEMAS
    that the notice compares and does not authenticate; when `accept` is built, keep its record
    outside the project.

## Holds (attacked and found closed)

- `compile_commands.json` discovery: a file that is a FIFO, a link out of the root (`/dev/stdin`
  included), or under `migration/` or a dot-folder is never read (`evidence.rs:248-275`); over
  64 MiB it is not read; deeply nested JSON stops at serde's depth limit of 128.
- Entries: a `directory` that is a link out of the root is ignored. A `file` naming `/dev/stdin`, a
  FIFO or an outside path is only recorded as unfound, never opened. `-Xclang @file`, `-isystem`
  with nothing after it, `-include ../x.h`, a `-D` value holding a newline or starting with `-`, a
  path after `-fvisibility=hidden`, and every option outside the grammar are refused by name. Ignored
  flags are named at most 200 times and shown through `safe_line`. `-std=c23` is in the set.
- `config.toml`: a link out of the root, a FIFO and a file over 1 MiB are refused before reading. A
  name holding `/` is refused (`..` is accepted, but a name never becomes a path). `system_headers`
  with `..` is refused; one with a space is only compared with walked paths. Names differing only by
  case are distinct, and `--configuration` matches exactly.
- The compile and link: every `cc` runs from the work folder with `PATH` filtered and the toolchain
  pinned, under the map profile with the network denied and writes only to the fresh folder. Path
  flags reach `cc` absolute (`-idirafter` only for walked folders). A dangling link accepted at check
  time can only point to a folder created later in the same run: the fresh folder's name is random,
  and anything read outside has its names withheld. `-Wl,-u,_<sym>` takes identifier-shaped names
  without `$` only, so no symbol can look like a linker option. `-lm`, `-lz` and `-lpthread` are the
  only guessed libraries. No compile-command flag reaches a link. Every linked program is deleted at
  once and never run.
- The map file and screen: no compiler text is stored (`message` is kept in memory and never shown;
  `at` must lie inside the root; `header` must be a clean relative path). Every printed project string
  goes through `safe_line`, including the toolchain line, the configuration name, flag values and
  ignored flags. `kind_guess`, kinds, reasons and indexes are fixed words. `--json` escapes
  everything `unsafe_to_show` names. Non-UTF-8 names cannot exist on APFS (on Linux they would be
  stored lossily).
- The project lock: `migration/` or `migration/map/` as a link is refused. A `.lock` that is a link,
  a FIFO or a hard link (`nlink > 1`) is refused. The lock is released and emptied on every refusal.
- The `.gitignore` writer: an existing file or link is never written through or replaced
  (`create_new`). The map file is written to a fresh temp file and renamed, which replaces a planted
  link rather than following it.
- Ids are made only from `[a-z0-9_-]`, so no hostile stem yields `/`, `.` or an empty id.


---

# Triage (the session, 2026-10-08)

The review of (a)+(c) is closed: every experiment re-run by the checkers passed, the three
byte-identity claims hold (101 facts files, zopfli's digest, u001's verdict), and all sixteen
reverts were caught. Step (b) works on zopfli, the benchmark, liblzg and lz4, but its link check
compiles differently from the map and its bounds cover only the compile loop. What follows decides
every finding and groups the work into three fix passes by file ownership. The decisions that
change the design are recorded in its §9 and in DECISIONS.

## Decisions

1. **The link check compiles exactly as the map did**: each file with its own `FileFacts::flags`
   through the map's `compile_argv` (so `-idirafter` for `system_headers` too), an object over
   64 MiB refused, objects freed as soon as no remaining closure needs them. A link that fails with
   nothing missing and nothing doubled says why ("a file did not compile for the link").
2. **A name the probe budget could not decide is "not checked", never "missing".**
3. **A need met only weakly still pulls in a single strong definer** (§3.1's rule: the strong one
   defines it); the weak/strong pair is a fact on the closure.
4. **`system_headers` joins the configuration digest** (design §3.3, SCHEMAS, code).
5. **One deadline through every phase** of `project map`: the hash and parse loops, the evidence
   reader, every link and probe; past it the run records `limits_hit: budget` and stops. A file
   over the 8 MiB cap is hashed over its size and its first 8 MiB (design §3.1 step 1; `too_large`
   files only, so no existing hash moves).
6. **Memory and size bounds**: a symbol name over 4 KiB counts as an odd name; a total of 64 MiB of
   name bytes is a limit beside the 200 000 names; parser facts are kept only for `.c` files that
   did not compile; `compile_commands.json` is read into typed entries with a cap of 50 000 entries,
   64 flags and 16 KiB per entry (the rest counted as ignored); a map file over 64 MiB is a limit hit.
7. **A failed compile reads its `-MD` list too**; `at` and `header` are dropped when they name
   anything outside the root; one bit per file (did the compile read outside) stays, and the
   design's §6 says that file existence outside the home folder can reach the map through
   `__has_include`. An allow-list read profile is the item that closes it (§8, open for later).
8. **A `config.toml` that came with the project is proposed, not stated.** The adoption record
   gains the hash of a `migration/map/config.toml` present when the root is first recorded on this
   computer; while the file still has that hash, `map` shows "came with the project: proposed" and
   keeps the source `guessed`; `project map --adopt` once states it (the same flag as every other
   trust decision), as does the person's own edit (a different hash).
9. **The map's closing line names what exists**: until steps (d) and (e) land it says that a tool
   is written by hand under `migration/tools/<id>/harness.toml`, and `ask` is named only when a set
   is held; the `--keep` hint never picks a definer for the person.
10. **Verdicts made elsewhere are shown as such**: adoption records its time; `state status` and the
    cockpit mark a verdict older than that time "made elsewhere" until `verify` runs it here; the
    adoption line speaks of claims only when something was verified, and of deletions only when
    something was deleted.
11. **Each unit's hashed closure is the resolver's**: `include_rule::closure` moves into harness-core
    and `compute_inputs`, the planner's `source_hash` and status hash it, so a header an ambiguous
    include lands on stales the verdict; the folder form is untouched (the resolver gives it
    nothing). `-idirafter` joins the grammar and the resolver so `accept` can carry
    `system_headers`.
12. **The mirror's per-file cap is 64 MiB** under the 256 MiB total; a manifest with `[project]`
    is refused; the include reader's `<` rule keeps a running line start (no quadratic search); the
    tree-sitter walk is bounded (an explicit stack or a depth cap); `#import`, `#include_next`
    (resolved as an ordinary include), `%:include`, a lone `\r` line end and a splice followed by
    blanks are read; a folder named by both `-I` and `-isystem` is searched at the `-isystem`
    position only; a dot-folder landing the resolver reaches is readable by a prompt; the
    `configuration` entry hashes resolved forms and only the unit's own files' folders plus the
    flags.
13. "Once per computer" is once per checkout (adoption is by canonical path); DECISIONS and the
    handoff say so.

## Fix pass D — the map (harness-oracle `projectmap/*`, harness-cli `project.rs`, the lock)

Decisions 1, 2, 3, 4, 5, 6, 7, 8 (the map's side), 9. Also: the `under` label records every
choice that reaches a set and names the kept one once settled (correctness 3); `included_by` shown
on closure and library lines and such files left out of the between-programs list (5); files a
`from = "compile_commands"` entry does not list are recorded (`not_in_compile_commands`) and keep
their closures guessed, and `settled()` reads each file's own flags (8); the C-file check runs
before the project lock is taken and a refusal cleans up what the run created (9); `-o`, `-MF`,
`-MT`, `-MQ`, `-MD`, `-MMD` dropped silently and the rest worded "not used by the map" (10);
definers and sets sorted by their numbers (11); a driver's own needs join each fuzzer's link, a
file defining `LLVMFuzzerTestOneInput` beside a data `main` is a fuzzer, and the between-programs
list comes from program closures only (12); the screen: "configuration: a guess (no config.toml)"
with `from` shown, compiler and runtime names folded into a count after the project's outside
symbols, long lines capped, "neither choice links" said, "programs: 30 (10 fuzzers, 1 driver)";
the `config.toml` parse error through `safe_line` (security 7); `-Wl,-ignore_auto_link` on Apple
(8); ids cut to 64 minus the suffix (9); "also found, not read" for other `compile_commands.json`
files (10); SCHEMAS says the notice compares and does not authenticate (11). Tests at the 16/17
boundary, a file listed twice with the same flags, the between-programs exclusion, the link
check's compile equal to the map's, the probe budget, the `under` label, a `.c` included by a
header on the screen, nothing built run.

## Fix pass E — the person's side, round two (harness-core adopt.rs/features.rs `invalid`/perf words/observer, harness-cli except project.rs, harness-tui, harness-mcp, harness-llm's and harness-oracle's hint strings, devtools)

Decisions 10 and 13. Also: harness-mcp's adoption sentence and status notes through
`command_line` with the server's tool (flow 2, 3); every remaining hint the flow report lists
spelled with `--tool` when a tool is open, through one helper that takes the open tool or the
ledger's relative path (4); `features::invalid()` and `FeatureSnapshot::load` take the ledger's
path so verify and promote name the tool's file, and `in_ledger` at the edges goes (5); the
`.gitignore` written when any command first creates a ledger (8); hints carry `--target <as
given>` when it is not the current folder (9); the tree row keeps the file's name and shows
headers neutrally until a scan says which are reached (10); the dialog lists the tools' build
folders too (11); `cockpit-drive` passes `RUHARNESS_ADOPTED` through (13); each sentence in flow
finding 14 made one plain sentence with a next step; a test for the cockpit binary's no-terminal
refusal; the no-sandbox test marked ignored on macOS; tools.rs removes its folders. SCHEMAS'
adoption paragraphs (flow 7) are updated here too, since the words are this pass's.

## Fix pass F — the readers' and oracle's residuals (harness-core sources.rs/status.rs/config flags/planner, harness-scan, harness-detect, harness-llm trajectory.rs, harness-oracle lib.rs/include_rule.rs/featuremap.rs/unit_crate.rs/shape of the closure)

Decisions 11 and 12. Also: one skip-note helper for scan and detect (fix-ab 10); the `cc -M` test
fails instead of skipping when `cc` fails; a timing test with a 1 MiB line.

## Recorded, not fixed now

- The map profile's read allow-list (decision 7's last sentence): §8.
- Flow finding 7 is folded into fix pass E; the rest of step (f)'s docs stay step (f).
