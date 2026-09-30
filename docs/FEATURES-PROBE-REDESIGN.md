# Features map — the compiler-guided probe (design)

Status: **revision 3 — to be built** (2026-09-30). Decided by the person: DECISIONS.md,
"Features map: PROPOSAL … let the compiler decide". Replaces the "Unwatched" rules and the
probe runtime of FEATURES-DESIGN.md §5.3–§5.4; the rest of §5 stays. History: the draft
(777cd27) had an adversarial review from four angles (37 findings, each reproduced by two
verifiers); revision 1 (a14c807) was checked (4 fully resolved, 33 in part, 20 new); revision 2
(f345a49) was checked (17 of 57 fully answered, 40 in part, 14 new — every new finding confirmed
by two verifiers). This revision takes every concrete change those checks asked for (§9) and is
built without a further design round: what remains are details the build's tests, its code
review and the checks of its fix passes find better than another reading (DECISIONS.md).

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
3. its body's `{` sits in a conditional group its head is not wholly in. Scanning from the
   start of the function's **name** (the declarator's identifier) to its `{` — a directive
   line being one whose first characters, after blanks, comments and line splices, are `#` or
   the digraph `%:` — the note is one-branch only when an `#if`, `#ifdef` or `#ifndef` opened
   in that span is still open at the `{`, or when an `#else`, `#elif`, `#elifdef`,
   `#elifndef` or `#endif` of a group opened before the name appears in it (two heads, one
   body: the parser gives the body to one name). Reason: "a # line between its head and its
   body". A conditional opened and closed inside the head (a parameter list split by
   `#ifdef`) and one before the name (`#ifdef _WIN32 __declspec(dllexport) #endif`) do not
   count.
4. on gcc only: its head or attributes name `naked` — gcc can accept a statement in a naked
   function, where the note could overwrite a register its assembly reads.

Removed (the compiler decides): the parse-error-in-the-head rule, the `naked` word rule and its
`parameter_list` helper (on clang), and every leading-run rule. `probe_source` stays pure; it
gains the list of `(file, id)` pairs **not** to watch and reports, for each note it places, the
copy's byte range of the function's body.

### 3.2 Every note is compiled (the check behind rule 3)

Rule 3 is a text rule; its net is the compiler's own preprocessing. In the copy used for the
listing runs (§3.3) — and only there, never compiled — each noted body also carries, just
before its closing `}`, an end token `__ruharness_end_N`. In each top-level file's
preprocessed text, note N must appear as often as its end token: a shortfall means the note's
`{` sits in a branch the build skips while the body's end does not, and the note is taken out,
reason "its body's brace is inside #if". The end tokens are removed with the notes before the
comparison of §3.3 step 4.

### 3.3 The copy preprocesses to the same program

The listing runs of FEATURES-DESIGN §5.3 (what each compile reads, `-M`, and enters, `-H`)
become, for the program and for the copy alike, `cc <flags> -E -o <out>/<side>-<n>.i -MD -MF
<out>/<side>-<n>.d -H <file>` (`<out>` is §3.4's random folder, `<side>` is `program` or
`copy`) — one run that gives the dependency list, the headers entered and the preprocessed
text. Then, in order:
1. **Files read as data.** For each compile: a probed file that this compile lists (`.d`) and
   does not enter (`-H`) — files compared as files (canonical paths), leaving out the
   compile's own input and the probe header — is read without being included (`#embed` in
   any spelling, `__has_embed`, or only looked up with `__has_include`). It goes back
   unprobed, reason "listed but never included by <file> (#embed, __has_embed or
   __has_include)". `.incbin` names files the assembler reads: in each top-level file's
   preprocessed text, adjacent string literals are joined and decoded and `incbin` is matched
   in any case; the named file, resolved as the assembler does (the working folder, then each
   `-I`), goes back unprobed if it is probed; a name that cannot be resolved sends every probed
   file back. A directive built by an assembler macro is not seen (§6).
2. **Notes turned into text.** A note's text inside a string or character literal of the copy's
   preprocessed text (a macro argument that becomes a string): its function loses its note,
   reason "its body is inside a macro argument that becomes a string".
3. If 1 or 2 took notes out, the files they touched are re-probed and the listings of the
   top-level files that read them run again (at most twice; a third change refuses the map:
   "the copy keeps changing while it is checked — a fault in the harness, not your program;
   please report it").
4. **The same code.** For each top-level file, the copy's preprocessed text and the program's
   are compared as code: in the copy, the probe header's region (from the line marker that
   enters it — its exact path, flag 1 — through the marker that leaves it, both included) and
   every note and end token are removed; then on both sides every line marker and blank line is
   dropped. The rest must be equal, line by line. A difference refuses the map by name before any
   build — "the copy of <file> is not the same program near: <the first differing line, cut>"
   — the net for anything the steps above do not name. This is also where a file both included
   and `#embed`-ed, or a file embedding itself, is caught: the embedded bytes differ, and the
   map refuses (§6). The probe header must keep this sound: it includes nothing, defines no
   macro and uses no `__COUNTER__`; it only declares. Each mirror file keeps its original's
   modification time (gcc's `__TIMESTAMP__` reads it; `SOURCE_DATE_EPOCH` covers it on clang
   only).

The comparison replaces nothing in FEATURES-DESIGN §5.3's `reads()` (the same files by the same
names, the headers entered in order and depth): both run.

### 3.4 Compile, and take out what does not compile

The copy's build becomes one real compile per top-level file, then one link:

1. `cc <the copy's flags> -c -o <objects>/<n>.o <file>` at `-O2` — full code generation (a
   syntax-only check misses errors that code generation finds, e.g. an `asm` "i" operand that
   is no longer a constant once the function stops being inlined). The compiler kind comes from
   `cc -dM -E -x c /dev/null`: `__clang__` defined is clang; `__GNUC__` defined without
   `__clang__` is gcc, `__GNUC__` its version; neither refuses the map ("the features map
   knows clang and gcc; `cc` here is neither"). Flags added: clang `-ferror-limit=0 -Xclang -fno-diagnostics-use-presumed-location`
   (every location is the copy's physical file and line — `#line` ignored — in the error lines
   and the include chain); gcc `-fmax-errors=0 -ftrack-macro-expansion=0`, plus
   `-fdiagnostics-column-unit=byte` from gcc 11.
2. **The whole error output.** A new runner call returns a failed compile's whole stderr (up to
   the 64 MiB output cap) and how it ended; the 8 KiB excerpt stays for messages only. A
   compile that timed out is refused — "the copy's compile of <file> did not finish in <n> s —
   raise [oracle] timeout_secs"; one that overflowed the cap or was killed is refused with its
   own words. A compile that exited failing with no located error line (`clang: error: …`,
   `cc1: error: …`) goes to step 4's first compile with every note out.
3. **Placing an error.** Each `error:` or `fatal error:` line gives a file, a line and a byte
   column. The file (canonicalized, the canonical mirror prefix stripped, as `reads()` does) is
   a probed file: the error's byte offset (lines counted as the compiler does — `\n`, `\r\n`
   and a lone `\r` each end one) falls in the innermost body that still carries a note, and that
   note is taken out. The file is not probed (a system header, an unprobed `.inc`): the include
   chain printed last before the error (the compiler prints one only when it changes) gives the
   innermost **probed** file and its line, placed the same way. On gcc, an error in a top-level
   file whose probed files hold a `#line` or line-marker directive goes to step 4 (gcc has no
   physical-location flag). Lines, for placement, end at `\n`, `\r\n` and a lone `\r`, as
   both compilers count them; a byte-order mark shifts gcc's columns on line 1 — such an
   error goes to step 4. The reason is kind "compile" with the error's message (§3.7).
4. **Errors that cannot be placed** — a probed file position that no body still carrying a note
   holds, a presumed location, a chain with no probed file, a failure with no located error —
   are handled after the round's placed notes are out and the file is compiled again. If it
   still fails: it is compiled once with every note of the top-level file and of each probed
   file it reads taken out. Still failing, the map refuses: "the copy of <file> does not
   compile even without notes: <words> — a fault in the harness, not your program; please
   report it" (the plain build compiled it). Compiling, a search starts: the notes, ordered by
   file path then byte offset, are taken out as a growing prefix, and a binary search keeps a
   failing lower end and a compiling upper end until they meet at a note k — with the first
   k−1 out the file fails, with the first k out it compiles. Note k is taken out, kind
   "elimination" (§3.7), and the search repeats until the file compiles. When the chased error
   also appears with `-fsyntax-only`, the search's trial compiles use it. Taking a note out can
   add an error elsewhere through inlining, so note k is the note whose removal, with the notes
   before it out, made the file compile — not always the one that caused the error; each search
   removes one note, and step 6 bounds them. After the file compiles, each note the search took
   out is put back alone once and kept if the file still compiles.
5. **The link**: the runtime's object first (its setup then runs before any constructor of
   the program on Mach-O and, at default priority, on ELF), then the program's objects in the
   plain build's input order, whatever order the rounds compiled them in, then the plain
   build's libraries. Undefined symbols:
   (a) a symbol that is a watched function of the program still carrying a note (less
   Mach-O's leading `_`) — a C99 `inline` function its note stopped from being inlined —
   loses that function's note, kind "link": "the program does not link with its note: <name>
   is undefined";
   (b) a symbol that is not a function of the program (a link-time check such as
   `__bad_size()` behind `__builtin_constant_p`): the referencing function the linker names —
   ld64's indented lines after `referenced from:`, GNU ld's ``in function `fn'``, lld's
   `>>> referenced by … (fn)` — loses its note if it still carries one; otherwise step 4's
   search runs over the notes of the objects the linker names, with the relink as the test;
   (c) a link that fails with no undefined-symbol line, or that timed out, overflowed or was
   killed, refuses the map with the linker's words.
   The link also writes its map (`-Wl,-map,<file>` on ld64, `-Wl,-Map=<file> -Wl,--cref` on GNU
   ld) for §3.5.
6. **Bounds and progress.** Per top-level file: at most 8 placed rounds and 64 compiles; for the
   whole pass: at most 400 compiles beyond each file's first compile and step 7's re-compiles.
   Past a per-file bound, that file and every probed file it reads go back unprobed (kind
   "file-limit") and it is compiled once more; past the whole-pass bound, every probed file not
   yet checked does (kind "not-checked"). The CLI prints "Checking where the notes compile…
   <file> (round <r>)" as it goes. One note found by search costs about log2(notes) + 1
   compiles (sqlite3.c: 14); the worst case for one file is 64 compiles (about 9 minutes for
   sqlite3.c at `-O2`), then that file unprobed.
7. **Re-compiles.** A probed file (a header or a `.c`) whose notes were taken out makes every
   top-level file that enters it (its `-H` list) compile again — after the first pass over all files, once per file
   per pass, not counted in step 6's per-file bound. Files are handled in sorted order; the
   result is the same for the same inputs.

Order: the listings (§3.3) and every refusal by name → the plain build → this pass (its objects
and link are the probed build) → the runs. Every file the map creates after the mirror — the
listing runs' `.i` and `.d`, the runtime's object, the plain program, the probed objects and
program, the link map — goes to a freshly made, randomly named folder outside the target,
`$TMPDIR/ruharness-map-<random>`, out of reach of the copy's `-I` folders, `..` included. After
the runs, the two programs are copied (not moved: the folders can be on different file systems)
into the build folder; a guard removes the random folder on every way out — success, refusal,
error, interrupt.

Cost: a file whose notes all compile costs its own compile (the probed build's) and one
sandboxed process, plus the link; the listing runs now write their preprocessed text (≈ the size of each
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
- The runtime calls as few names as it can: no `mem*`/`str*` functions, and `TMPDIR` read by
  scanning `environ` (no `getenv`); a unit test pins the runtime object's imports (`open`,
  `fstat`, `mmap`, `close` and the like). The program must not define any of them, because the
  runtime's setup runs before `main` and would call the program's own: the probed link's map
  (§3.4 step 5) says which object defines each import, and one defined by a program object
  refuses the map by name — "the program defines open(), which the probe's runtime also uses;
  the features map cannot map it".

### 3.6 The note and the runtime

The note is `__ruharness_seen[N] = 1;` — one store, no call. `__ruharness_seen` is a pointer
that starts at a static array in the program, so a note that runs before setup still lands
somewhere. Before **every** scenario run — plain, probed, plain — the run's setup
(`run_scenario` in confine.rs, which gains this) creates `$TMPDIR/.ruharness-notes`, N + 1 zero
bytes, after the run's temp dir and its `run/` folder and before the spawn: the three runs' temp
dirs hold the same entries and the programs the same environment. In the probed program, a
constructor (linked first, §3.4 step 5) finds `TMPDIR`, opens that file (`O_RDWR | O_CLOEXEC |
O_NOFOLLOW`, never creating or truncating it), checks that its size is N + 1, maps it
`MAP_SHARED`, points `__ruharness_seen` at the mapping, then sets in it every byte set in the
static array — a merge, so a second image of the program (re-executed or spawned with its
environment) adds its notes to the first's instead of wiping them — sets the last byte (the
**attach byte**) to 1, and closes the descriptor. After the probed run the harness reads
exactly N + 1 bytes, each 0 or 1: the attach byte 0 reads "notes unavailable — the probe's setup
did not run" (the program changed `TMPDIR`, exited inside its own constructor, …), never a
record where nothing ran; otherwise byte n set means function n ran. Why:
- no call means no name the program could define at note time, and nothing its descriptors,
  `RLIMIT_NOFILE` or a closed-descriptor loop can break;
- a crash keeps every note made before it (the mapping's pages belong to the file);
- a smaller note changes inlining less: the review's builtin-flip sweep narrowed from about
  11–33 function sizes to 1–2 — not to none (§6).
Checked under the sandbox's run profile (scratchpad `spike-mmap/`, an earlier form that
created the file itself): a note from a constructor before setup, a loop closing every
descriptor, and a crash after notes were all recorded; the build re-checks this form.

### 3.7 What the person sees

- `map.json` gains `unwatched_reasons`: one `{file, id, kind, detail}` per unwatched pair, an
  optional field (older harnesses ignore it). `kind` is one of a closed list, each shown in
  plain words: `parser` "the parser could not read its definition"; `not-a-block` "its body is
  not a { } block"; `conditional-brace` "a # line between its head and its body";
  `skipped-branch` "its body's brace is inside #if"; `naked` "a naked function (gcc)";
  `stringized` "its body is inside a macro argument that becomes a string"; `data` "listed but
  never included by <file> (#embed, __has_embed or __has_include)" or "read by .incbin in
  <file>"; `compile` "a note at its start does not compile: <message>"; `elimination` "its note
  broke the build (found by building without it): <message>"; `link` "the program does not link
  with its note: <name> is undefined"; `file-limit` "the scratch copy of this file did not build
  with notes: <message>"; `not-checked` "not checked: the notes check stopped at its limit before
  this file". `detail` (the file, message or name) is at most 160 bytes, valid UTF-8, cut on a
  character boundary, Unicode control characters as `?`; the writer and the strict reader use
  that one rule, and each pair must be in the file's own `unwatched` list. Reasons are
  display-only: never in prompts, events (which carry counts per kind) or harness-mcp. SCHEMAS.md's
  map.json entry names the field and the `probe` input.
- `harness features map` prints "Checking where the notes compile…" with progress, then one
  line per reason kind with a count and one example, e.g. `features: 3 function(s) unwatched —
  a note at their start does not compile (src/fp.c, kernel: '#pragma STDC FENV_ACCESS' can only
  appear at file scope or at the start of a compound statement)`.
- The cockpit, where it now says a function is unwatched ("(the probe could not put a note in
  it)"), shows that function's reason instead.
- The map's inputs gain `probe` (this design's version, `compiler-guided-1`); a map made before
  this change reads out of date, "made by an older harness", not current.

### 3.8 gcc

The same pass on gcc (the Linux CI job): the flags of §3.4 step 1; gcc's include chain ("In
file included from <file>:<line>," then "from …:", innermost first); `#line` files to
elimination (step 3); the three linker forms (step 5). gcc accepts a statement in a naked
function on some targets and ignores some pragmas the clang judge rejects: on gcc, §2's judge
facts are re-run by the map tests of §4 on the Linux job, and rule 4 keeps naked functions
unwatched on gcc.

## 4. Tests and checks

Map-level tests, each failing without its rule. Placement fixtures use a rejection both
compilers make — `__label__` right after the note — so they also run on the Linux job;
clang-only cases (the STDC pragmas) skip on gcc with a printed note; gcc's message forms are unit
tests on recorded text. The review's repro inputs the build re-runs are copied into the repo's
test fixtures.
- §3.1 — rule 1: a head split by `#ifdef WIDE`/`#else` that the parser cannot read, with later
  functions under the same ERROR node (both configurations); rule 2; rule 3 in each spelling —
  `#ifdef`, `#ifndef`, `#if X`, `#  ifdef`, `/* c */ #ifdef`, `%:ifdef`, `#\⏎ifdef` — with and
  without the define; two heads for one body; watched: a parameter list split by `#ifdef`, a
  head that starts inside `#ifdef _WIN32 __declspec(dllexport) #endif`; unwatched: Cython's `#if X /
  static int f(…) / #else / static int f(…) / #endif / {` shape, even with the same name both
  ways (rule 3 does not compare the heads' names — counted in §6's cost); rule 4 on gcc only.
- §3.2 — a brace in a skipped branch that the text rule is made to miss (a unit test that turns
  rule 3 off): the end-token count unwatches it.
- §3.3 — `#embed`, `# embed`, `%:embed`, `#\⏎embed`, `#/*c*/embed`, `__has_embed(… clang::offset(N))`;
  `.incbin`, `.INCBIN`, an `.incbin` built by a C macro, of another top-level file's source;
  `__has_include` only (notes out with its reason, no refusal); negatives that unwatch nothing:
  a header whose comment mentions `#embed`, `#if defined(__has_embed)`, `#embed "logo.bin"`; a
  one-file program keeps its notes; a header reached twice through a file link under `#pragma
  once` keeps its notes; a stringized note; a file both included and embedded, and a file
  embedding itself (refused by step 4, by name); a target under a folder named with `é`, a
  no-break space and a backslash (maps); gcc: a file using `__TIMESTAMP__` passes.
- §3.4 — a pragma macro in each spelling (clang, one round); 200 functions whose notes are
  rejected across a header and the file at the real mirror path (well over 8 KiB of errors: at
  most 2 rounds, exactly those unwatched, no search — a mutation capping stderr at 8 KiB must
  fail it); ten functions on one line with only the last rejected, a tab and a multibyte
  character before the error's column; a lone CR in a comment above a rejected function; two
  rejected notes in one included file (one each, no search); a bison-shaped `#line N "x.y"` and
  a renumbering `#line N` (the right function; on gcc through the search); two functions under
  one `#line "x.tpl"` and one macro used in two functions giving the same error (exactly those
  two); an error through two include levels, through an unprobed `.inc`, and through a
  `..`-spelled include; an error no body holds (the search finds the one note, and the restore
  pass keeps the others); the `asm` "i" operand (`asm("" :: "i"(n))`); the C99 `inline` link
  case (the inline function unwatched, its callers watched) and a callee's note tipping it; a
  `__builtin_constant_p`-guarded call to an undefined external (the referencing function
  unwatched after one relink, no search — a mutation dropping rule (b) must fail it) and the
  same shape with `__attribute__((error))` (one placed round); a compile with no located error
  that still fails with every note out (refused after exactly one extra compile); a compile
  that times out (an `always_inline` doubling tree, `timeout_secs = 5`: refused, not read); the
  per-file round bound (`#pragma clang diagnostic fatal "-Wdeclaration-after-statement"` over 9
  functions) and the pass bound; two top-level files with a weak function of the same name and
  a constructor each, the first recompiled last (the first file's weak function noted as run);
  the progress lines.
- §3.5 — the runtime compiled without `-I`; the probe header found only by its path; a
  program defining `open` (refused by name); `__has_include("../../../plain")` and
  `__has_include("../../../1.i")` finding nothing; the runtime's imports pinned (unit test).
- §3.6 — notes kept across a closed-descriptor loop, a crash, a self re-exec and a self spawn
  with the environment (unsandboxed), a constructor that forks before setup, a constructor's
  thread running thousands of watched functions (none read "not run"; a copy-then-switch
  runtime must fail it); an earlier constructor that changes `TMPDIR` (reads "notes
  unavailable", not a complete record); a strict read of a short or bad notes file; the three
  runs' temp dirs holding the same entries.
- §3.7 — a map holding a maximal reason of each kind, written and read back strictly; a reason
  whose 160th byte falls inside a multibyte character; an older map reads out of date.
- The runner — a child that writes past the output cap then sleeps, and one ignoring SIGPIPE
  that keeps writing: both end as output overflow well before the deadline.
- The two probe-level test functions of today are rewritten: the removed rules' expectations
  become "watched"; the kept rules' fixtures stay.
- Mutation checks of each rule above. Re-runs through `harness features map` (§8): both
  corpora, the five pragma matrices, the checkers' repro suites, zopfli's map (the same
  functions per scenario, fewer unwatched); reported with the probed build's time, peak memory,
  object size and a probed run's time against plain, for sqlite3.c and zopfli.

## 5. The last check's 45 findings (scratchpad `check7/findings.md`, in its order)

| # | finding | disposition |
|---|---|---|
| 1 | `#embed` of a watched file | §3.3 steps 1 and 4 |
| 2 | the copy's build sees `plain` | §3.4 "Order": everything in a random folder outside the target |
| 3 | the probe header dropped wherever found | §3.5 |
| 4 | `.incbin` of a watched file | §3.3 step 1 |
| 5 | the runtime compiled with the target's `-I` | §3.5 |
| 6 | the note shifts `__builtin_COLUMN()` | residual §6 |
| 7 | a Unicode space in a listed path makes the map refuse | fix: the make-rule reader splits on ASCII blanks only |
| 8 | the head-error rule unwatches half of sqlite | removed (§3.1) |
| 9 | "features need a program with one main()" printed for any refusal | fix: only when the facts' main count is the refusal's cause |
| 10 | a program-side list misread worded "does not build" | fix: "the compiler's list of what <file> reads cannot be read back: …" |
| 11 | the listing re-run without `-H` after a timeout | fix: no re-run after a timeout or overflow |
| 12 | the runner's poll adds 25–50 ms to short children | fix: tool runs (the compiler) wait 1 ms, doubling to at most 8 ms |
| 13 | an unreadable file in `source_dir` refuses the map | fix: left out of the copy; if the program reads it, the listings differ and the map refuses by name |
| 14 | a header's function reached through a file link noted under the link's path only | residual §6 |
| 15 | a struct in a macro read as a function | the compiler decides (§3.4): its note does not compile |
| 16 | an absolute include of a function-less header refused | fix: refused only when the file has notes |
| 17 | `source_dir = "."` accepts no `include_dirs` | fix: both paths normalised before the prefix test |
| 18 | an unsandboxed scenario's forked child kept or killed by chance | fix: built-program and scenario runs keep the fixed 50 ms poll |
| 19 | the process-group timeout test flakes | fix: its timeout (a fresh binary's first exec under load) |
| 20 | `a_scenario_run_ends_as_data` flakes | fix: its timeout, as #19 |
| 21 | a huge `timeout_secs` overflows the deadline | fix: clamped at load, with a message |
| 22 | the poll ramp's later wake-ups for 32–100 ms children | fixed with #12 (tool runs only) |
| 23 | doc comments still describe a fixed 50 ms poll | fixed with #12 and #18 |
| 24–29 | the split guard's one line, `#include`/`#pragma` inside a split, `#elif` closing it early, keyword-led splits, the leading-edge walk, `FENV_ON;` read as a type | removed rules (§3.1); the compiler decides |
| 30 | `#pragma clang attribute push(naked)` | the compiler decides on clang; gcc: rule 4 |
| 31 | `__label__` behind a macro | the compiler decides |
| 32 | the head-error rule's cost (a third of sqlite and Cython) | removed (§3.1) |
| 33 | one ERROR node unwatches every later function | residual §6 (rule 1) |
| 34 | any `#pragma` at a body's start unwatches | removed; the compiler decides |
| 35, 36 | typedef-led and `return` splits unwatched | removed; the compiler decides |
| 37 | §5.3's costs left out the costliest rules | §6 names the costs that remain, with numbers |
| 38 | a no-break or ideographic space in the target's path | fixed with #7 |
| 39 | the copy finding the probe header or `plain` not refused | §3.5 and §3.4 "Order" |
| 40 | the lone-name rule's cost unlisted | removed |
| 41 | `ends_its_line` misreads a comment followed by code | removed |
| 42 | a loop macro opening a body unwatched | removed; the compiler decides |
| 43 | the split exception's untested parts | removed |
| 44 | "a listed path that does not resolve" has no killing test | fix: a map test |
| 45 | the `-H` order and depth comparison is no longer exercised | fix: a fixture where only the order differs (`#pragma once` through a file link) |

## 6. Residuals and costs (named, each with what would change it)

- **A definition the parser cannot read** stays unwatched (rule 1): 115 on real code — 109 of
  the extension files' (nkf.c, 109 of 140) and all 6 left in the corpus (signal-hook's
  extract.c, 3 in each of two versions) — plus the sqlite3 gem's `database.c` (34 of 37).
  Revisit if a preprocessing scan frontend (libclang) lands.
- **Rule 3's cost**: 27 in the extension files (7 `PyInit_*` and 20 heads split by `#if`); 0 on
  the corpus.
- **`__has_include` of a probed file** costs that file's notes (§3.3 step 1).
- **A file both included and embedded, or a file embedding itself**, refuses the map by name
  (§3.3 step 4) rather than being unprobed.
- **An `.incbin` built by an assembler macro** is not seen (the assembler expands it after
  the preprocessor).
- **Inlining-dependent choices**: a note makes its function a little larger; at the exact
  inlining threshold, `__builtin_constant_p` or `__builtin_object_size` inside it can fold
  differently, so the copy runs other functions with identical output. The store note narrows
  the window to 1–2 function sizes in the review's sweep; it does not close it.
- **The search's culprit** is the note whose removal made the file compile, which inlining can
  make an innocent one (the restore pass puts back what it can).
- **A function in a header reached through a file link** is noted under the link's path; under
  its real path it reads as not run.
- **`__builtin_COLUMN()`** on a body's first line reads a different column in the copy.
- **Images without the notes**: an image started without `TMPDIR`, or with another one, loses
  its own notes (other images' notes are kept); a program that exits inside its own
  constructor before the runtime's setup reads "notes unavailable"; a thread started by an
  earlier constructor at a non-default priority, or from a dynamic library's constructor, can
  lose a note made during setup's switch to the mapping.
- **gcc**: `#line` files and a byte-order mark cost search compiles; naked functions are
  unwatched (rule 4); trigraphs (`??=` for `#`) are not read as directives.

## 7. What is removed

From `harness-scan`: `blocks_note`, `leading_run_blocks`, `opens_with_error`,
`split_by_directive`, `continues_a_split`, `lone_macro`, `branch_is_empty`, `ends_its_line`,
`parameter_list`, the naked-head and head-error checks; `probe_point`'s unused `declarator`
binding; `FnDef.probe_at`'s doc comment rewritten to §3.1 (the naked check stays for gcc as
rule 4). The probe-level tests of the removed rules are rewritten (§4). From the runtime: the
append-an-id writer, the descriptor ≥ 900 logic and the reopen-once logic.

## 8. The build's own premise re-run

Before the steps are built on it, the build re-runs §2's four inputs and the review's repro
folders through the real `harness features map` (not the prototype), and replaces §2's table
with those numbers.

## 9. Review record

### 9.1 The draft (777cd27) — 37 findings, and how revision 1 was checked

| finding (plain words) | where it is answered now |
|---|---|
| a `{` inside `#if` folded into the declarator: the note sits in a skipped branch (found twice) | §3.1 rule 3 (depth form, every directive spelling) and §3.2 (the compiler's preprocessing as its net) |
| a note inside a macro argument that becomes a string | §3.3 step 2 |
| syntax-only checks miss code-generation and link errors (found three times) | §3.4 steps 1 and 5, including a callee's note that tips inlining |
| error locations wrong for nested includes, `#line` files, several functions on one line (found twice) | §3.4 steps 1, 3, 4: physical locations, byte placement with the compiler's line model, the chain printed last, gcc's `#line` files to elimination |
| inlining-dependent builtins pick other functions | §6, with the sweep's real result (narrowed, not closed) |
| the runtime calls names the program can define, and loses notes on closed descriptors | §3.5 (refused by name) and §3.6 |
| the data-read rule misses spellings and refuses harmless `__has_include` | §3.3 step 1 (listed but not entered, `.incbin` by path) and §6 |
| the runner returns only 8 KiB of errors and reads a timeout as an ordinary failure (found three times) | §3.4 step 2 and its §4 test with a mutation |
| the fallback copied back too little and blamed the original | §3.4 steps 4 and 6 |
| gcc's flags, chain order, columns and macro locations; `-ferror-limit=0` fails on gcc (found three times) | §3.4 step 1, §3.8 |
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

### 9.3 The check of revision 2 (f345a49) — 40 items answered in part, 14 new

| finding | answer in revision 3 |
|---|---|
| the probe header's leaving marker left in the copy: every program refused | §3.3 step 4: the header's whole region, both markers, then every marker dropped |
| line markers spell non-ASCII, `\` and `"` as escapes: targets under such folders refused | §3.3 step 4 compares code lines only; markers are dropped |
| "listed but never entered" flagged every top-level file | §3.3 step 1: the compile's own input and the probe header left out, files compared as files |
| a header both included and embedded | §3.3 step 4 refuses by name; §6 |
| the kept-line check misread clang's line markers | §3.2: end tokens counted instead of line markers |
| gcc's `__TIMESTAMP__` reads the mirror's new times | §3.3 step 4: mirror files keep their times |
| `.i`/`.d` files reachable from the mirror | §3.3 and §3.4 "Order": all in the random folder |
| a pre-zeroed notes file turned "never attached" into "nothing ran" (found twice) | §3.6: the attach byte |
| the link blamed the callers of a C99 `inline` function (found twice) | §3.4 step 5 (a) |
| the runtime-names refusal used facts names | §3.5: the probed link's map; imports pinned; no `getenv`, no `mem*`/`str*` |
| "taking notes out never adds an error" was false (found twice) | §3.4 step 4: the search's invariant as it is, the restore pass, §6 |
| the notes file only in the probed run's temp dir | §3.6: created before every run |
| the random folder: moving across file systems, cleanup | §3.4 "Order": copied, a guard removes it |
| reasons: missing kinds, no rule for "cascade", contradictory limits | §3.7: a closed list of kinds, one rule for writer and reader |
| step 4's order of placed and unplaced errors; the fallback's scope; its words | §3.4 step 4 |
| a compile failing with no error line, and a timeout's words | §3.4 steps 2 and 4 |
| the pass bound and its reason; the cost of the search | §3.4 step 6, §3.7 |
| gcc: detection, naked, `#line` tests, repeated errors, byte-order mark | §3.4 steps 1 and 3, §3.1 rule 4, §4 |
| rule 3's reach (from the name; two heads for one body) and its cost | §3.1 rule 3, §6 |
| the runtime's order against other constructors, threads, re-exec | §3.4 step 5 (runtime first), §3.6, §6 |
| `__has_include`-only files' reason, `.incbin` spellings | §3.3 step 1, §3.7 |
| the tests: fixtures named, the both-compilers trigger, the missing tests | §4 |
| the table's numbering | §5 |
| the runner's two polls and an overflow that sleeps | §5 #12, #18; §4 "The runner" |
| words a person cannot act on | §3.7, and refusals that say what to do or that the fault is the harness's |
