# Features map — the compiler-guided probe (design)

Status: **revision 2, for its check** (2026-09-30). Decided by the person: DECISIONS.md,
"Features map: PROPOSAL … let the compiler decide". Replaces the "Unwatched" rules and the
probe runtime of FEATURES-DESIGN.md §5.3–§5.4; the rest of §5 stays. History: the draft
(777cd27) had an adversarial review from four angles (37 findings, each reproduced by two
verifiers); revision 1 (a14c807) was checked (4 findings fully resolved, 33 in part, 20 new —
each new one confirmed by two verifiers). §9 records how this revision answers all of them.

## 1. What changes, in plain words

The features map runs each scenario on a scratch copy of the C program in which every watched
function starts with a small note ("I ran"). Today a set of syntax rules guesses whether a note
at a function's start would compile; macros make that guess impossible to get right, and seven
checks in a row found wrong guesses both ways.

The new probe stops guessing:
- it puts a note in every function where one can be placed at all (§3.1);
- it checks that the copy, apart from its notes, **preprocesses to exactly the same text** as
  the original program — the general "is this the same program" check (§3.3);
- it compiles the copy for real, one top-level `.c` file at a time, and takes the note out of
  exactly the functions the compiler's errors land in, recording the compiler's words (§3.4);
- the note is a one-byte store into memory the harness reads back (§3.6).

Words used below: a **top-level file** is one of the program's top-level `.c` files — one
compile each; a **probed file** is a file of the scratch copy that has notes in it (a `.c` or a
header).

## 2. Premise, run end to end

A prototype (scratchpad `proto/`: the scan crate with the guessing rules off and a "skip these"
list; a driver that compiles, takes out the notes the errors land in, and tries again), before
this revision's changes:

| input | the copy builds (today → prototype) | functions left unwatched (today → prototype) | rounds |
|---|---|---|---|
| 24 adversarial repro files of the last three checks | 17/24 → **24/24** | 2 930 → 2 567 | ≤ 2 |
| the same with `-DA` (25 compile) | 18/25 → **25/25** | 2 140 → 1 999 | ≤ 2 |
| 42 files of the 294-file corpus that compile as they are (sqlite ×2, blake3, tree-sitter, zopfli, …) | 42/42 → 42/42 | **3 044 → 6** | 1 |
| 37 Python/Ruby extension files that compile with the Python headers | 37/37 → 37/37 | **1 162 → 129** | 1 |

The prototype read each compile's whole error output and used `-fsyntax-only`; the build (§8)
re-runs all four through `harness features map` itself and replaces this table. On real code the
compiler rejected no note: sqlite3.c goes from ~1 500 unwatched functions to 0.

What the compiler judges, and what it does not. It tells whether a note **compiles** where it
stands: on Apple clang 21 every pragma that must open a block (`STDC FENV_ACCESS`,
`FP_CONTRACT`, `CX_LIMITED_RANGE`, `FENV_ROUND`, `float_control`, every `clang fp` form) is an
error after a statement, as are a statement in a naked function and `__label__` after a
statement. It does **not** tell whether the copy is the same program: a note in an `#if` branch
the build skips is never compiled (§3.1 rule 3, §3.2), a note inside a macro argument that
becomes a string compiles and changes the program's text, a file read as data changes its
bytes (both §3.3), and a note that changes inlining can change what an optimisation-dependent
builtin picks (§6). The run-time guard stays: each scenario runs plain, probed, plain, and a
probed run whose output differs is recorded (`probe_agrees = false`) — it covers output
differences only.

## 3. The design

### 3.1 Where a note can go

A function the facts record gets a note right after its body's `{` unless:
1. the parser could not read the definition (it lies under an ERROR node — the body's bounds
   are then a guess). Rule 1 also guards a head split by `#ifdef`/`#else` that the parser
   cannot read; it is not narrowed without §3.2's check.
2. its body is not a `{ … }` block, or the body's first byte is not a real `{` (reason: "its
   body is not a { } block");
3. its body's `{` is inside a conditional group the definition itself opened or crossed:
   scanning from the definition's first byte to its `{`, a directive line is one whose first
   characters, after blanks, comments and line splices, are `#` or the digraph `%:`; an `#if`,
   `#ifdef` or `#ifndef` still open at the `{`, or an `#else`, `#elif`, `#elifdef`, `#elifndef`
   or `#endif` of a group opened before the definition's first byte, makes the note one-branch
   only (reason: "a # line between its head and its body"). A conditional opened and closed
   inside the head (a parameter list split by `#ifdef`) leaves the `{` unconditional and does
   not count.

Removed (the compiler decides): the parse-error-in-the-head rule, the `naked` word rule and its
`parameter_list` helper, and every leading-run rule. `probe_source` stays pure; it gains the
list of `(file, id)` pairs **not** to watch and reports, for each note it places, the copy's
byte range of the function's body and the line of its `{`.

### 3.2 Every note is compiled (the check behind rule 3)

Rule 3 is a text rule; its net is the compiler's own preprocessing. Each top-level file of the
copy is preprocessed in its listing run (§3.3), and the preprocessed text's line markers say
which lines of which files were kept. A note whose function has a body line kept but whose own
line (its `{` line) was dropped sits in a skipped branch: it loses its note, reason "its body's
brace is inside #if". §8 re-runs this check over every file and define set of the premise
inputs: every note appears in the preprocessed text exactly when its definition does.

### 3.3 The copy preprocesses to the same program

The listing runs of FEATURES-DESIGN §5.3 (what each compile reads, `-M`, and enters, `-H`)
become, for the program and for the copy alike, `cc <flags> -E -o <build>/<n>.i -MD -MF
<build>/<n>.d -H <file>` — one run that gives the dependency list, the headers entered and the
preprocessed text. Then, in order:
1. **Files read as data.** A probed file that some compile lists (`.d`) but never enters
   (`-H`) is read as data (`#embed` in any spelling, `__has_embed`); a probed file named by an
   `.incbin` in any top-level file's preprocessed text (matched by its path under the mirror or
   its name) is too. Each goes back unprobed, reason "read as data by <top-level file>".
   `__has_include` reads no bytes: a file only looked up this way is listed and not entered,
   and costs its notes — rare, and never a wrong map.
2. **Notes turned into text.** A note's text inside a string or character literal of the copy's
   preprocessed text (a macro argument that becomes a string) — its function loses its note,
   reason "its body is inside a macro argument that becomes a string".
3. If 1 or 2 took notes out, the files they touched are re-probed and the listings of the
   top-level files that read them run again (at most twice; a third change refuses the map with
   the words "the copy keeps changing while it is checked", a harness bug).
4. **The same text.** For each top-level file: the copy's preprocessed text, with the probe
   header's lines and every note's text removed and the mirror's paths in line markers put
   back to the target's, must equal the program's preprocessed text. A difference refuses the
   map by name before any build — "the copy of <file> is not the same program at <file>:<line>"
   — the net for anything the steps above do not name.

The comparison replaces nothing in FEATURES-DESIGN §5.3's `reads()` (the same files by the same
names, the headers entered in order and depth): both run.

### 3.4 Compile, and take out what does not compile

The copy's build becomes one real compile per top-level file, then one link:

1. `cc <the copy's flags> -c -o <objects>/<n>.o <file>` at `-O2` — full code generation (a
   syntax-only check misses errors that code generation finds, e.g. an `asm` "i" operand that
   is no longer a constant once the function stops being inlined). The compiler kind comes from
   `cc -dM -E -x c /dev/null` (`__clang__` defined: clang; otherwise gcc, with `__GNUC__` its
   version). Flags added: clang `-ferror-limit=0 -Xclang -fno-diagnostics-use-presumed-location`
   (every location is the copy's physical file and line — `#line` ignored — in the error lines
   and the include chain); gcc `-fmax-errors=0 -ftrack-macro-expansion=0`, plus
   `-fdiagnostics-column-unit=byte` from gcc 11.
2. **The whole error output.** A new runner call returns a failed compile's whole stderr (up to
   the 64 MiB output cap) and how it ended; the 8 KiB excerpt stays for messages only. A
   compile that timed out, overflowed or was killed, or that failed with no located error line
   (`clang: error: …`, `cc1: error: …`, a signal), is a refusal — "the copy's compile of <file>
   did not finish: <words>" — never read as note errors.
3. **Placing an error.** Each `error:` or `fatal error:` line gives a file, a line and a byte
   column. The file (canonicalized, the canonical mirror prefix stripped, as `reads()` does) is
   a probed file: the error's byte offset (lines counted as the compiler does — `\n`, `\r\n`
   and a lone `\r` each end one) falls in the innermost body that still carries a note, and that
   note is taken out. The file is not probed (a system header, an unprobed `.inc`): the include
   chain printed last before the error (the compiler prints one only when it changes) gives the
   innermost **probed** file and its line, placed the same way. On gcc, an error in a top-level
   file whose probed files hold a `#line` or line-marker directive goes to step 4 (gcc has no
   physical-location flag). The reason recorded is the error's message after `error: `, at most
   160 bytes cut on a character boundary, control characters as `?`.
4. **Errors that cannot be placed** — a probed file position that no body still carrying a note
   holds, a presumed location, a chain with no probed file — first: the top-level file is
   compiled once with all of its notes out. If the error remains, the map refuses: "the copy of
   <file> does not compile even without notes: <words>" (the plain build compiled it, so the
   copy differs — a harness bug). If it goes away: the notes of the top-level file and the
   probed files it reads are put in a fixed order, and a binary search finds the smallest k such
   that taking out the first k makes the file compile; note k is the culprit and is taken out,
   reason "found by elimination: <the error>"; repeated until the file compiles. Taking notes
   out never adds an error, so the search is sound, and each search removes one note.
5. **The link**, objects in the plain build's input order, the runtime's object last. An
   undefined symbol whose referencing function is watched and still carries a note (ld64
   `"_name", referenced from: _fn in f.o`; GNU ld ``in function `fn'``; lld `>>> referenced by
   f.c … (fn)`) takes that function's note out and recompiles its top-level file. Any other
   undefined symbol (no referencing function given, or one already without a note — a callee's
   note that tipped an inline function over the threshold): step 4's search over the notes of
   the top-level files the linker names, with the relink as the test. The map refuses only when
   the link fails with those files' notes all out.
6. **Bounds and progress.** Per top-level file: at most 8 placed rounds and 64 compiles; for the
   whole pass: 400 compiles or 15 minutes. Past a per-file bound, that file and every probed
   file it reads go back unprobed (reason "the copy of this file did not compile with notes:
   <words>") and it is compiled once more; past the whole-pass bound, every remaining probed
   file does. The CLI prints "Checking where the notes compile… <file> (round <r>)" as it goes.
7. **Re-compiles.** A probed header whose notes were taken out makes every top-level file that
   enters it (its `-H` list) compile again — after the first pass over all files, once per file
   per pass, not counted in step 6's per-file bound. Files are handled in sorted order; the
   result is the same for the same inputs.

Order: the listings (§3.3) and every refusal by name → the plain build → this pass (its objects
and link are the probed build) → the runs. The plain build and the probed objects are written
to a freshly made, randomly named folder outside the target (`$TMPDIR/ruharness-map-<random>`),
never in reach of the copy's `-I` folders, `..` included; the plain program is moved into the
build folder once the probed link is done.

Cost: a file whose notes all compile costs its own compile (the probed build's) and one
sandboxed process; the listing runs now write their preprocessed text (≈ the size of each
file's preprocessed source, e.g. 9 MB for sqlite3.c). A rejected note costs one more compile of
its file, plus one compile of each file that enters a changed header. Measured with the store
note on sqlite3.c: compile time +3 %, peak memory +15 MiB, object +12.5 %; a probed run +2 %
over plain. A probed run keeps the same `timeout_secs` as plain: a scenario near the limit can
time out only when probed, recorded as `probe_agrees = false`.

### 3.5 The probe's own build

- The runtime is compiled without the target's include folders: its reads are system headers
  only.
- The probe header is left out of the copy's lists only as the `-include` it is (its exact
  path); found any other way, it is a read only the copy makes — refused as today.
- The program is refused by name if it defines a function the runtime imports (the runtime
  object's undefined symbols, `nm -u`, compiled with `-fno-builtin`: `open`, `close`, `fstat`,
  `mmap`, `getenv` and the like) — "the program defines open(), which the probe's runtime also
  uses; the features map cannot map it" — because the runtime's setup runs before `main` and
  would call the program's own.

### 3.6 The note and the runtime

The note is `__ruharness_seen[N] = 1;` — one store, no call. `__ruharness_seen` is a pointer
that starts at a static array in the program, so a note that runs before setup still lands
somewhere. Before each probed run the harness creates the notes file in the run's temp dir —
`$TMPDIR/.ruharness-notes`, N zero bytes (the path comes from `TMPDIR`, which the plain runs
get too: no environment variable only the probed run has). A constructor opens that file
(`O_RDWR | O_CLOEXEC | O_NOFOLLOW`, never creating or truncating it), checks that its size is N,
maps it `MAP_SHARED`, points `__ruharness_seen` at the mapping, then sets in it every byte set in
the static array (a merge — a second image of the program, re-executed or spawned with its
environment, adds its notes to the first's instead of wiping them), and closes the descriptor.
After the run the harness reads exactly N bytes, each 0 or 1 (anything else: "notes
unavailable"); byte N set = function N ran. Why:
- no call means no name the program could define at note time, and nothing its descriptors,
  `RLIMIT_NOFILE` or a closed-descriptor loop can break;
- a crash keeps every note made before it (the mapping's pages belong to the file);
- a smaller note changes inlining less: the review's builtin-flip sweep narrowed from about
  11–33 function sizes to 1–2 — not to none (§6).
Checked under the sandbox's run profile (scratchpad `spike-mmap/`): a note from a constructor
before setup, a loop closing every descriptor, and a crash after notes are all recorded.

### 3.7 What the person sees

- `map.json` gains `unwatched_reasons`: one `{file, id, reason}` per unwatched pair, an optional
  field (older harnesses ignore it). It is read strictly when present — each reason printable,
  at most 160 bytes, its pair in `unwatched` — and is display-only: never in prompts, events or
  harness-mcp. SCHEMAS.md's map.json entry names it and the `probe` input.
- The reasons, in plain words: "the parser could not read its definition"; "its body is not a {
  } block"; "a # line between its head and its body"; "its body's brace is inside #if"; "its
  body is inside a macro argument that becomes a string"; "read as data by <file>"; "a note at
  its start does not compile: <message>"; "found by elimination: <message>"; "an error from
  another function's note landed in it: <message>" (a cascade); "the copy of this file did not
  compile with notes: <message>".
- `harness features map` prints "Checking where the notes compile…" with progress, then one
  line per reason kind with a count and one example, e.g. `features: 3 function(s) unwatched —
  a note at their start does not compile (src/fp.c, kernel: '#pragma STDC FENV_ACCESS' can only
  appear at file scope or at the start of a compound statement)`.
- The cockpit, where it now says a function is unwatched ("(the probe could not put a note in
  it)"), shows that function's reason instead.
- The map's inputs gain `probe` (this design's version, `compiler-guided-1`); a map made before
  this change reads "made by an older probe — map again", not current.

### 3.8 gcc

The same pass on gcc (the Linux CI job): the flags of §3.4 step 1; gcc's include chain ("In
file included from <file>:<line>," then "from …:", innermost first); `#line` files to
elimination (step 3); the three linker forms (step 5). gcc accepts a statement in a naked
function on some targets and ignores some pragmas the clang judge rejects: on gcc, §2's judge
facts are re-run by the map tests of §4 on the Linux job, and a naked function is watched —
a crash it causes shows as `probe_agrees = false` (named in §6).

## 4. Tests and checks

Map-level tests (each rule has one that fails without it); clang-only cases skip on gcc with a
printed note, and gcc message forms are unit tests on recorded text:
- §3.1: rule 1 (`r2_under_error.c` of the review, both configurations); rule 2; rule 3 in each
  form — `#ifdef`, `#ifndef`, `#if X`, `#  ifdef`, `/* c */ #ifdef`, `%:ifdef`, with and without
  the define — and a parameter list split by `#ifdef` (watched);
- §3.2: a brace in a skipped branch the text rule is made to miss (the check unwatches it);
- §3.3: `#embed`, `# embed`, `__has_embed`, `.incbin` of another top-level file's source;
  `__has_include` only (notes out, no refusal); a stringized note; a difference only the text
  comparison catches (refused by name);
- §3.4: a pragma macro in each spelling (one round); 200 functions opening with a pragma macro
  across a header and the file, at the real mirror path (well over 8 KiB of errors: ≤ 2 rounds,
  exactly those unwatched, no fallback — a mutation capping stderr at 8 KiB must fail it); ten
  functions on one line with only the last rejected, with a tab and a multibyte character before
  the error column; two rejected pragmas in one included file (one unwatched each, no
  elimination); a bison-shaped `#line N "x.y"` and a renumbering `#line N`; an error through
  two include levels and through an unprobed `.inc`; an error no body holds (elimination finds
  the one note); the `asm` "i" operand; the C99 `inline` link case and a callee's note tipping
  it; a compile with no located error (refused after one extra compile); a compile that times
  out (refused); the bounds; a copy that fails without notes (refused with its words);
- §3.5: the runtime without `-I`; the probe header only by its path; a program defining `open`
  or `getenv` (refused by name); `__has_include("../../../plain")` finding nothing;
- §3.6: notes kept across a closed-descriptor loop, a crash, a self re-exec with the
  environment; a strict read of a short or bad notes file; the plain and probed runs' identical
  environment;
- §3.7: reasons in `map.json` for every kind, read strictly; an older map reads "map again".
- The two probe-level test functions of today are rewritten: the removed rules' expectations
  become "watched"; the kept rules' fixtures stay.
- Mutation checks of each rule above. Re-runs through `harness features map`: both corpora,
  the five pragma matrices, the checkers' repro suites, zopfli's map (the same functions per
  scenario, fewer unwatched); reported with the build and run time for sqlite3.c and zopfli.

## 5. The last check's 45 findings (scratchpad `check7/findings.md`, in its order)

| # | finding | disposition |
|---|---|---|
| 1 | `#embed` of a watched file | §3.3 steps 1 and 4 |
| 2 | the copy's build sees `plain` | §3.4 "Order": outputs in a random folder outside the target |
| 3 | the probe header dropped wherever found | §3.5 |
| 4 | `.incbin` of a watched file | §3.3 step 1 |
| 5 | the runtime compiled with the target's `-I` | §3.5 |
| 6 | the note shifts `__builtin_COLUMN()` | residual §6 |
| 7 | a Unicode space in a path makes the map refuse | fix: the make-rule reader splits on ASCII blanks only |
| 8 | the head-error rule unwatches half of sqlite | removed (§3.1) |
| 9 | "features need a program with one main()" printed for any refusal | fix: only when the facts' main count is the refusal's cause |
| 10 | a program-side list misread worded "does not build" | fix: "the compiler's list of what <file> reads cannot be read back: …" |
| 11 | the listing re-run without `-H` after a timeout | fix: no re-run after a timeout or overflow |
| 12 | the runner's poll adds 25–50 ms to short children | fix: tool runs ramp from 1 ms to at most 8 ms |
| 13 | an unreadable file in `source_dir` refuses the map | fix: left out of the copy; if the program reads it, the listings differ and the map refuses by name |
| 14 | a header's function reached through a file link noted under the link's path only | residual §6 |
| 15 | a struct in a macro read as a function | the compiler decides (§3.4): its note does not compile |
| 16 | an absolute include of a function-less header refused | fix: refused only when the file has notes |
| 17 | `source_dir = "."` accepts no `include_dirs` | fix: normalise both paths before the prefix test |
| 18 | an unsandboxed scenario's forked child kept or killed by chance | fix: scenario runs keep the fixed 50 ms poll |
| 19, 20 | two flaky timing tests (a fresh binary's first sandboxed exec under load) | fix: those tests' timeouts |
| 21 | a huge `timeout_secs` overflows the deadline | fix: clamped at load, with a message |
| 22, 23 | the poll ramp's later wake-ups; stale doc comments on the poll | fixed with #12 and #18 |
| 24–30 | split-exception gaps, keyword-led splits, the leading-edge walk, `FENV_ON;` read as a type | removed rules (§3.1); the compiler decides |
| 31 | `#pragma clang attribute push(naked)` | the compiler decides on clang; gcc: §3.8 |
| 32 | `__label__` behind a macro | the compiler decides |
| 33 | the head-error rule's cost | removed (§3.1) |
| 34 | one ERROR node unwatches every later function | residual §6 (rule 1) |
| 35 | any `#pragma` at a body's start unwatches | removed; the compiler decides |
| 36, 37 | typedef-led and `return` splits unwatched | removed; the compiler decides |
| 38 | §5.3's costs left out the costliest rules | §6 names the costs that remain |
| 39 | a no-break space in the target's path | fixed with #7 |
| 40 | the copy finding the probe header or `plain` not refused | §3.5 and §3.4 "Order" |
| 41 | the lone-name rule's cost unlisted | removed |
| 42 | `ends_its_line` misreads a comment | removed |
| 43 | a loop macro opening a body unwatched | removed; the compiler decides |
| 44 | the split exception's untested parts | removed |
| 45 | "a listed path that does not resolve" has no killing test; the `-H` order and depth are no longer exercised | fix: a map test for each (`#pragma once` through a file link for the order) |

## 6. Residuals and costs (named, each with what would change it)

- **A definition the parser cannot read** stays unwatched (rule 1): 115 of the extension
  files' functions (nkf.c 109 of 140) and the sqlite3 gem's `database.c` (34 of 37). Revisit
  if a preprocessing scan frontend (libclang) lands.
- **Rule 3's cost**: the 7 `PyInit_*` functions of the extension files (their heads split by
  `#if`); 0 on the corpus.
- **`__has_include` of a probed file** costs that file's notes (§3.3 step 1).
- **Inlining-dependent choices**: a note makes its function a little larger; at the exact
  inlining threshold, `__builtin_constant_p` or `__builtin_object_size` inside it can fold
  differently, so the copy runs other functions with identical output. The store note narrows
  the window to 1–2 function sizes in the review's sweep; it does not close it.
- **Cascades**: an error from one note that lands in another function's body takes that note out
  too (reason says so; a cost, never a failure).
- **A function in a header reached through a file link** is noted under the link's path; under
  its real path it reads as not run.
- **`__builtin_COLUMN()`** on a body's first line reads a different column in the copy.
- **A program that re-executes itself without its environment**, or exits inside its own
  constructor before the runtime's setup, loses those notes; a thread started by an earlier
  constructor can lose a note made during setup's switch to the mapping.
- **gcc**: naked functions are watched (a crash shows as `probe_agrees = false`); `#line` files
  cost elimination compiles.

## 7. What is removed

From `harness-scan`: `blocks_note`, `leading_run_blocks`, `opens_with_error`,
`split_by_directive`, `continues_a_split`, `lone_macro`, `branch_is_empty`, `ends_its_line`,
`parameter_list`, the naked-head and head-error checks; `probe_point`'s unused `declarator`
binding; `FnDef.probe_at`'s doc comment rewritten to §3.1. The probe-level tests of the removed
rules are rewritten (§4). From the runtime: the append-an-id writer, the descriptor ≥ 900 logic
and the reopen-once logic.

## 8. The build's own premise re-run

Before the steps are built on it, the build re-runs §2's four inputs and the review's repro
folders through the real `harness features map` (not the prototype), and replaces §2's table
with those numbers.

## 9. Review record

### 9.1 The draft (777cd27) — 37 findings, and how revision 1 was checked

| finding (plain words) | where it is answered now |
|---|---|
| a `{` inside `#if` folded into the declarator: the note sits in a skipped branch (two angles) | §3.1 rule 3 (depth form, every directive spelling) and §3.2 (the compiler's preprocessing as its net) |
| a note inside a macro argument that becomes a string | §3.3 step 2 |
| syntax-only checks miss code-generation and link errors (three angles) | §3.4 steps 1 and 5, including a callee's note that tips inlining |
| error locations wrong for nested includes, `#line` files, several functions on one line (two angles) | §3.4 steps 1, 3, 4: physical locations, byte placement with the compiler's line model, the chain printed last, gcc's `#line` files to elimination |
| inlining-dependent builtins pick other functions | §6, with the sweep's real result (narrowed, not closed) |
| the runtime calls names the program can define, and loses notes on closed descriptors | §3.5 (refused by name) and §3.6 |
| the data-read rule misses spellings and refuses harmless `__has_include` | §3.3 step 1 (listed but not entered, `.incbin` by path) and §6 |
| the runner returns only 8 KiB of errors and reads a timeout as an ordinary failure (three angles) | §3.4 step 2 and its §4 test with a mutation |
| the fallback copied back too little and blamed the original | §3.4 steps 4 and 6 |
| gcc's flags, chain order, columns and macro locations; `-ferror-limit=0` fails on gcc (three angles) | §3.4 step 1, §3.8 |
| the reason text and file names undefined | §3.4 step 3 |
| re-checks unbounded and unordered | §3.4 steps 6–7 |
| round 1 can be the existing listing run | §3.3: one listing run gives the list, the headers and the preprocessed text |
| a compile failing with no error line | §3.4 step 2 |
| the cost of watching ~1 500 more sqlite functions unstated | §3.4 "Cost" with measured numbers |
| the include chain's innermost mirror file may be an unprobed `.inc` | §3.4 step 3: the innermost **probed** file |
| rule 1 is the largest loss, unnamed | §3.1 rule 1, §6 with its numbers |
| the tests guard none of the remaining rules | §4, §7 |
| what the person sees explained only compiler rejections | §3.7: a reason for every kind, the cockpit line |
| the reasons log lived in a folder every run deletes | §3.7: reasons in `map.json`, read strictly |
| 7 of the 45 earlier findings without a disposition; the table's numbers wrong | §5, renumbered in the check's order |
| the pass's place relative to the plain build; `plain` reachable from the mirror | §3.4 "Order" |
| `parameter_list` left dead; `probe_at`'s doc | §7 |
| an old map read as current | §3.7 `probe` input |
| the text leaned on material a newcomer cannot reach | §1 (words defined), plain reasons, no review codes |

### 9.2 New in revision 1 (20, each confirmed by two verifiers)

| finding | answer |
|---|---|
| a second image of the program wiped the notes | §3.6: the harness creates the file; the runtime merges, never truncates |
| the runtime's setup still calls `open`, `mmap`, `getenv` … by name | §3.5: refused by name |
| the `#embed`/`.incbin` text trigger missed spellings and other files' `.incbin` | §3.3 step 1 (listed but not entered; `.incbin` by path in any file) and step 4 (the text comparison) |
| notes lost during setup's switch if an earlier constructor's thread runs | §6 |
| the probed run got an environment variable the plain runs did not | §3.6: the path comes from `TMPDIR` |
| halving undefined for two notes giving the same error | §3.4 step 4: a monotone prefix search, one culprit per search |
| the link's object order unstated | §3.4 step 5 |
| byte placement needs the compiler's line and column model | §3.4 step 3 and gcc's byte columns (step 1) |
| rule 3 too broad (110 needless losses) and too narrow (other directive spellings) | §3.1 depth form; §3.2 |
| the constructor's callees recorded as run | §3.5 |
| the text trigger unprobes whole files for harmless text | §3.3 step 1 no longer reads text for `#embed` |
| worst-case elimination ~9 minutes, no pass-wide bound, no progress | §3.4 step 6 |
| gcc: repeated errors, display columns, fragile detection | §3.4 steps 1 and 4 |
| a self re-exec and an exit inside a constructor lose notes | §3.6 merge; §6 |
| `plain` reachable by `..` from the mirror | §3.4 "Order" |
| the 45-findings table named the wrong findings | §5 renumbered |
| the waiter thread lacked overflow and kill paths | dropped: tool runs ramp to 8 ms (§5 #12) |
| reasons and refusals used words a person cannot act on | §3.7 |
| tests named that cannot be built on clang, or expected clang on gcc | §4: clang-only cases skip on gcc; gcc forms as unit tests |
| the happy path is not free | §3.4 "Cost" |
