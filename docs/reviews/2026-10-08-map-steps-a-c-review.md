# Review of the map's steps (a) and (c) — 2026-10-08

Four Opus checkers at high effort read `git diff 3f54389..610fd50` from four lenses (the oracle, the readers, security, usability and the tests), each in its own worktree, with experiments. Their reports follow verbatim; the triage is at the end.

---

# harness-oracle after the map's steps (a) and (c): checker's report

Worktree at 610fd50 (fast-forward was already up to date). Read: the briefing §10–§12, the design
§3.1 step 5, §3.7, §3.9, §4 "The rest of the harness", §5 (a) and (c), and
`git diff 3f54389..HEAD -- crates/harness-oracle`. Experiments ran in the scratchpad with
`RUHARNESS_ADOPTED` pointed at a scratch file (the person's adoption file was never touched). The
temporary test file used for two experiments was deleted; the worktree is clean.

## High

**1. The configuration's flags reach the boundary check's own runtime, so `-std=c89` makes every
boundary check red.**
Evidence: `boundary_run.rs:177` builds `harness_args` from `unit_args` (the configuration's flags
plus the unit's folders), and `boundary_run.rs:209` compiles every harness-owned file with it,
including `ruharness_guard.c` and `ruharness_probe.c` (`:327`, `:333`, `:339`). The grammar allows
`-std=c89`, `c90`, `gnu89`, `gnu90`. `ruharness_guard.c` is not C89: it declares `int j` in two
`for` loops at lines 616 and 626. Experiment: the hand-written tool in `tests/file_list.rs` with
flags `-DPAIR_WIDE=4 -std=c89` passes every check except
`[false] boundary — boundary driver invalid (C side): the guard runtime does not compile …
redefinition of 'j'`. `gnu89` fails the same way.
Failure: any file-list tool whose build says `-std=c89` (common in older C) can never get a green
boundary check. The same path also hands a project's `-include config.h` or a `-D` that renames a
libc function to the guard's own C.
Fix: compile the guard and probe runtimes with no configuration (only `bd/`, as the features map
already does for its probe runtime with `FileArgs::default()`). Keep the configuration for the
wrapper, which includes the unit's headers, and make the wrapper C89-clean. Add `-std=c89` to
`tests/file_list.rs`.

**2. Headers reached only through the configuration's `-I`, `-iquote`, `-isystem` or `-include`
are passed to every compile but are missing from the facts.**
Evidence: `Base::resolve` (`lib.rs:364-379`) turns the configuration's path flags into arguments
for every compile. The scanner's file-list read (`harness-scan/src/lib.rs:300-313`) follows only
each file's `include_dirs` and never these flags. End to end: a project with
`flags = ["-Iinc"]` and no per-file folders gets `"includes":[]` for both files from a real
`harness scan --tool`. On the same facts:
(a) verify's driver-shape lint refuses the driver's `#include "pair.h"` ("is not one of the unit's
own headers");
(b) `compute_inputs`'s `unit_source` does not change when `inc/pair.h` changes, so a verified unit
stays "green (fresh)" after its header is edited;
(c) `map_features` refuses, because the scratch copy holds only the facts' closure.
`forced_includes` copies the `-include` file into the mirror, but not anything that file
includes. All three were run.
Failure: a configuration taken from a Makefile, which is where `-Iinclude` usually sits, gives a
tool whose staleness misses its headers, whose drivers fail the lint, and whose features cannot be
mapped.
Fix: pick one rule in one place. Either refuse path flags in the configuration and fold them into
each file's `include_dirs` when the tool is written, or have the scanner (and the closure readers
`unit_headers`, `driver_folders` and `unit_header_names`) treat the configuration's folders as
every file's folders and its `-include` files as reached headers.

## Medium

**3. A verdict does not record the configuration, so changing the flags never makes it stale.**
Evidence: `lib.rs:890-905`. The toolchain entries are rustc, cc, sandbox, the constant
`cflags: -ffp-contract=off`, the boundary runtime and `observable`. `VerdictInputs` carries no
configuration name, flags or per-file folders. `harness-core/src/status.rs:290-310` compares only
`unit_source`, `rust_crate` and `driver`. The v2 program digest covers the configuration, but only
in the features coverage marker, and only when a features file exists.
Failure: editing `-DPAIR_WIDE=4` to `=8` changes a struct's layout under the verified Rust, and
`state status` still says "green (fresh)".
Fix: add a toolchain entry for file-list targets only, such as
`configuration: <name> <blake3 of flags and per-file folders>`, so folder-form verdicts stay byte
for byte. Have status mark the verdict stale when the entry differs.

**4. A mutant's quoted includes do not search the original file's folder first when the
configuration has `-I` or `-iquote`.**
Evidence: `validate.rs:562` returns `file_args(original).with_first(orig_dir)`. `cc_argv`
(`lib.rs:1768-1790`) puts every configuration flag before the `-I` list. In the mutant's
`dv/mut-N/` copy, a quoted `"x.h"` therefore searches the configuration's `-iquote`/`-I` folders
before the original's folder. The original object, compiled in place for the same-object
(equivalence) comparison, finds its own folder's `x.h` first.
Failure: when a configuration folder and the file's own folder both hold a header of the same
name, mutants are built against another header. They fail to compile, or are wrongly counted as
not equivalent, and the mutation score lies. Folder-form targets are unaffected (no flags).
Fix: pass the original's folder as `-iquote<dir>` placed before the configuration's flags. A
mutant-only option in `CcInvocation` would do it.

## Low

**5. The driver-shape gate no longer sees absolute or indirect definitions that `nm -gU` showed.**
Evidence: `objsyms.rs:307` skips every Mach-O type other than undefined and in-a-section, which
includes N_ABS and N_INDR. `objsyms.rs:457` skips ELF `SHN_ABS`. Experiment: a driver with
`.globl _absval; _absval = 42` gives `A _absval` under `nm -gU`, which the old check refused as a
second definition. `objsyms::external` does not report it, so `defines only main` passes.
Indirect aliases still leave their target undefined, so they are caught. An absolute symbol is not.
Fix: report N_ABS, N_INDR and `SHN_ABS` external symbols in `external` (the map may ignore them),
and add the case to `the_object_is_read_without_nm_and_odd_names_are_refused`.

**6. The driver object is read whole with no size cap.**
Evidence: `shape.rs:293` uses `std::fs::read(obj)`, while the map caps objects at 64 MiB
(`projectmap.rs:836`). `nm` used to stream the object. A model-written driver with a large
initialized static array makes the harness load the whole object into memory.
Fix: check the size against the map's `MAX_OBJECT_BYTES` and fail driver-shape above it.

**7. `objsyms` puts no budget on name bytes.**
Evidence: `name_at` (`objsyms.rs:164`) copies each name in full, twice per need. A string table
whose names share tails, with many symbols pointing into one long run, costs about the square of
the object's size in memory. A compiler is unlikely to produce such an object, which is why this is
Low.
Fix: count a name over a few KiB as odd without copying it, or cap the total bytes of names read.

**8. The unit-crate check leaves gaps the design's intent covers.**
Evidence: `unit_crate.rs:116-140`.
- A manifest with no `[workspace]` passes. cargo then searches the project's folders above for a
  workspace root, and that root's `[profile.release]` (overflow checks, debug assertions) applies
  to the unit.
- `[lib] path = "../../x.rs"` passes. Its source lies outside the crate digest
  (`unit_crate_file_set_hash`), so editing it never makes the verdict stale.
- The underscore spellings `dev_dependencies` and `build_dependencies` are not refused. They are
  harmless today, since only `cargo build` runs.
- The symbol-baseline crate (`symbols.rs:576-610`) goes to cargo with no check at all.
Fix: require an empty `[workspace]`, refuse `path` keys in `[lib]` and `[[bin]]`, and refuse the
underscore spellings. Run `check_unit_crate` on the baseline folder too, or empty that folder
before writing it.

**9. The map stores a missing header's name raw, which can be an absolute machine path.**
Evidence: `projectmap.rs:1044` sets `header: Some(name)` from the compiler's message.
`#include "/Users/me/x.h"` gives `/Users/me/x.h`. Design §3.1 step 5 says the record holds "the
header's relative name". This matters once step (b) writes the map file.
Fix: keep the name only when it is a clean relative path; otherwise record `missing-header`
without a name.

**10. The new object folders stay in each unit's build folder.**
Evidence: `perf/build.rs:239` creates `<out>.obj/`. After a zopfli verify, `drv_c.obj`,
`drv_rs.obj`, `drv_c_san.obj`, `whole_c.obj` and `whole_mixed.obj` remain, a full copy of the
program's objects twice per unit. No stale object is ever linked: every object is rebuilt and
hashed before its link, and `dv/` is recreated.
Fix (hygiene only): remove `<out>.obj/` after a successful link.

## Judging the builders' tests

- `cargo_never_reads_a_projects_config_or_toolchain_file`: the `target-dir` half and the
  `path`-toolchain half prove the claim. The `rustc-wrapper` marker proves nothing, because
  `cargo metadata --no-deps` never runs rustc. A `cargo build` would.
- `tests/file_list.rs`: it does prove the configuration's `-D` reaches the driver builds, the
  shape compile, the boundary wrapper and the features map. Its comment "each listed file finds
  pair.h only through its own folder" is not exercised: both files share `inc`. The lib test
  `a_units_files_compile_each_with_its_own_folders` does prove per-file folders. Nothing tests
  `-std`, harness-owned C under the configuration, configuration `-I` against the closure, or a
  forced `-include` in the mirror. Findings 1 and 2 slipped through those gaps.
- The `unit_crate` tests prove what they name. A symlinked `src/` is refused by the code but not
  tested; only a symlinked `target/` is.
- `the_map_profile` and `cc_compiles_and_links_under_the_map_profile` prove the profile byte for
  byte, the compile and link, the working folder, `TMPDIR`, and that a header under home outside
  the project is unreadable. They are good tests.

## Holds

- **Folder form unchanged.** A real `harness verify --adopt` of u001-katajainen on a copy of
  targets/zopfli gives an `oracle-latest.json` byte-identical to the committed one (all 17 checks
  green). A bench case with `include_dirs` (`B01_organic/collided_lib`) gives identical digests;
  the only difference is the `observable` entry the committed record predates. harness-scan's
  committed-facts test passes over every committed `facts.jsonl`.
- **Argument order.** `cc_argv` gives the judge's flags, then the configuration, then `-I` in
  order. `-O` is recorded and never applied. Only `-pthread` reaches a link (`link_file_args`).
  Sanitizer flags reach both the object compiles and the link.
- **Confinement.** `project_path` canonicalizes and refuses anything under `migration/` or the
  ledger, and anything reached through a link out of the root. A file that is not listed has no
  arguments. Configuration paths, including `-include`, go through the same check.
- **Children.** Every tool child starts in the work folder (made each run, refused when it is a
  link or in a temporary folder). `RUSTUP_TOOLCHAIN` is pinned with `RUSTUP_AUTO_INSTALL=0`. Tool
  and built children both get the `PATH` filter. The only spawns outside `Runner` are perf's
  launcher (absolute clang, cleared environment) and tests.
- **The map profile** denies `/Users`, `/Volumes`, `/private/tmp` and the home folder except the
  root, the fresh folder and the work folder. It denies the cargo and rustup homes and writes only
  to the fresh folder. Objects and `.d` files are deleted as read.
- **`objsyms` bounds.** Every offset is checked. A string table with no terminator is an error.
  Huge counts end at the buffer's end.
- **Driver-shape names.** An `asm` label that is not identifier-shaped is refused. A libc name
  with one `$` suffix is judged by its base name.
- **`unit_header_names`** for a header reached through `../include` gives `include/x.h`, the
  basename, and the name relative to each driver folder. It never gives a `..` form.
- **Mutants** link originals compiled this run in a `dv/` that is recreated every call.


---

# Readers after the map's steps (a) and (c): review

Scope: `git diff 3f54389..HEAD` (head 610fd50) over harness-scan, harness-detect, harness-core's
`walk.rs`, `sources.rs`, `features.rs`, `planner.rs`, harness-llm's `trajectory.rs`, `migrate.rs`,
`driver_gen.rs`, and the CLI's `stale_fact_files`. Experiments ran on fixtures under the scratchpad
(`exp/e1`…`e7`) with the built `harness` binary and a temporary test file (deleted; the worktree is
clean). Paths below are relative to the worktree.

## High

**1. The configuration's path flags reach the compile but no reader: the scan records one header, the compile reads another, and the digest never moves.**
Evidence: the compile puts the configuration's flags (`-I`, `-iquote`, `-isystem`, `-include`, all
allowed by `crates/harness-core/src/config/flags.rs:33`) before the file's own folders
(`crates/harness-oracle/src/lib.rs:480-492`, `cc_argv` at 1767-1790). Every reader searches only the
file's `include_dirs`: the scan (`crates/harness-scan/src/lib.rs:332`), staleness
(`crates/harness-core/src/features.rs:1559-1563`), the file-list program
(`features.rs:1349-1360`), hence the planner's `source_hash` (`planner.rs:111`) and the prompt scope
(`crates/harness-llm/src/trajectory.rs:1931-1937`). Experiment `e1`: `src/main.c` includes `"x.h"`
and `<y.h>`, listed with `include_dirs = ["b"]`, configuration `["-Ia", "-includea/forced.h"]`.
`harness scan` records `src/main.c -> b/x.h` only; `cc -M` with the oracle's order reads
`a/forced.h`, `a/x.h`, `a/y.h` and never `b/x.h`. Editing all three files the compile reads left
`program_digest_now` identical (`blake3:aadb04f1…` before and after).
Failure: the model is shown `b/x.h` while the build uses `a/x.h`; a change to the forced include
(say `#define LZG_FAST 0`) keeps every green verdict and the features coverage "current"; the
features mirror copies the closure and the forced file but not `a/y.h`, so its compile fails.
Fix: give `Confine::resolve_include` the configuration's folders in the compiler's order (quoted:
own folder, `-iquote`, then `-I` in argument order; angle: `-I` in order, then `-isystem`) and treat
each `-include` file as the first include of every listed file, so the scan, staleness,
`program_paths` and prompts all see it; add the lzg fixture with a non-empty configuration.

**2. An include inside a struct, union, enum or initializer (the X-macro pattern) makes the facts stale forever: `plan` and `detect` refuse right after a fresh scan.**
Evidence: the scanner takes include names from tree-sitter (`harness-scan/src/lib.rs:537-564`),
which does not produce a `preproc_include` inside a field list, an enumerator list or an
initializer; the staleness reader is the lexical `include_names` (`sources.rs:221`), which does.
Measured on crafted inputs: `struct s {\n#include "fields.h"\n};`, the same in a `union`, `enum op
{\n#include "ops.def"\n};` and `static const char *names[] = {\n#include "names.h"\n};` are all
read by the lexical reader and missed by tree-sitter. Also different: `#inc\<newline>lude "e.h"`,
`#/**/include "c.h"`, `#include "a.h" junk` (lexical only) and `#include <sys//types.h>`
(tree-sitter only: `strip_comments` eats `//b.h>`). Experiment `e3` (an X-macro array):
`harness scan` then `harness plan` gives "facts.jsonl is stale: 1 file(s) changed on disk; run
`harness scan` first"; scan again, same refusal. `detect` uses the same gate
(`harness-cli/src/main.rs:1144`).
Failure: any file-list tool over a project using X-macros (Lua's opcode names, CPython's opcode
targets, many interpreters) can never be planned or detected; the X-macro header is also never
scanned, hashed or shown to the model. The builders' own test
`include_names_reads_both_forms_and_skips_comments` (`sources.rs:339-352`) asserts the
`#inc\<newline>lude` reading, enshrining a divergence while the module doc says "as the scanner's
parser reads them".
Fix: one reader for both. The scanner can take its include names from `sources::include_names`:
over all 496 `.c`/`.h` files of `targets/` (zopfli and the 100 cases), lz4 and liblzg the two
readers agree exactly, so the 101-root identity holds; then fix `<a//b.h>` in the lexical reader
and replace the enshrining test with one that checks both readers agree on each crafted case.

## Medium

**3. Staleness asks only "is the header found now already recorded", never "is it the one this file's record names".**
Evidence: `unrecorded_program_files` (`features.rs:1519-1567`) pushes the resolved file and flags it
only when no record of that path exists; it never compares with the including file's recorded
`includes`. Experiment `e6`: `src/main.c` (folders `["b"]`) and `src/other.c` (folders `["a"]`)
both include `"x.h"`; scan; edit `harness.toml` so `main.c`'s folders are `["a"]`. `harness plan`
succeeds and `facts.jsonl` still says `src/main.c -> b/x.h`; the compile now reads `a/x.h`.
Fix: in `unrecorded_program_files`, also report stale when a resolved include is not among the
including file's recorded `includes` (and when a recorded include no longer resolves).

**4. A file the scan cannot read is a note in the scan, a permanent staleness in the digest, and a hard stop in detect.**
Evidence: the scanner notes and drops it (`harness-scan/src/lib.rs:192-198`). Folder form:
`program_digest_now` sees an unrecorded top-level `.c` that `is_file()` and returns `facts-stale`
(`features.rs:1488-1500`); experiment `e5` (`src/b.c` mode 000) gives `facts-stale` immediately
after a fresh scan. File-list form: `unrecorded_program_files` marks an unreadable listed file
unrecorded (`features.rs:1547-1552`), so `plan` refuses forever. Detect reads with `?`
(`harness-detect/src/lib.rs:1308-1313`): on `e5`, `harness detect` exits 1 with "Permission denied"
while `scan` exited 0. Before this diff the scan stopped on such a file, so the person saw why.
Fix: let the scan record an unreadable file as a fact (path, "unreadable") so staleness counts it
as recorded, and have detect skip it with the same note.

**5. A file-list target's prompt scope is computed from `facts.jsonl`, which the target owns.**
Evidence: `read_sources` allows exactly `facts.include_closure(listed)` (`trajectory.rs:1931-1937`);
detect does the same (`harness-detect/src/lib.rs:1293-1301`). A forged edge `src/a.c -> notes/secret.h`
(or `.env`; `Confine::allows` does not prune dot-folders) puts that file in scope, and no staleness
rule catches an extra edge whose target's hash is right (finding 3). The folder form is bounded by
`source_dir` whatever the facts say; this form's boundary is the hostile file itself. The test
`a_file_list_target_reads_only_its_files_and_their_headers` (`trajectory.rs:2400-2446`) says
"another project file, even one the facts claim, is refused" but only tries an unlisted unit file,
never a forged include edge.
Fix: compute the allowed set by resolving includes afresh with `Confine` (the walk
`unrecorded_program_files` already does), refuse dot-folder paths, and add the forged-edge case.

**6. A header's includes are the union over every listed file that reached it.**
Evidence: `read_file_list` keys `includes` by path alone (`harness-scan/src/lib.rs:330-335`), while
resolution depends on the listed file's folders. With `inc/common.h` including `"cfg.h"`, reached
from `a.c` (folders `inc, d1`) and `b.c` (folders `inc, d2`), the facts say `common.h` includes
both `d1/cfg.h` and `d2/cfg.h`; `a.c`'s closure, prompt and driver folders then hold `d2/cfg.h`,
which its compile never reads (by reading the code; not run).
Fix: when one header's includes resolve differently under two listed files, record it as an
ambiguous-include fact (as §3.1 step 3 does for the map) instead of merging silently.

## Low

**7. An include that lands outside the root or under `migration/` vanishes without a note.**
`resolve_include` returns `None` (`sources.rs:210-211`), correctly not recording it, but nothing
says so, while the compile still reads that file (`#include "../../migration/tools/t-x/x.c"`).
Fix: add it to `ScanNotes` ("an include of X reaches outside the project; not read").

**8. A dot-named link to a walked folder is counted as skipped through the link.**
`walk.rs:390-393` runs the count-only pass on the link (`read_dir` follows it), so `.alias -> src`
lists `src`'s files as skipped though they are walked; `.alias -> .` counts the whole root
(up to 200 000 entries). Fix: for a link, check the target first and record `IntoPruned` or an
alias; count only real dot-folders.

**9. Detect reads a whole file after a size check, without a bound.**
`harness-detect/src/lib.rs:1309-1313`: a file growing past 8 MiB between the check and the read is
read whole; a forged facts path to a FIFO blocks. Fix: use `ledger::read_regular(abs, MAX_SOURCE_BYTES)`.

**10. The v2 digest hashes folders and flags as written.**
`features.rs:636-648`: `include_dirs = ["src/include/"]` versus `["src/include"]`, or `-I./x`
versus `-Ix`, is a new program, so a cosmetic edit drops every verdict. Fix: hash the resolved
forms `Confine::listed_files` returns.

**11. The folder form's inputs moved for dot-folders and links.**
`program_paths` no longer hashes headers under dot-folders (`features.rs:1394-1396`) and the scanner
no longer scans them, so a v1 target with such folders gets a new digest and new facts (none of the
101 roots has any). By reading: with `source_dir` itself a link, `real()` (`features.rs:1365-1371`)
names a linked top-level `.c` by its canonical path while the scan records it under `source_dir`
as given, so it looks unrecorded forever. Fix: write the first into DECISIONS; for the second,
compare both paths relative to the canonical `source_dir`.

## The builders' tests, judged

- `the_shared_walk_keeps_the_committed_facts_byte_identical`: proves its name. I ran it (passes);
  no committed `facts.jsonl` changed since 3f54389, so it compares against the old scanner's output.
- `zopfli_keeps_its_committed_v1_program_digest`: proves it (passes; `d191be5c…`).
- `a_file_list_scans_like_the_folder_form_over_a_flat_copy`: proves it for an empty configuration
  only; no test anywhere has a configuration `-I` or `-include` (finding 1).
- `a_file_list_program_is_stale_where_a_scan_would_record_otherwise`: covers edit, new header and
  vanished file; it never checks "a scan clears it" against an include the readers see differently
  (finding 2) or a changed folder list (finding 3).
- `include_names_reads_both_forms_and_skips_comments`: asserts a divergence from the scanner (finding 2).
- `a_file_list_target_reads_only_its_files_and_their_headers`: claims more than it tests (finding 5).
- `a_non_utf8_file_and_an_unreadable_folder_are_noted_and_the_scan_goes_on`: proves it for the scan;
  an unreadable file, and detect on either, are untested (finding 4).
- The walk tests (`a_link_into_a_pruned_folder_is_an_issue`, `a_folder_link_is_an_alias…`,
  `a_file_link_is_an_alias_of_the_file`) prove their names; a dot-named link is untested (finding 8).
- `the_v2_digest_covers_the_configuration_each_files_folders_and_the_run_name`: proves the record's
  fields; it cannot see that file contents behind the flags are missing (finding 1).

## Holds

- harness-core, harness-scan and harness-detect test suites all pass at 610fd50.
- The v1 record: `source_dir()`/`include_dirs()` serialize as the old fields did; zopfli's digest holds.
- Over 496 real files (targets, lz4, liblzg) tree-sitter and the lexical reader agree exactly.
- macOS case-insensitive disks: `canonicalize` returns the on-disk case (tested), so a `Migration/`
  folder is pruned and `#include "LZG.h"` is recorded under one path.
- `../` through a folder link: clang read `real/y.h` for `inc/../y.h` (physical), as
  `canonicalize` does, so scan and compile agree.
- A header reached through `../` out of the root, or into the ledger, is never recorded or read;
  a listed file or folder through a link into `migration/` or out of the root is refused by name.
- Non-UTF-8: functions, calls, includes and spans equal a UTF-8 copy's; only signature text shows U+FFFD.
- The scanner's 8 MiB check: bounded read (`take`), hash from the same bytes it parsed; the
  staleness reader's cap (`read_regular`) agrees with it.
- The walk: a link to the root itself and two links to one file give aliases, no loop, one record.
- Scan notes reach `--json` consumers as message events; only the CLI scans.
- harness-detect's folder walk is now the confined walk and prunes the ledger at `source_dir = "."`.


---

# Security check of steps (a) and (c) of the project map

Worktree at 610fd50. Scope: what steps (a) and (c) added that an untrusted project could abuse.
Experiments ran against crafted folders under the scratchpad, with `RUHARNESS_ADOPTED` pointed at a
scratch file, so the person's own adoption file was never touched. A temporary probe test was
added to harness-oracle and then deleted. The worktree is clean. Nothing was committed or downloaded.

## High

None found.

## Medium

1. **A root `harness.toml` that is a link, or a FIFO, is still followed: one line of any file the
   person can read gets printed, or the command hangs.**
   Evidence: `crates/harness-core/src/config.rs:503-508` (`load_file` calls `read_to_string`
   without a size limit or a link check), reached through `find_target`'s
   `root.join(CONFIG_FILE).exists()` at `config.rs:841`, which follows links. A mapped tool's
   file, by contrast, must be a real file (`is_real_file`, `config.rs:775`).
   The attack: a download ships `harness.toml` as a link to a file outside the project. A relative
   link such as `../../.git-credentials` reaches the home folder from a typical download folder.
   The TOML parse error then quotes the failing line, and the `--json` error event carries the same
   line, which reaches an agent through harness-mcp. A FIFO in place of the file hangs every
   command, the cockpit and harness-mcp included, because the read has no timeout.
   Ran it: yes. A link to a fake credentials file printed `aws_access_key_id = AKIAFAKE…` in both
   the plain error and the `--json` error. A FIFO hung `harness scan` until it was killed at 8 s.
   This bug existed before step (a), but step (c) rewrote this loader and kept it.
   Fix: read the root `harness.toml` the way the ledger files are read: a regular file only, never
   through a link, with a size limit (`read_regular`). Do not quote the source line in the parse
   error, or quote only the line and column numbers.

2. **The features map's mirror can hang forever on a `-include` that names a FIFO, and reads
   listed files with no size limit.**
   Evidence: `crates/harness-oracle/src/featuremap.rs:1320-1338` (`write_mirror_files` adds
   `base.forced_includes()` and reads each path with `std::fs::read`). This runs before any
   compile (`featuremap.rs:205`), in the harness's own process, with no timeout. The `.c` files are
   checked to be regular files (`irregular_c_file`), but the `-include` targets are not, and the
   loader accepts a FIFO (`resolves_inside` checks location only).
   The attack: `flags = ["-includeinc/fifo.h"]` with `inc/fifo.h` a named pipe. `features map` (and
   the cockpit act that runs it) never returns. A huge or sparse file is also read whole into
   memory before the 256 MiB total limit is checked.
   Ran it: yes. The config loaded, and `map_features` was still blocked after 40 s.
   Fix: read every mirrored file with `read_regular` and a per-file limit (the scanner's 8 MiB).
   Also check at load, and again in `Base::resolve`, that a `-include` target is a regular file.

3. **Adoption keeps the token a download ships, so "a different tree at the same path" can still
   be trusted.**
   Evidence: `crates/harness-core/src/adopt.rs` `ensure_token` keeps any well-formed token already
   in `migration/.ruharness-adopted`. `adopt_inner` records that token (design §3.7: "adopting a
   root whose token file already exists records that token and writes none").
   The attack: the author ships a fixed token in an innocent first version. After the person
   adopts it once, any later tree from the same author, unpacked at the same path with the same
   token, is trusted silently: a new ledger, new verdicts, a new crate.
   Ran it: no; this is reasoned from the code and the design text.
   Fix: on `--adopt`, always write a fresh random token, replacing any shipped one, and keep the
   "record the existing token" path for the test helper and the committed fixtures only.

## Low

4. **The unit-crate manifest check does not require an empty `[workspace]`, and does not refuse
   `cargo-features`.**
   Evidence: `crates/harness-oracle/src/unit_crate.rs` `manifest_problem` allows a manifest with
   no `[workspace]` at all. Cargo then searches the folders above the crate for a workspace root:
   the project's own `Cargo.toml`, or one placed under `migration/`, which adoption's
   fixed-names check does not look inside. `cargo-features` is not refused either.
   Effect on stable: the run fails or picks up the project's profile settings, and no code runs.
   On a nightly toolchain, which the pinning allows when the harness itself runs on nightly,
   unstable manifest features could change how the build runs.
   Ran it: no (only stable is installed here).
   Fix: require `[workspace]` to be present and empty (the harness's own manifest already has it),
   and refuse `cargo-features`.

5. **The driver-shape check reads the driver object with no size limit.**
   Evidence: `crates/harness-oracle/src/shape.rs` `object_symbols` uses `std::fs::read(obj)`.
   The design (§3.9) bounds every object read at 64 MiB, and `projectmap.rs` applies that limit.
   The attack: a project header that makes the driver's object very large, for example a big
   initialized array, makes verify read it whole into memory.
   Ran it: no.
   Fix: check the size first and refuse above `MAX_OBJECT_BYTES`, as `projectmap::compile` does.

6. **objsyms has no overall limit on the Mach-O section entries it collects.**
   Evidence: `crates/harness-oracle/src/objsyms.rs` `macho`: each `LC_SEGMENT_64`'s `nsects` is
   read without checking it against the command's own size, so many small commands can each claim
   a large section count over the same bytes.
   Effect: a crafted object could use a very large amount of time and memory. This cannot be
   reached today, because objsyms only reads what the compiler wrote, but it would matter as soon
   as it reads a project's shipped `.o` or `.a`.
   Ran it: no.
   Fix: require `72 + 80 * nsects <= cmdsize`, and stop once the total exceeds the file size divided by 80.

7. **The map profile leaves `/private/var/tmp` readable.**
   Evidence: `crates/harness-oracle/src/sandbox.rs:275`: the denied reads are `/Users`, `/Volumes`
   and `/private/tmp`. `/private/var/tmp` is world-writable, persists across reboots, and may hold
   other tools' files. §6 names `/private/var/folders` as readable, but not this folder.
   Ran it: no.
   Fix: add `/private/var/tmp` to `MAP_DENIED_READS`, or name it in §6.

## Holds (attacked and found closed)

- The flag grammar: `@` and `-` values, control characters, `-B`, `-fplugin`, `-Xclang`, `-o`,
  the `-M` family, `-l`, separate-argument forms, `-std=` oddities and unknown `-f` spellings are
  all refused. A joined prefix that clang would read as a longer option, such as `-include-pch` or
  `-isystem-after`, starts with `-` once the prefix is removed, so it is refused. `-I=dir`
  (sysroot-relative) fails the clean-path rule.
- Case tricks on macOS (`-IMIGRATION/map`): `realpath` returns the case as stored on disk, so
  `resolves_inside` and `Confine::place` both see `migration/`. Checked with `/bin/realpath`.
- A dangling link at load time that later points into `migration/`: `Base::resolve` resolves
  every path again at each run, and `project_path` refuses it then.
- The map sandbox reached through the firmlinked `/System/Volumes/Data/...` spelling: the deny
  still applies (ran it with `sandbox-exec` and `cat`/`head`).
- `--tool` ids: the strict id pattern, and the tool must be a real folder holding a real file, so
  a link in `migration/tools/` is no tool, and `..` is not an id.
- Adoption: the token is read only from a regular file, never through a link; `--adopt` deletes
  links themselves, never their targets (`remove` checks with `symlink_metadata`, and Rust's
  `remove_dir_all` does not follow links); a `migration/` that is a link is refused as foreign;
  harness-mcp only checks and never adopts; `$RUHARNESS_ADOPTED` comes from the person's
  environment, and children run with a cleaned environment.
- The walk: links are never descended, FIFOs and devices are listed as issues and never read,
  dot-folders and `migration/` are pruned by name and by canonical path, and depth and count limits apply.
- The scanner reads a file only after checking it is a regular file and within the size limit.
- The work folder: a link anywhere along its path is refused, and a temporary folder is refused.
  Cargo does not read a project's `.cargo/config.toml` or `rust-toolchain.toml` (the existing test
  passes). `PATH` drops relative entries and entries inside the project, including a different-case
  spelling, because entries are resolved before they are compared. `RUSTUP_AUTO_INSTALL=0` is set.
- The map's compile keeps only identifier-shaped symbol names, deletes objects once read, and
  limits object size. The error line is reduced to a closed reason and scrubbed of machine paths.


---

# Steps (a) and (c): what a person meets, and what the tests prove

Checked at 610fd50 in a worktree, binaries built from it. Everything ran under a scratch root
(`…/scratchpad/u/`, written `U/` below) with `RUHARNESS_ADOPTED` pointed at a scratch file, so the
person's adoption file was never touched. The fixtures: a copy of targets/zopfli (`z1`); the
liblzg-shaped tool from crates/harness-cli/tests/file_list.rs (`lzg`), the same with a second tool
`t-other` (`two`); the `t-pair` tool with `-DPAIR_WIDE=4` (`pair`); and small one-off folders
(`own`, `ft`, `ff`, `b1`–`b14`). Nothing was downloaded. The worktree is clean again (every revert
restored with `git checkout`).

## High

**1. A project with its own `migration/` folder is first told to adopt it as harness results.**
Evidence: a folder holding `migration/001_init.sql` and `src/a.c`:
`harness project map --target U/own` prints "error: this folder already holds migration results
made elsewhere (0 units, 0 verified): to trust them here, add `--adopt` once"; only after the
person follows that advice does `--adopt` print "this project has a migration/ folder of its own;
move or rename it, or map a copy". The same with a folder-form target (`scan`). Cause:
`adopt::check` (crates/harness-core/src/adopt.rs:268-283) refuses any existing `migration/`
(`has_ledger` is "the folder exists", adopt.rs:199) before it asks `is_harness_ledger`.
What the person meets: a project with database migrations is asked to "trust" them; the right
sentence (§3.7's own) comes only after wrong advice was followed.
Fix: in `check`, when `migration/` is not the harness's, return the "of its own" refusal first.

**2. A tool the person just wrote by hand is refused as "results made elsewhere"; the same file at
the root is not.** Evidence: `ft` (only `migration/tools/t-a/harness.toml` and `src/a.c`):
`harness scan --target U/ft` → "this folder already holds migration results made elsewhere
(0 units, 0 verified)…"; `ff` (the same, folder form at the root) → scans and records the folder
as created here. Adopting then prints "deleted 0 build folder(s) made elsewhere; the verdicts are
claims made elsewhere until `harness verify` runs them here" for a folder with no verdicts.
Every test that writes a tool by hand quietly passes `--adopt` first
(crates/harness-cli/tests/tools.rs:69 "the first command adopts the project, whose migration/ the
test wrote"; file_list.rs:160 and :320; crates/harness-mcp/tests/file_list.rs:58).
What the person meets: step (c)'s only way to make a tool is by hand, and the first answer is a
trust question about results that do not exist, in words that say they came from elsewhere.
Fix: a `migration/` holding only `tools/<id>/harness.toml` (no facts, plan, units or map) holds no
results: record it as created here, as the root form does; print the "deleted / claims" line only
when something was deleted or counted.

**3. An agent is told to adopt, and nothing tells it not to.** Evidence: harness-mcp on the
unadopted `pair` answers every call with `"kind":"not-adopted"` and the CLI's sentence "…to trust
them here, add `--adopt` once"; harness-mcp has no `--adopt`, so an agent's only route is
`harness --adopt …` in a shell. The design (docs/PROJECT-MAP-DESIGN.md:528) says the
`sync-runtime` block tells agents never to pass `--adopt` unasked; the block
(crates/harness-core/src/runtime_view.rs:106-110, and the AGENTS.md it wrote for `two`) has no such
line — `grep adopt runtime_view.rs` finds nothing.
What happens: an agent reading the refusal does exactly what it says, and the trust decision the
design keeps for the person is made by the agent.
Fix: harness-mcp's refusal says "ask the person to adopt it (`harness … --adopt`, or the cockpit's
question); an agent never adopts"; add that line to every generated block.

**4. A typo in a listed file gives an empty tool, and every command exits 0.** Evidence: `b8`
lists `src/nope.c`: `scan` → "skipped src/nope.c: cannot be read: No such file or directory",
"0 files", exit 0; `plan` → "no changes (0 units)", exit 0; `state status` → "facts fresh (0 files,
0 stale vs tree)"; facts.jsonl holds only its header. Removing a listed file from `lzg` and
scanning: `state status` → "facts fresh (5 files, 0 stale vs tree)" while harness.toml still
lists six. Nothing records the missing file (the design keeps walk errors as facts).
What the person meets: a mistyped path silently becomes a smaller tool, all green.
Fix: refuse a listed path that is missing or not a regular file, naming the entry ("[target] files
entry `src/nope.c` does not exist; fix the path"), or record it as an issue that `state status`
reports as stale.

## Medium

**5. Next-step hints leave out `--tool`, so in a two-tool project they lead into a refusal.**
Evidence (`two`, `--tool t-lzg` given): `features init` → "…then run `harness features map`";
`perf init` → "…then run `harness perf run`"; `perf show` → "nothing measured yet — run harness
perf run"; "loading facts (run `harness scan` first)" (crates/harness-cli/src/perf.rs:136,
promote.rs:393, features.rs:110, gen_driver.rs:112, hand_edit.rs:35, main.rs:790, 830, 944, 1143,
1185); the `sync-runtime` block lists bare `` `harness scan` · `harness plan` … `` with "(all take
`--target` and `--tool t-lzg`)" (runtime_view.rs:107). Running a hint as written refuses: "U/two
has 2 mapped tools and no harness.toml of its own; pick one with --tool (t-lzg, t-other)".
Fix: one helper that spells "harness X --tool <id>" when a tool is open, used by every hint and by
the block's command list (an agent copies commands literally).

**6. The workloads file is named by its folder-form path when a tool is open.** Evidence:
`harness perf run --target U/two --tool t-lzg` with a bad tool workloads file → "error:
migration/perf/workloads.toml line 2, column 1: unknown key "extra" … — fix it, or Edit the
workloads file"; with no workload → "add a [[workload]] to migration/perf/workloads.toml". Code:
crates/harness-core/src/perf/workloads.rs:80-91 and 115-119 build the path from `MIGRATION_DIR`;
the cockpit's Speed view shows the same `blocker()`. Also features.rs:264 "migration/features must
be a directory". The file to fix is `migration/tools/t-lzg/perf/workloads.toml`.
Fix: pass the ledger's relative path into `WorkloadsError` and `blocker()` (and the features one).

**7. Nothing on the cockpit's screen, or in harness-mcp's status, says which tool is open.**
Evidence: the cockpit drawn headlessly on the tui's own `file_list_tool` fixture (a scratch test
in view.rs, removed): the title is "harness-tui-zz-review-2471 · ✓0/2", the tree's root the folder
name; "t-lzg" and "lzg" appear nowhere. harness-mcp's `harness_status` on `two --tool t-lzg`:
`"target":{"text":"U/two"…}` with no tool, and its start line "serving U/two".
What the person (or agent) meets: in a project of two tools there is no way to see which ledger
the screen or the answers come from.
Fix: "(tool t-lzg)" in the cockpit's title row, a `tool` field in the status and the start line.

**8. "Not a target" and "no such tool" read differently in the CLI and the cockpit, and one
sentence contradicts itself.** Evidence: `harness state status --target U/empty` → "io error at
U/empty/harness.toml: No such file or directory (os error 2): No such file or directory (os error
2)"; `harness-tui --target U/empty` → "… is not a harness target (no harness.toml, and no mapped
tool under migration/tools/); start with `--target <target dir>`". `--tool t-x` on the zopfli copy
→ "has no mapped tool t-x (it has none); pick one of them with --tool, or run `harness project map`
to see its programs". The "two tools, no --tool" refusal exits 1 in the CLI, 2 in the cockpit and
harness-mcp. Fix: one harness-core sentence for "no target here" used by all three; for a project
with no tools: "this project has no mapped tools; drop --tool (its harness.toml is the target)".

**9. `--adopt`'s words undersell what is trusted, and the "claims" mark does not last.**
Evidence: the help says "Trust the migration results … deletes their build folders; their
verdicts stay claims until `harness verify` runs them here". It does not say that from then on
the harness builds and runs the code the folder brought — its drivers, Rust crates, features and
workloads — inside the sandbox, with its harness.toml's flags. After adopting `z1`,
`state status` prints "u001-katajainen [verified] plan=fresh verdict=green (fresh)": nothing marks
it as a claim. The cockpit's dialog lists the deletions exactly (crates/harness-tui/src/adoption.rs:13-19)
but says "the token is also written to migration/.ruharness-adopted" even when the folder already
holds one (zopfli's copy: none is written). Fix: one honest line in the help and the dialog ("the
harness will build and run the code it holds, in the sandbox"); either show a "made elsewhere"
mark until re-verified or drop "stay claims"; say "the token already there is recorded" when so.

**10. `project map`'s first screen is hard to read for someone who has not read the design.**
Evidence (`pair`, right after its verify was green): "app/main.c — did not compile (syntax at
inc/pair.h:4)" — the header's `#error` for the missing `-DPAIR_WIDE`, called "syntax", because the
map does not use the tool's configuration in this step; the header line "with Apple clang …,
flags -O2 -ffp-contract=off" reads as the project's flags but is the harness's own; every header
shows "include folders: none"; "skipped folder: migration (7 C files)" gives no reason; "defines 3,
needs 19" never says what a need is; on an empty folder it prints "0 files … nothing written yet"
and then the error; the run ends on "nothing written yet" with no next step.
Fix: label the harness's own flags; leave include folders off header lines; "skipped folder:
migration (the harness's own files)"; "an #error directive" rather than syntax; skip the summary
when refusing; a closing line saying what the map shows today and that it changes nothing.

## Low

**11. Two sentences point at `project map` for things it does not show yet.** "…or run `harness
project map` to see its programs" and the `--tool` usage error's "as `harness project map` prints
them" (docs/SCHEMAS.md:1624, 1638): today's map prints files, not programs or tool ids. Fix: drop
the pointer until step (b), or say "files".

**12. A file-list `harness.toml` without `schema_version` is sent the wrong way first.** `b14`
(holds `files`) → "`schema_version` is missing; write schema_version = 1"; doing so (`b3`) → "[target]
files is the file-list form; write schema_version = 2". Fix: suggest 2 when `files` is present.

**13. The adoption refusal says "this folder" without its path, and the cockpit tells you to
"add --adopt".** `Error::NotAdopted` carries `root` but its text (error.rs) does not print it — in
harness-mcp and `bench` the folder is not obvious. The cockpit away from a terminal says "…add
`--adopt` once (start the cockpit in a terminal to be asked, or run any harness command on it with
--adopt)"; `harness-tui --adopt` is a usage error. Fix: name the path; the cockpit's sentence says
only its own way.

**14. The cockpit's `◌` means two things.** In the tree it is "not part of this tool"
(crates/harness-tui/src/files.rs:601), in the help's Features legend "all its code is still C"
(view.rs:3000); the help's States list has no entry for the tree's `◌`, and the tree shows the
label only on the selected row, cut ("decode.c not part o…"). Fix: another mark, listed in States.

**15. Some config refusals are serde's words.** "[target]: unknown variant `Makefile`, expected
one of `make`, …", "[target]: unknown field `include_dir`, expected `path` or `include_dirs`" (the
field is in a files entry, not `[target]`), "[target]: missing field `configuration`". They name
the thing but not what to write; the hand-written ones beside them do (b5–b7, b11, b13).

## Tests

About 120 tests were added or changed by (a) and (c). Most prove what their names say, with real
fixtures (the `#error` in `pair.h` that makes the configuration's `-D` load-bearing; two
`conf.h` in two folders for per-file include folders; zopfli's and the 100 cases' committed facts
byte-identical). They are deterministic (temp folders named by pid and a random tag; a per-process
adoption file) and bounded (slowest: harness-mcp's e2e 121 s, the CLI's file-list verify 17 s).
crates/harness-cli/tests/tools.rs never removes its temp folders; file_list.rs leaves them on
failure.

Six reverts, each the smallest piece, each restored afterwards:

| Reverted | Result |
|---|---|
| harness-mcp's acts no longer pass `--tool` (crates/harness-mcp/src/acts.rs:89) | all harness-mcp tests pass — no test runs an act on a tool |
| `TargetContext::open` skips the adoption check (crates/harness-core/src/config.rs:891) | caught: the cockpit read-model test and the CLI's adopt test fail (harness-mcp's own check, server.rs:397, still holds) |
| the v2 program record without `extra_link_args` (crates/harness-core/src/features.rs:647) | all harness-core and harness-scan tests pass; the v2 test (features.rs:2033) varies flags, folders, configuration name and run name, never the link arguments §3.7 lists |
| harness-detect's pruned list emptied (crates/harness-detect/src/lib.rs:1286) | caught by `the_detectors_prune_the_ledger_when_source_dir_is_the_root` |
| adoption no longer deletes each tool's build folders (crates/harness-core/src/adopt.rs:599-601) | harness-core and the CLI's adopt, tools and file_list tests all pass; the `shipped` fixture (adopt.rs:768) has no `tools/` |
| the cockpit binary's background re-read ignores `--tool` (crates/harness-tui/src/main.rs:1033) | all harness-tui tests pass (tests reload through `App::reload`, app.rs:1211, a separate path); in a two-tool project the first refresh after any act would refuse |

Weak or misleading tests:
- `a_file_list_target_is_refused_by_no_command` (tools.rs:208-224) only checks that stderr lacks
  the two old refusal phrases; no exit code, no reason. A new refusal in other words passes.
- `a_tools_acts_carry_its_id_after_the_target` (crates/harness-tui/src/app.rs:5712) sets
  `config.tool` on a folder-form fixture: it proves the argv, not that a tool's act works.
- `the_projects_own_migration_folder_is_not_adopted` (crates/harness-cli/tests/adopt.rs:145) and
  every hand-written-tool test pass `--adopt` on the first command, so the sentences of High 1 and
  High 2 are never seen by a test.

What §4 lists for (a) and (c) that no test covers:
- `project map` refused with no sandbox (only the help text mentions it).
- A cut-short walk through `project map`: exit 1 by name with the file facts printed (only the
  core walk's cap is tested).
- "The 100 benchmark cases … the same include_dirs bench init writes": one case is mapped
  (crates/harness-oracle/src/projectmap/tests.rs:444).
- An object over 64 MiB (`too-large-object`, projectmap.rs:843): no test.
- `--tool` run through the binary for `review`, `migrate`, `override`, `promote`, `features save`,
  `perf save`, `perf show` (each accepts it; none is run), and harness-mcp's acts (revert 1).
- `--adopt` deleting a tool's `target/` and `build/` (revert 5).
- The v2 digest moving with the link arguments (revert 3).
- A missing listed file (High 4); §4's "a listed file vanished" is tested only before a rescan.
- Belongs to later steps, noted so it is not lost: zopfli as a file-list target giving
  byte-identical facts and `source_hash`es with `verify u001-katajainen` green.

## Holds

- The lookup order works as written in the CLI, the cockpit and harness-mcp; the two-tool refusal
  names both tools and says `--tool`; a bad id is a usage error in one sentence.
- Every tool command writes only under `migration/tools/<id>/`; `verify` on `t-pair` is green with
  the configuration's `-D` reaching the driver, the boundary check and the feature run; the
  awaiting `resume` carries `--tool=t-pair`.
- `harness.toml` v2 refusals are one sentence, name the file and the entry, and say what to write
  (both forms, files under v1, source_dir under v2, schema 3, `-fuse-ld=/x`, `-DFOO BAR`,
  `../elsewhere`, `migration/x.c`, `./src/a.c`).
- `sync-runtime` keeps one block per tool and its `--check` names the right command.
- The cockpit lists several tools to pick from in a terminal and greys files outside the tool.
- `--json` for `project map` follows the events stream: header, one `project-file` per file with
  raw paths, `result`; a newline in a file name shows as `?` in text and escaped in JSON.


---

# Triage (the session, 2026-10-08)

Every finding above was read against the design and the person's rule (simplicity means
usability). What follows is the decision on each, grouped into three fix passes by file ownership,
plus what goes to step (b)'s part 3 and what is recorded rather than fixed. The fix passes are
built by Opus builders at medium effort from a precise rule; the rules are decided here.

## Decisions that change the design (recorded in DECISIONS and the design's §9)

1. **One include rule, in one place, with the configuration's path flags in it.** The oracle's
   second finding, the readers' first, third and sixth are one defect: every compile honours the
   configuration's `-I`, `-iquote`, `-isystem` and `-include`, and no reader does. The rule is
   the compiler's own search order, written once in harness-core's `sources.rs` and used by the
   scanner, staleness, `program_paths`, the planner's `source_hash`, the prompt scope, and the
   oracle's `unit_headers`, `driver_folders` and `unit_header_names`:
   - a quoted include: the including file's own folder; the configuration's `-iquote` folders in
     order; the configuration's `-I` folders in order; the file's `include_dirs` in order; the
     configuration's `-isystem` folders; then the system;
   - an angle-bracket include: the same without the own folder;
   - each `-include` file of the configuration is the first include of every listed file, and
     its own includes are followed like any header's.
   The oracle keeps its argument order (the judge's flags, the configuration's flags, then the
   file's `-I`), which gives exactly this order, and a test proves it: `cc -M` over a fixture that
   has every flag kind reads the files the resolver predicts.
2. **One include reader.** The scanner takes include names from the lexical reader in
   `sources.rs` (over all 496 committed files, lz4 and liblzg the two readers agree exactly, so the
   101 committed facts stay byte-identical), the `<a//b.h>` case is fixed in it, and the test that
   enshrined the difference becomes one that checks both readers agree on each crafted case.
3. **A missing listed file is refused when `harness.toml` loads**, naming the entry, as a path
   outside the root already is. A tool can no longer shrink silently; the staleness case "a listed
   file vanished" becomes unreachable and is dropped.
4. **The harness's own C runtimes compile without the configuration.** The guard and probe
   runtimes take only their own folder; the boundary wrapper, which includes the unit's headers,
   keeps the configuration and is made C89-clean. `-std=c89` joins the file-list test.
5. **A file-list verdict records the configuration** as one toolchain entry
   (`configuration: <name> <blake3 of the flags and each file's folders>`), present only for the
   file-list form so folder-form verdicts stay byte for byte; `state status` marks the verdict
   stale when it differs.
6. **`--adopt` always writes a fresh token**, replacing any the download shipped; only the test
   helper and `bench`'s suite adoption (whose trust is `corpus.lock`) record a token already there.
   The two committed token files are untracked and the `.gitignore` exceptions removed: the
   person adopts `targets/zopfli` once per computer, as any target.
7. **A `migration/` that holds no results is the project's own ledger, made here:** one holding
   only `tools/<id>/harness.toml` files or `map/config.toml` (no facts, plan, units, map file or
   verdicts) needs no adoption and is recorded as created here. The "of its own" refusal (a
   `migration/` that is not the harness's) comes before any adoption question. The "deleted N
   build folders; the verdicts are claims" line prints only when something was deleted or counted.
8. **An agent never adopts.** harness-mcp's refusal says "ask the person to adopt it (`harness …
   --adopt`, or the cockpit's question); an agent never adopts", and every generated
   `sync-runtime` block carries that line.
9. **Next-step hints spell `--tool <id>` when a tool is open**, through one helper used by every
   hint and by the `sync-runtime` block's command list.

## Fix pass A — the readers (harness-core, harness-scan, harness-detect, harness-llm, the CLI's staleness)

- The one include rule (decision 1) and the one include reader (decision 2).
- Staleness compares the include a file would resolve now with the one its record names, both
  ways (readers 3); a header whose includes resolve differently under two listed files is an
  ambiguous-include fact, not a silent union (readers 6).
- An unreadable file is a fact the scan records and detect skips with the same note, so it is
  neither a permanent staleness nor a hard stop (readers 4).
- The prompt scope and detect resolve includes afresh through the resolver instead of trusting
  `facts.jsonl`'s edges, and refuse dot-folder paths; the forged-edge test (readers 5).
- An include that lands outside the root or under `migration/` is a scan note (readers 7); a
  dot-named link is not counted through the link (readers 8); detect reads through
  `read_regular` with the 8 MiB cap (readers 9); the v2 digest hashes the resolved forms of
  folders and flags, and a test moves it with the link arguments (readers 10; usability's third
  revert); the folder form compares a linked `source_dir`'s files by canonical paths (readers 11).
- A missing listed file refused at load (decision 3); the root `harness.toml` read as a regular
  file with a size cap, never through a link, and the parse error names line and column without
  quoting the line (security 1); a `-include` target must be a regular file at load (security 2,
  the load half); a file-list file without `schema_version` is told to write 2 (usability 12);
  the three serde-worded refusals get their own sentences (usability 15).

## Fix pass B — the oracle (harness-oracle, plus `status.rs`'s compare for decision 5)

- The runtimes without the configuration and the C89-clean wrapper (decision 4); the
  configuration entry in the verdict and the status rule (decision 5); the `cc -M` test of the
  search order (decision 1's oracle half; `unit_headers`, `driver_folders` and
  `unit_header_names` call the shared resolver once fix pass A lands — build against the rule
  as written and switch to the resolver at the merge).
- A mutant's original folder passed as `-iquote` before the configuration's flags (oracle 4).
- `objsyms` reports absolute and indirect external symbols so the driver-shape check sees them
  (oracle 5); the driver object is refused above 64 MiB (oracle 6, security 5); a budget on name
  bytes (oracle 7); the Mach-O section count checked against the command's size (security 6).
- The unit-crate check requires an empty `[workspace]` (zopfli's hand-written crate has one),
  refuses `path` keys in `[lib]` and `[[bin]]`, the underscore dependency spellings and
  `cargo-features`, and runs on the symbol-baseline crate too (oracle 8, security 4).
- The mirror reads every file through `read_regular` with the 8 MiB cap (security 2); the
  per-build object folders are removed after a successful link (oracle 10); `/private/var/tmp`
  joins the map profile's denied reads (security 7).
- Not in this pass: the missing header's raw name (oracle 9) goes to step (b) part 3 with the map
  file, since it is only stored from there.

## Fix pass C — the person's side (harness-cli, harness-tui, harness-mcp, adopt.rs, runtime_view.rs)

- Adoption: decisions 6, 7 and 8; the help and the cockpit's dialog say that the harness will
  build and run the code the folder holds, in the sandbox; "the token is written" is now always
  true (usability 9); the refusal names the folder's path and the cockpit's sentence names only
  its own way (usability 13).
- Hints with `--tool` (decision 9, usability 5); the workloads and features errors name the
  tool's ledger path (usability 6); the tool's id in the cockpit's title row and in harness-mcp's
  status and start line (usability 7); one harness-core sentence for "no target here" used by the
  CLI, the cockpit and harness-mcp, one for "this project has no mapped tools; drop --tool", and
  one exit code for "two tools, no --tool" (usability 8); a mark of its own for "not part of this
  tool", listed in the help's States (usability 14).
- Tests for what the reverts showed uncovered: harness-mcp's acts carry `--tool`; the cockpit
  binary's background re-read carries `--tool`; adoption deletes a tool's `target/` and `build/`;
  `--tool` through the binary for review, migrate, override, promote, features save, perf save and
  perf show; `project map` refused with no sandbox; a cut-short walk exits 1 by name with the facts
  printed; `a_file_list_target_is_refused_by_no_command` checks exit codes; the hand-written-tool
  tests drop their first `--adopt` once decision 7 makes it unneeded.

## To step (b), part 3 (the map file and the screen)

The first screen's wording (usability 10: the harness's own flags labelled, no include folders on
header lines, "skipped folder: migration (the harness's own files)", an `#error` named as such,
no summary when refusing, a closing line), the two sentences that point at `project map` for
programs it does not show yet (usability 11), the missing header's name kept only when it is a
clean relative path (oracle 9), and b1's note that a program's files with differing entry flags
make a flags-differ fact that keeps the source guessed.

## Recorded, not fixed

- Dot-folders are no longer scanned or hashed in the folder form (readers 11, first half): none
  of the 101 committed roots has one, and the map's walk prunes them by design.
- The `rustc-wrapper` half of `cargo_never_reads_a_projects_config_or_toolchain_file` proves
  nothing because `cargo metadata` never runs rustc; the other two halves carry the test. Left as
  is, noted here.
- `tests/file_list.rs`'s comment that each file finds `pair.h` only through its own folder is not
  exercised there; the lib test covers per-file folders. The comment is corrected in fix pass B.
