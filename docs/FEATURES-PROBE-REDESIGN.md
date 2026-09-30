# Features map — the compiler-guided probe (design)

Status: **draft for review** (2026-09-30). Decided by the person: DECISIONS.md, "Features map:
PROPOSAL … let the compiler decide". Replaces the "Unwatched" rules of FEATURES-DESIGN.md §5.3;
everything else in §5 stays.

## 1. What changes, in one paragraph

The features map runs each scenario on a scratch copy of the C program in which every watched
function starts with a one-line note ("I ran"). Today a set of syntax rules guesses, from the
unpreprocessed source, whether a note at a function's start would compile; macros make that
guess impossible to get right, and seven checks in a row found new wrong guesses in both
directions (a function watched whose copy then fails to build — the whole map fails — and
thousands of ordinary functions left unwatched). The new probe stops guessing: it puts a note
in every function where a note can be placed at all, asks the compiler whether the copy
compiles, and takes the note out of exactly the functions the compiler's errors land in —
recording the compiler's own words as the reason.

## 2. Premise, run end to end (scratchpad `proto/`)

A prototype (the scan crate with the guessing rules switched off and a "skip these functions"
list; a Python driver doing the compile-and-retry of §3.2 with `cc -fsyntax-only
-ferror-limit=0` and the copy's own flags):

| input | the copy builds (today → prototype) | functions left unwatched (today → prototype) | rounds |
|---|---|---|---|
| 24 adversarial repro files of the last three checks (no defines) | 17/24 → **24/24** | 2 930 → 2 567 | ≤ 2 |
| the same, `-DA` (25 compile) | 18/25 → **25/25** | 2 140 → 1 999 | ≤ 2 |
| the 294-file corpus — the 42 that compile as they are (sqlite ×2, blake3, tree-sitter, zopfli, …) | 42/42 → 42/42 | **3 044 → 6** | 1 |
| 128 Python/Ruby extension files — the 37 that compile with the Python headers | 37/37 → 37/37 | **1 162 → 129** | 1 |

sqlite3.c goes from ~1 500 unwatched functions to 0, its copy still compiling. On the
adversarial files the functions left unwatched are the ones the compiler rejects — pragma
macros in every spelling, `naked`, `__label__`, an `#include` of a pragma. On real code the
compiler rejected no note at all: every function today's rules left unwatched there was a
needless loss.

Why the compiler is a sound judge here (Apple clang 21, checked): every pragma that must open
a block is an **error** when a statement comes first — `STDC FENV_ACCESS`, `FP_CONTRACT`,
`CX_LIMITED_RANGE`, `FENV_ROUND`, `float_control`, every `clang fp` form; a naked function
with a statement is an error; `__label__` after a statement is an error. The pragmas a
misplaced note leaves silently accepted (`fenv_access(on)` without `-fms-extensions`, `GCC
optimize`, `clang optimize off` inside a body) have no effect inside a body in either build.
The run-time guard stays as it is: each scenario runs plain, probed, plain, and a probed run
whose output differs is recorded (`probe_agrees = false`), never hidden.

## 3. The design

### 3.1 Where a note can go (the hard rules — the only syntax rules left)

A function the facts record gets a note right after its body's `{` unless:
1. the parser could not read the definition (it lies under an ERROR node — the body's bounds
   are then a guess);
2. its body is not a `{ … }` block, or the first byte of the body is not a real `{`;
3. a line starting with `#` stands between the declarator and the body (a brace inside `#if`:
   the note would exist on one branch only).

Removed (the compiler decides): the parse-error-in-the-head rule, the `naked` word rule, and
every leading-run rule (pragma spellings, bare names, names alone on a line, leading edges,
nested definitions, splits and their exceptions, lone macros, `__label__`, `#include`).
`probe_source` stays pure; it gains the list of `(file, id)` pairs **not** to watch.

### 3.2 The compile-and-retry pass (featuremap.rs, before the probed build)

For each translation unit of the copy (each top-level `.c` of the mirror — headers are checked
through the units that include them):

1. `cc <the copy's flags> -fsyntax-only -ferror-limit=0 <unit>` (the same includes, `-include
   fnprobe.h`, `-fmacro-prefix-map`, `-DRUHARNESS_FNPROBE_N`, `-O2`, `-ffp-contract=off`),
   through the tool runner and its sandbox profile, as every build.
2. It compiles: next unit. It fails: read every `error:` line. Each names a file of the mirror
   and a line — its own location, or, for an error inside a file the mirror does not probe (a
   system header), the innermost `In file included from <mirror file>:<line>` above it. The
   notes never move a line, so that line is the original's. The watched function of that
   file whose definition spans the line (the innermost, for nested spans) loses its note; its
   reason is the error's text (first line, control characters shown as `?`, cut at 160
   bytes).
3. Re-probe the files that lost notes and compile the unit again. At most 8 rounds per unit.
4. If an error lands in no watched function, or 8 rounds pass, the files of that unit's
   errors are copied back **unprobed** (every function in them unwatched, reason "the copy
   of this file does not compile with notes: <error>"), and the unit is compiled once more.
   If it still fails, the map refuses, as today, with the compiler's words — the original
   program itself does not build this way.

A unit that compiled in an earlier round is compiled again only if a file it includes was
re-probed. Every top-level `.c` compiles at least once more than today (`-fsyntax-only`,
without code generation); a unit whose notes all compile costs one extra compile.

### 3.3 A probed file read as data is refused (the silent case the last check found)

The copy reads each probed file with its notes. A program that reads such a file as **data**
— `#embed "unit.c"`, `__has_embed`, `.incbin` in inline assembly — gets different bytes in the
copy, and the map would describe a program nobody built. `reads()` already lists what each
compile reads (`-M`) and enters (`-H`): a probed file of the mirror that a compile lists but
never enters, and that is not the unit itself, is refused by name, before any build ("the
program reads <file> as data; its copy has notes in it"). `.incbin` is not listed by `-M` at
all: named as a residual (§5).

### 3.4 What the person sees

- `harness features map` prints, before the scenarios, one line when notes were taken out:
  `features: <n> function(s) unwatched — the compiler rejected a note at their start (e.g.
  <file>:<function>: <reason>)`. The map's `unwatched` list is unchanged in shape (the
  `(file, id)` pairs of `map.json`, schema version 1 — no new field); the reasons live in the
  map's log in the gitignored build folder (`migration/build/.features/unwatched.txt`).
- The cockpit's "(n unwatched)" words and the "not proof" wording of FEATURES-DESIGN §8 are
  unchanged: fewer functions will carry them.

### 3.5 Tests and checks

- Map-level tests replace the probe's rule tests: a program whose functions open with a
  pragma macro (object-like, function-like, `_Pragma`, inside `#else`), a `naked` function, a
  `__label__`, an `#include` of a pragma — the map succeeds, exactly those functions are
  unwatched with the compiler's words, the others watched.
- A file whose errors no function holds falls back unprobed; a unit that fails even unprobed
  is refused with the compiler's words.
- The `#embed` refusal (§3.3), with the checker's repro.
- Mutation checks of: the error-line parsing (own location, the include chain), the
  innermost-span choice, the round bound, the fallback, the data-read refusal.
- Re-runs: both corpora and the five pragma matrices must build with no failures; the
  unwatched counts reported.

## 4. The other findings of the last check

The check of the seventh pass confirmed 45 findings (scratchpad `check7/findings.md`). Beyond
the probe's rules (made moot by §3) and the data read (§3.3), they are fixed in the same build
where small, or named as residuals:
- wording: a program-side listing failure worded "does not build"; a refusal reported as
  "features need a program with one main()"; an absolute include of a function-less header
  worded "would run unwatched";
- the listing re-run without `-H` after a **timeout** (it should not re-run: the FIFO case
  would wait twice);
- a Unicode space in a path (the make-rule reader splits on it): read only ASCII blanks as
  separators;
- the runner's poll ramp: its doc comments, the phase shift, the root cause of two flaky
  timing tests (the first sandboxed exec of a fresh binary under load exceeds a 1–2 s test
  timeout) — the tests' timeouts, not the harness;
- an unreadable file in `source_dir` refusing the map (leave it out of the copy, as a
  control-character name is);
- a huge `timeout_secs` overflowing the deadline (clamp at load);
- the copy seeing the `plain` binary built after the listings, and the probe header dropped
  wherever it is found (drop it only as the `-include`, by its exact spelling);
- the probe runtime compiled with the target's `-I` dirs (compile it without them).

## 5. Residuals (named)

- `.incbin` of a probed file (not listed by `-M`).
- An error from one note that cascades into another function's lines unwatches that function
  too (a cost, never a failure).
- A `__builtin_COLUMN()` on a body's first line reads a different column in the copy.
- gcc: the same pass works (the same `file:line: error:` and `In file included from` forms);
  pragma diagnostics differ in wording, not in kind — to be run on a Linux lane.

## 6. What is removed

From `harness-scan`: `blocks_note`, `leading_run_blocks`, `opens_with_error`,
`split_by_directive`, `continues_a_split`, `lone_macro`, `branch_is_empty`, `ends_its_line`,
the naked-head and head-error checks, and their unit tests (replaced by §3.5's map-level
tests). About 400 lines.
