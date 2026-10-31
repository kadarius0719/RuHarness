# Functions written in headers

The design behind the lz4 run's fourth wall (2026-10-30): `programs/util.h` writes fourteen
helpers in full, marked `UTIL_STATIC`; the plan cut a piece from the header and the judge
refused its test program ("no .c file to validate a driver against"). zopfli's `symbols.h` is
the same shape, and had passed unnoticed: its helper calls were recorded as unresolved names.

## 1. What a header helper is

A function defined in a header with `static` (directly, or through a macro the project defines
as `static …`) is not one function. Every `.c` file that includes the header compiles its own
private copy; no object exports it; nothing can link to it. It is part of every file that
includes the header, the way a macro is. Three consequences, each a part of the design:

- the scanner must see the `static`, however the project spells it, and must know which copy a
  call reaches;
- the plan must never cut a piece from a header, which no compile builds on its own;
- a piece whose C uses such helpers carries its own translation of them: its Rust cannot call
  the C copies, and its C object never exported them.

## 2. The scanner (harness-scan)

**A specifier macro defined as `static`.** Before the functions are collected, every scanned
file's lines are read for `#define NAME static…` (object-like only: `NAME(` is a function-like
macro and skipped; `static` must end the token: `staticky` and `static_assert` do not match).
A function whose signature's first token is such a name is static. The reader does not
preprocess: a macro defined under one `#if` branch counts for every branch, and a `#define` in
a comment is a false hit that costs nothing unless a function head starts with that word.

**Where a call lands.** A call to `N` from file `F` resolves, in this order: `F`'s own static
`N`; the static `N` of the nearest file in `F`'s include closure (breadth first, includes in
sorted order, the file itself excluded); the public `N`; else unresolved. A helper calling a
helper of its own header resolves within the header (its own static). The facts keep their
shape: the helper's symbol is `<header>::<name>`, visibility `internal`, and the ref's `to`
names it, `resolved: true`.

The committed facts of zopfli changed with this (refs only): `deflate.c`'s calls to
`symbols.h`'s helpers resolve now. Its plan did not change; its features map was rebuilt, since
a map is bound to the facts' digest (FEATURES-DESIGN §8.5) and reads "out of date" otherwise.

## 3. The plan (harness-core planner)

A unit-seed file defines at least one public symbol and is not a header (`.h`, `.hh`, `.hpp`,
`.hxx`, by `planner::is_header`). With the scanner's rule a header of helpers has no public
symbol anyway; the header rule is for the odd header with a non-static function written in it,
which no compile builds on its own either (included once it is part of that includer; included
twice the program would not link). Such a function is not listed in any piece's `symbols`; the
judge's export check reads the compiled C object, so an includer's Rust is held to what its
object exports, header function included — an edge left as it is, noted in §7.

A piece the planner no longer computes is set aside (`status = "blocked"`), as before; the
cockpit's words for it now say why it can happen ("a header whose helpers move with the files
that include them").

## 4. The prompt (harness-llm migrate)

`Facts::header_helpers(files)` lists, per header, every internal symbol outside `files` that a
resolved call from `files` reaches, and every one those reach in turn. For a unit with any,
the migrate prompt carries a `[HELPERS WRITTEN IN HEADERS]` section between the callees
section and `[C SOURCE]`: a fixed paragraph (the rule: translate each helper the unit uses as a
private `fn` in `src/logic.rs`, never export its name, never declare it `extern "C"`), then one
fenced, JSON-encoded line per helper, `<header>: <C prototype>`. The header itself is already in
`[C SOURCE]` (the prompt sends the unit's include closure), so the model has the code. A unit
with no helpers renders byte for byte as before (fixture `migrate-translate-helpers.txt` holds
the new branch; every other fixture is unchanged).

The judge needs no change. The export check (`symbol-set`) compares the Rust's exports to the
C object's, which never held the helpers; a helper exported by mistake is "an export the C does
not have". The source rule check refuses an `extern "C"` declaration that is not a callee's. A
helper declared extern and not defined would fail the link in any case.

The test program (driver) is C and includes the unit's headers: it may use the helpers as the
C does; its object holds private copies, which `driver-shape` ignores (it looks at exports and
undefined references).

## 5. The cockpit

The piece card says `Uses N helper functions written in <header>, translated as part of this
piece: …` after its `Its C:` line, from the facts, when there are any. The header's row in the
tree reads `header`, as every header not owned by a piece does.

## 6. What this does on lz4

`u-util` disappears from the plan on the next Refresh (set aside); `util.h` is a header.
`u-lz4io`, `u-bench` and `u-lz4cli` get the helpers section: nine, six and one helpers
respectively. `UTIL_countCores` in `util.c` is a real function, and `util.c` is "not part of
this tool" because the map did not see lz4cli link it; that row is still on the walls list.

## 7. Open

- A header with a non-static function written in it (§3): its includer's object exports it,
  so that includer's Rust must too, yet the piece's `symbols` do not list it. Rare; refuse the
  map with a sentence when it happens rather than guess.
- The driver prompt does not name the helpers; a test program that needs one has the header.
- Platform `#if` branches in helpers (timefn's shape) are the per-platform design, not this one.
