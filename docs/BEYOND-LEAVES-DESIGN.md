# Beyond the leaves: a piece that calls other pieces moves (design, 2026-10-21)

*Status: current — designed from the spike of 2026-10-21 (DECISIONS 2026-10-21, "Beyond the
leaves"); built in passes A (the judge) and B (the cockpit and the docs). SCHEMAS.md governs the
record shapes once built.*

## 0. What it is, in plain words

Today a piece of a program moves to Rust only if its C calls nothing in the program's other
pieces: the judge tests each piece alone, and every build it makes links the piece's own C or
Rust and nothing else, so a piece that calls another cannot even link its test program. On liblzg
two of four pieces "stay C"; on a large program almost every piece would. This design lets a piece
that calls other pieces move, in dependency order, with the pieces it calls linked **as the
program stands** — their Rust once accepted, else their C — so the judge still tests one piece at
a time and the program accumulates Rust piece by piece.

The spike (DECISIONS 2026-10-21) proved it on liblzg with no new mechanism: u-encode, which calls
u-checksum, went green on every check once the callee's code was linked; two Rust pieces linked
into one program on macOS in either order; a program with both ran byte-identical to the C on
twenty-four runs; one combined crate over both unit crates also linked and ran. The walls were
all one assumption, in six places, plus one real guard that must become an allow-list.

## 1. The rule

A piece **P** may move when the plan knows every piece it calls (its `depends_on`, transitive;
the plan already refuses unknown ids and cycles, and the planner merges mutual callers into one
piece). Nothing else gates it. The order the cockpit recommends is the plan's execution order
(leaves first, `Plan::execution_order`), so the common path accumulates Rust bottom-up; a person
may move a caller first, and its verdict simply goes out of date when a callee's Rust is later
accepted (§4).

The piece holding `main` does not move in this version: its test program would have to replace
`main`, and a program whose `main` is Rust is the end state (PROJECT-MAP-DESIGN §8, problem 5),
designed separately. Its state word is `holds main — stays C in this version`.

**The dependency inputs** of P, computed in one place (`harness_oracle::deps::inputs(prep, unit)`):
for each piece D that P calls, transitively, in execution order, **as the program stands** — D's
promoted Rust staticlib when D's status is verified or merged and its crate builds, else D's C
files (D's `replaces`). Each input carries its digest (the staticlib's hash after its build; the
C files' content digests). The same function serves every build below, so no build can forget a
callee again.

## 2. Every build the judge makes links the dependency inputs

| build | today | now |
|---|---|---|
| driver validation: `-O2`, `-O0`, sanitizer, each mutant (`validate.rs:282-296`, `:525-546`) | driver + P's C | driver + P's C + dependency inputs; **only P's own C is mutated** |
| differential C side (`lib.rs:1083`) | driver + P's `replaces` | + dependency inputs |
| differential Rust side (`lib.rs:1085`) | driver + P's staticlib | + dependency inputs |
| sanitizer build (`lib.rs:1140`) | as the C side | as the C side |
| boundary plain driver (`boundary_run.rs:411`) | driver + P's C | + dependency inputs |
| whole program, mixed (`lib.rs:1460`, `features.rs build_whole_mixed`) | all C but P, + P's staticlib | all C but every **accepted** piece and P, + each one's staticlib — the program as it stands (what `perf` already builds) |

Two staticlibs from the harness's template link on macOS in either order (`panic = "abort"`
in both, so no runtime clash; the perf design's rules for no-std and LTO stand and are checked
there). On Linux the link wraps the staticlibs in `--start-group … --end-group` (unchecked here:
no Linux machine; said in SCHEMAS).

**A failed link is a red check, never a harness error.** Today an undefined symbol in the
differential link raises an error and leaves the attempt "in-progress" with no verdict. Now the
C side failing is `differential-driver` red with "the C side does not link: <first error line>"
(a harness-side cause, so the status line says "a build problem, not the Rust's"), and the Rust
side failing is `rust-build` red with the linker's first line — a model that forgot a callee sees
a red it can repair.

## 3. What the Rust may call, and how it is told

- **The prompt** (`harness-llm`, the migrate task) gains a section **"Functions of other pieces
  you may call"** when P has dependencies: each callee function's C prototype (from the facts'
  symbols and the headers P includes), the rule that they are declared in `ffi.rs` inside one
  `extern "C" { … }` block exactly as prototyped, called only from `ffi.rs` or through a safe
  wrapper handed into `logic.rs` as a function value (the spike's shape), and never redefined.
  No other `extern` block is allowed.
- **The source rule check** (`emission.rs:514`, the deny scan): an `extern` block is refused
  unless every declaration in it names one of P's dependency symbols (the existing allow-list
  path, `deny_scan_with`, with the dependencies' exported symbols); any other foreign
  declaration stays refused with today's words. This is the one real guard the spike met, and
  it stays a guard.
- **The capabilities check** gains an import rule: an undefined symbol of the candidate's own
  members that is a function **defined in the program's C** (the facts know every one) must be
  in P's dependency symbols; otherwise the check fails with "calls <name>, which belongs to a
  piece it does not depend on". Imports of libc and std stay as today.
- **The symbol-set check** keeps ignoring imports (a candidate may not *define* a callee: that
  stays refused as an unexpected export). It also learns LLVM's promoted constants: a defined
  symbol matching `anon.<hex>.<n>.llvm.<n>` with private-external (hidden) visibility is
  compiler-made, not an export, and is not reported (the spike's side finding; any large piece
  can produce one).

## 4. Freshness: a changed callee makes the caller out of date

A verdict's inputs (`oracle-latest.json`, SCHEMAS "verdict inputs") gain `dependencies`: the
list of (piece id, kind `c` | `rust`, digest) the build linked. The driver-validation record gains
the same. `state status` and the cockpit compare them as they compare `unit_source`: a
mismatch is **`out of date (a piece it calls changed)`**, in the attention colour, with Re-check
as the Next step. So accepting u-checksum's Rust makes u-encode's verdict out of date: the next
Re-check tests u-encode against u-checksum's Rust, which is what the program now runs.

## 5. The cockpit

- `stays C` is only for the piece holding `main` (`holds main — stays C in this version`) and
  for a piece whose callee is unknown to the plan (never, after `plan`); every other piece has the
  journey's stage words. The tool's counts and the journey line count every movable piece.
- The Next step on the tool's row recommends the first piece in execution order that can move;
  a piece's card says, when it calls others: "It calls u-checksum: its test program and its Rust
  are checked with u-checksum as the program stands — its Rust once accepted, else its C."
- The hint line and the `i` card of a piece whose verdict is out of date say which callee
  changed. The migrate refusal "calls …, so in this version it stays C" goes; the greyed reasons
  with it.
- The whole-program check and the features run the program **as it stands** (§2): the tool's
  card and the Project view's tool row say "the program runs with 2 of 4 pieces in Rust" from the
  same facts (the first half of the "program works" stage; the stage itself is its own design).

## 6. Docs

TUTORIAL "Why some units wait" becomes "The order pieces move in"; the judge topic says a
piece is tested with its callees as the program stands; the guide's Part 7 (today the u-encode
refusal) walks u-encode moving after u-checksum, its screens re-verified with cockpit.py and the
CLI; SCHEMAS: the verdict's and the driver record's `dependencies`, the import rule, the two red
details, the Linux link note; PROJECT-MAP-DESIGN §8 problems 2 and 3 marked "built: BEYOND-
LEAVES-DESIGN"; FIRST-HOUR unchanged (u-version is a leaf). docs/README.md gains this document.

## 7. Tests

A two-piece fixture (callee and caller, the caller's C calling the callee's function) through
the whole judge: the caller's test program validates with the callee's C linked; the caller's
Rust with an `extern` block for the callee goes green; the callee promoted, the caller's verdict
is out of date and goes green again against the callee's Rust; a mutant touches only the caller's
C; an `extern` block naming a non-dependency function is refused by the source rule; an import of
a program function outside the dependencies fails capabilities; a Rust side that forgets the
callee is `rust-build` red, not an error; the whole-program mixed build links every accepted
piece's staticlib; the `anon.…llvm.…` symbol is ignored; the cockpit's words, order and card; the
guide's Part 7 gate green. Then a live walk on liblzg, in the cockpit, moving u-checksum, u-encode
and u-version in order with hand answers, and `perf run` on the result.

## 8. Order of work

Pass A — the judge (harness-core, harness-oracle, harness-llm): §1's inputs function, §2, §3, §4,
the fixture and its tests, SCHEMAS. Pass B — the cockpit and the docs (§5, §6), after A. Then the
live walk. The combined crate (one staticlib per program, LTO: 381 KB against 1.42 MB in the
spike) and the piece holding `main` are the end-state design, later.
