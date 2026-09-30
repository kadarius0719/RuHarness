# Features map — the compiler-guided probe (design)

Status: **revision 1, for its check** (2026-09-30). Decided by the person: DECISIONS.md,
"Features map: PROPOSAL … let the compiler decide". Replaces the "Unwatched" rules and the
probe runtime of FEATURES-DESIGN.md §5.3–§5.4; the rest of §5 stays. The draft (777cd27) had
an adversarial review from four angles — soundness, the retry's mechanics, cost, and what the
person sees — whose 37 findings were each reproduced by two independent verifiers; §8 says how
each is resolved.

## 1. What changes, in plain words

The features map runs each scenario on a scratch copy of the C program in which every watched
function starts with a small note ("I ran"). Today a set of syntax rules guesses whether a note
at a function's start would compile; macros make that guess impossible to get right, and seven
checks in a row found wrong guesses both ways — a function watched whose copy then fails to
build (the whole map fails), and thousands of ordinary functions left unwatched.

The new probe stops guessing:
- it puts a note in every function where one can be placed at all (§3.1);
- it compiles the copy, unit by unit, for real, and takes the note out of exactly the functions
  the compiler's errors land in, recording the compiler's words (§3.2);
- a few things the compiler cannot see are handled by name (§3.3–§3.5);
- the note itself becomes a one-byte store into memory the harness reads back, instead of a
  call that writes to a file (§3.6).

## 2. Premise, run end to end (scratchpad `proto/`, and the review's experiments)

A prototype — the scan crate with the guessing rules off and a "skip these" list, and a driver
that compiles, takes out the notes the errors land in, and tries again:

| input | the copy builds (today → prototype) | functions left unwatched (today → prototype) | rounds |
|---|---|---|---|
| 24 adversarial repro files of the last three checks | 17/24 → **24/24** | 2 930 → 2 567 | ≤ 2 |
| the same with `-DA` (25 compile) | 18/25 → **25/25** | 2 140 → 1 999 | ≤ 2 |
| 42 files of the 294-file corpus that compile as they are (sqlite ×2, blake3, tree-sitter, zopfli, …) | 42/42 → 42/42 | **3 044 → 6** | 1 |
| 37 Python/Ruby extension files that compile with the Python headers | 37/37 → 37/37 | **1 162 → 129** | 1 |

sqlite3.c goes from ~1 500 unwatched functions to 0. On real code the compiler rejected no
note: every function today's rules left unwatched there was a needless loss.

What the compiler judges, and what it does not: it tells whether a note **compiles** where it
stands. On this clang every pragma that must open a block (`STDC FENV_ACCESS`, `FP_CONTRACT`,
`CX_LIMITED_RANGE`, `FENV_ROUND`, `float_control`, every `clang fp` form) is an error after a
statement, as are a statement in a naked function and `__label__` after a statement. It does
**not** tell whether the copy is the same program: a note in an `#if` branch the build skips
is never compiled (§3.1 rule 3), a note inside a macro argument that becomes a string compiles
and changes the data (§3.3), a file read as data changes its bytes (§3.4), and a note that
changes inlining can change what an optimisation-dependent builtin picks (§3.6, §6). Those are
handled by name. The run-time guard stays: each scenario runs plain, probed, plain, and a
probed run whose output differs is recorded (`probe_agrees = false`) — it covers output
differences only.

## 3. The design

### 3.1 Where a note can go (the only syntax rules left)

A function the facts record gets a note right after its body's `{` unless:
1. the parser could not read the definition (it lies under an ERROR node — the body's bounds
   are then a guess). This is the largest remaining loss (129 of the 1 162 in the extension
   files); named in §6.
2. its body is not a `{ … }` block, or the body's first byte is not a real `{`;
3. a line starting with `#` stands anywhere between the **definition's first byte** and its
   body's `{` — not only between the declarator and the body: the parser can fold an `#ifdef`
   into the declarator, and a `{` inside `#if` means the note would exist on one branch only,
   invisible to the compiler in the other. The note shares the `{`'s line, so a `{` that is
   compiled always carries a compiled note — this rule is the whole of "a note in a skipped
   branch";
4. its name is one the probe runtime itself calls (none, once §3.6's runtime calls nothing
   through the program's names — kept as a check: the runtime object's imported names, `nm -u`,
   must not name a function the facts record, else that function is unwatched with "the
   probe's runtime uses a function of this name").

Removed (the compiler decides): the parse-error-in-the-head rule, the `naked` word rule and its
`parameter_list` helper, and every leading-run rule. `probe_source` stays pure; it gains the
list of `(file, id)` pairs **not** to watch and reports, for every note it places, the copy's
byte range of that function's body.

### 3.2 Compile, and take out what does not compile

The copy's build becomes one real compile per unit (each top-level `.c`), then one link:

1. `cc <the copy's flags> -c -o <build>/probed/<n>.o <unit>` — full code generation at `-O2`
   (a syntax-only check misses errors a note causes there: an `asm` "i" operand that is no
   longer a constant once the function stops being inlined — shown in the review). The flags
   add: for clang `-ferror-limit=0 -Xclang -fno-diagnostics-use-presumed-location` (every
   location is the physical file and line of the copy, `#line` ignored, in the error lines and
   in the include chain); for gcc `-fmax-errors=0` (gcc has no such location flag: an error
   whose file is not a probed file of the mirror goes to step 4). Which compiler `cc` is comes
   from its `--version` line (the toolchain line verify already records).
2. The runner returns the compile's whole stderr (up to the output cap) and how it ended. A
   compile that timed out, overflowed or was killed is a refusal — "the copy's compile of
   <unit> did not finish: …" — never read as errors.
3. It compiles: the next unit. It fails: each `error:` line (and `fatal error:`) gives a file,
   line and column. If the file is a probed file of the mirror, the error is placed by its
   exact position: the innermost watched function whose body range (from the probe) holds that
   byte loses its note. If the file is not probed (a system header, an unprobed `.inc`), the
   include chain above the error (clang: "In file included from <file>:<line>:", outermost
   first; gcc: "In file included from <file>:<line>," then "from …", innermost first) gives
   the innermost **probed** file and its line, placed the same way. The reason recorded is the
   error's text: its first line, the path shown relative to the target, control characters as
   `?`, at most 160 bytes.
4. Errors that cannot be placed — no position, a presumed name (gcc with `#line`), a position
   in a probed file outside every body — are found by elimination instead: the unit's notes
   are halved, one half taken out, the unit compiled; the half whose removal makes the error go
   away is halved again, until one note is left, which is taken out with the reason "found by
   elimination: <the error>". Each round removes at least one note, so the pass ends.
5. The link: an undefined symbol that names a watched function (a C99 `inline` function whose
   note stopped it being inlined — shown in the review) takes that function's note out, and the
   units that define or call it are compiled again. Any other link error is a refusal with the
   linker's words.
6. Bounds: at most 8 placed rounds and 64 compiles per unit; past either, every probed file the
   unit includes goes back unprobed (all their functions unwatched, reason "the copy of this
   file did not compile with notes: <error>") and the unit is compiled once more. If it still
   fails with no notes at all, the map refuses — "the copy of <unit> does not compile even
   without notes: <error>" — which means the mirror differs from the original (the plain build
   compiled it), a harness bug, never the program's.
7. A probed header's notes taken out: every unit that includes it (from its `-H` list) is
   compiled again. Units are handled in sorted order; the result is the same for the same
   inputs.

Order: the listings and every refusal by name (FEATURES-DESIGN §5.3) → the plain build → this
pass (its objects and link are the probed build) → the runs. Cost: a unit whose notes all
compile costs nothing extra (its object is the probed build's); a rejected note costs one more
compile of its unit (and of the units that include its file).

### 3.3 A note inside a macro argument that becomes a string

`#define SHOW(x) puts(#x)` then `SHOW(void shade(void) { … })` — the parser sees a definition,
the note compiles, and the program prints different text. Before the compiles of §3.2, each
unit is preprocessed once (`cc -E`, the copy's flags) and the output read for note text inside
a string literal; each such note's function loses its note, reason "its body is inside a macro
argument that becomes a string". (The review showed the check finds the case.)

### 3.4 A probed file read as data

`#embed "unit.c"`, `__has_embed`, `.incbin "unit.c"` read a file's bytes: in the copy those
bytes carry notes, and the map would describe a program nobody built. If any file a unit reads
(its `-M` list, mirror files only) contains `#embed`, `__has_embed` or `.incbin`, every probed
file that unit lists goes back unprobed, reason "read as data by <unit>" — a whole unit's
notes for a rare construct, never a refusal. `__has_include` of a probed file reads no bytes
and triggers nothing.

### 3.5 The probe's own build

- The runtime is compiled without the target's include folders (only its own header): its
  reads can then be nothing but system headers.
- The copy never sees `plain`: the plain program is built in `<build>/plain-build/`, outside
  every folder the copy's `-I` list reaches.
- The probe header is left out of the copy's lists only as the `-include` it is (its exact
  path); found any other way (a lookup from the mirror), it is a read only the copy makes —
  refused as today.

### 3.6 The runtime: a one-byte store into shared memory

The note becomes `__ruharness_seen[N] = 1;` — one store, no call. `__ruharness_seen` is a
pointer that starts at a static array in the program (so a note that runs before setup still
has somewhere to go); a constructor opens the notes file in the run's temp dir (the path from
`RUHARNESS_FNPROBE_OUT`, `O_CLOEXEC | O_NOFOLLOW`), sizes it to N bytes, maps it
`MAP_SHARED`, copies the static array in, points `__ruharness_seen` at the mapping and closes
the descriptor. The harness reads the file after the run: byte N non-zero = function N ran.
Why:
- no call means no name the program could define (the review showed a program defining
  `strlen` recorded as running it), and nothing the program's descriptors, `RLIMIT_NOFILE` or a
  closed-descriptor loop can break (both shown losing notes today);
- a smaller note changes inlining less (the review's builtin-flip experiment flipped at no
  function size with a store-only note, and at 11 sizes with today's);
- a crash keeps every note made before it (the mapping's pages belong to the file).
Kept from today: the runtime and its header are harness-owned C built with `cc`; the notes file
is read strictly (exactly N bytes, each 0 or 1 — anything else is "notes unavailable").
A program that re-executes itself loses the mapping for the new image, whose constructor maps
the file again when `RUHARNESS_FNPROBE_OUT` is still in its environment; without it, that
image's functions read as not run — named in §6.

### 3.7 What the person sees

- `map.json` gains `unwatched_reasons`: one `{file, id, reason}` per unwatched pair (an
  optional field — older readers ignore it; no schema version change). Every unwatched
  function has a reason: "the parser could not read its definition", "its body's brace is
  inside #if", "its body is inside a macro argument that becomes a string", "read as data by
  <unit>", "a note at its start does not compile: <error>", "found by elimination: <error>",
  "the copy of this file did not compile with notes: <error>", "the probe's runtime uses a
  function of this name".
- `harness features map` prints one line per kind before the scenarios, e.g. `features: 3
  function(s) unwatched — a note at their start does not compile (src/fp.c: kernel: '#pragma
  STDC FENV_ACCESS' can only appear at file scope or at the start of a compound statement)`.
- The cockpit shows the reason beside an unwatched function wherever it already says
  "unwatched".
- The map's inputs gain `probe` (this design's version, `compiler-guided-1`), so a map made
  before this change reads "made by an older probe — map again", not current.
- The words a person sees follow the plain-language rule: no review codes.

### 3.8 gcc

The same pass on gcc: `-fmax-errors=0`, gcc's include-chain form (innermost first), macro
errors placed at gcc's reported location or, when it is not a probed file, by elimination; no
physical-location flag, so `#line` goes to elimination too. Run on the Linux CI job.

## 4. Tests and checks

Each rule has a test that fails without it (map level unless marked):
- §3.1: each hard rule, including the brace inside `#if` folded into the declarator
  (`#ifdef`, `#ifndef`, with and without the define — unwatched, reason given) — probe-level
  fixtures kept for the rules that stay;
- §3.2: a pragma macro in each spelling (unwatched with the compiler's words, the others
  watched, one round); ten functions on one line with only the last rejected (exactly that one
  unwatched); a bison-shaped `#line N "x.y"` and a renumbering `#line N` (the right function);
  an error through two include levels and through an unprobed `.inc`; an error no body holds
  (elimination finds the one note); the `asm` "i" operand at code generation; the C99 `inline`
  link case; a compile that times out (refused, not read); the round and compile bounds; a unit
  that fails without notes (refused with its words); gcc's message forms (parsed from recorded
  text, a unit test);
- §3.3 stringized notes; §3.4 `#embed`, `__has_embed`, `.incbin`, and `__has_include`
  triggering nothing; §3.5 the runtime without `-I`, `plain` out of reach, the header by its
  path; §3.6 notes kept across a closed-descriptor loop, a program defining `write`/`strlen`,
  a crash after notes, a strict read of a short or bad notes file;
- §3.7 reasons in `map.json` for every kind; an older map reads "map again";
- mutation checks of each rule above; re-runs of both corpora, the five pragma matrices, the
  checkers' repro suites and zopfli's map (the same functions per scenario, fewer unwatched).

## 5. The last check's 45 findings (scratchpad `check7/findings.md`)

| # | finding | disposition |
|---|---|---|
| 1 | `#embed` of a watched file | §3.4 |
| 2 | the copy sees `plain` | §3.5 |
| 3 | the probe header dropped wherever found | §3.5 |
| 4 | `.incbin` of a watched file | §3.4 (text trigger) |
| 5 | the runtime compiled with the target's `-I` | §3.5 |
| 6 | `__builtin_COLUMN()` shifts | residual §6 (the store-only note still shares the brace's line) |
| 7, 37 | a Unicode space in a path | fix: the make-rule reader splits on ASCII blanks only |
| 8, 22 | macro words in a head leave half of sqlite unwatched | gone with the head-error rule (§3.1) |
| 9 | "features need a program with one main()" on any refusal | fix: that line only when the facts count ≠ 1 is the refusal's cause |
| 10 | a program-side list misread worded "does not build" | fix: "the compiler's list of what <unit> reads cannot be read back: …" |
| 11 | the listing re-run without `-H` after a timeout | fix: no re-run after a timeout or overflow |
| 12 | the poll's lag on short children | fix: tool runs wait on the child with a waiter thread (no poll); scenario runs keep the 50 ms poll |
| 13 | an unreadable file in `source_dir` refuses the map | fix: left out of the copy (as a control-character name); read by the program, the listings differ and the map refuses by name |
| 14 | a function in a header reached through a file link noted under the link's path only | residual §6 |
| 15 | a struct in a macro read as a function | the compiler decides (§3.2): its note does not compile |
| 16 | an absolute include of a function-less header refused | fix: refused only when the file has notes |
| 17 | `source_dir = "."` accepts no `include_dirs` | fix: normalise both paths before the prefix test |
| 18 | an unsandboxed forked child kept or killed by chance | fix: scenario runs keep the fixed 50 ms poll (the ramp only for tool runs — then a waiter thread, #12) |
| 19, 20 | two flaky timing tests | fix: their timeouts, not the harness (the first sandboxed exec of a fresh binary under load exceeds 1–2 s) |
| 21 | a huge `timeout_secs` overflows the deadline | fix: clamped at load, with a message |
| 23, 24 | the ramp's phase shift; stale doc comments | fixed with #12 |
| 25–31, 33–36, 38–42 | wrong guesses and costs of the syntax rules | gone with those rules (§3.1–§3.2) |
| 32 | `#pragma clang attribute push(naked)` | the compiler decides: a note in a naked function is an error |
| 43 | the split exception's untested parts | gone with the rule |
| 44 | "a listed path that does not resolve" has no killing test | fix: a map test |
| 45 | the `-H` order and depth comparison is no longer exercised | fix: a fixture where only the order differs (`#pragma once` through a file link) |

## 6. Residuals (named, each with what would change it)

- **A definition the parser cannot read** stays unwatched (rule 1) — the largest remaining loss
  (129 in the extension files). Revisit if a scan frontend that preprocesses (libclang) lands.
- **Inlining-dependent choices**: a note makes its function a little larger; a function right at
  the inlining threshold can stop being inlined, and `__builtin_constant_p` or
  `__builtin_object_size` inside it can fold differently, so the copy runs other functions
  with identical output. The store-only note (§3.6) removed every flip in the review's
  experiment; not proved impossible.
- **Cascades**: an error from one note that lands in another function's body takes that note
  out too (a cost, never a failure).
- **A function in a header reached through a file link** is noted under the link's path; under
  its real path it reads as not run.
- **`__builtin_COLUMN()`** on a body's first line reads a different column in the copy.
- **A program that re-executes itself without its environment** loses the notes of the new image.
- **gcc**: `#line` errors go to elimination (more compiles, same result).

## 7. What is removed

From `harness-scan`: `blocks_note`, `leading_run_blocks`, `opens_with_error`,
`split_by_directive`, `continues_a_split`, `lone_macro`, `branch_is_empty`, `ends_its_line`,
`parameter_list`, the naked-head and head-error checks, and the unit tests of those rules
(about 400 lines); the probe-level fixtures of the rules that stay are kept. From the runtime:
the append-an-id writer, the descriptor ≥ 900 dance and the reopen-once logic (§3.6 replaces
them).

## 8. Review record (the draft, 777cd27 — 37 findings, each reproduced by two verifiers)

| finding (plain words) | resolution |
|---|---|
| a `{` inside `#if` that the parser folds into the declarator: the note sits in a skipped branch, the function reads "not run" (found by two angles) | §3.1 rule 3 measured from the definition's first byte; test |
| a note inside a macro argument that becomes a string | §3.3 |
| a syntax-only check misses errors that code generation and linking find | §3.2 steps 1 and 5: real per-unit compiles and the link |
| error locations wrong for nested includes, `#line` files, and several functions on one line (found by two angles) | §3.2 steps 1, 3 and 4: physical locations, byte-exact placement, elimination |
| inlining-dependent builtins pick other functions | §3.6 store-only note; residual §6 |
| the runtime calls names the program can define, and loses notes when descriptors close | §3.6 |
| the data-read rule misses a file both included and embedded, and refuses harmless `__has_include` | §3.4: a text trigger, never a refusal |
| the runner returns only 8 KiB of a failed compile's errors, and reports a timeout as an ordinary failure (found by three angles) | §3.2 step 2 |
| the fallback copied back too little and blamed the original | §3.2 step 6 |
| gcc's flag, include-chain order and macro locations differ; `-ferror-limit=0` fails every gcc compile (found by three angles) | §3.2 step 1, §3.8 |
| the reason text and file name undefined | §3.2 step 3 |
| re-checks unbounded and unordered; one reading costs 17× more compiles | §3.2 steps 6–7 |
| round 1 can be the build itself | §3.2: the per-unit objects are the probed build |
| a compile failing with no error line misreported | §3.2 steps 2 and 4 |
| the cost of watching ~1 500 more sqlite functions unstated | §4 re-runs report the build and run time; the one-byte note is the cheapest possible |
| the include chain's innermost mirror file may be an unprobed `.inc` | §3.2 step 3: the innermost **probed** file |
| rule 1 is the largest remaining loss and is not named | §6 |
| the tests guard none of the remaining rules; the removal would delete their fixtures | §4, §7 |
| what the person sees explains only compiler rejections | §3.7: a reason for every unwatched function |
| the reasons log lived in a folder every run deletes | §3.7: reasons in `map.json` |
| 7 of the 45 earlier findings had no disposition | §5: all 45 |
| the pass's place relative to the plain build unstated | §3.2 "Order" |
| `parameter_list` left dead | §7 |
| a map made before the change reads as current | §3.7: the `probe` input |
| the text leaned on material a newcomer cannot reach | §1 and plain words throughout |
