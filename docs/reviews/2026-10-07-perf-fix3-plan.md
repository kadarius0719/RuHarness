# Fix pass 3 of the speed work — the triage of the check round (2026-10-07)

Source: docs/reviews/2026-10-07-perf-fix2-check.md (17 Opus agents at c92f6ec). Triaged by the main
session (Fable). Kept: what two checkers agree on, or what one checker reproduced with a probe or a
measured run. Dropped: what is already done, or an observation with no wrong behaviour behind it.

## Kept, grouped by the area that fixes it (one Opus fixer per area, its own worktree)

### A. perf show and perf run (harness-cli `src/perf.rs`, `tests/perf.rs`; harness-oracle `perf/tools.rs`)

1. **(medium, both CLI checkers, reproduced)** Without facts, a program-as-it-stands row reads current
   although the units it holds were never checked (`measurable` is None, so "left out now",
   "accepted since", "verified since" and "the plan's order changed" are all skipped silently).
   Fix: when facts are missing (or `perf_measurable` fails, the `.ok()` on line ~465) and an
   as-it-stands row is stored, the one not-checked line says so: "perf: the C and the units the
   program as it stands holds not checked: no facts — run harness scan". With only C-alone or unit
   rows stored, keep today's line. Test: an as-it-stands row holding u001, u001 set to pending in
   plan.toml, facts removed — the wider line is printed once, the row has no out-of-date line; with
   facts, "u001-katajainen is left out now".
2. **(low, both CLI checkers)** The early `--as-it-stands-only` refusal says "one unit measured"
   before anything is measured, and does not name the units left out or why (design §3.10 says it
   "says why"; §3.2's example is "one unit measured (u001) — u-tree left out: verify it first").
   Fix: the early check gets the left-out units and their reasons (a small public function in
   harness-oracle returning each NotVerified id with `left_out_words`, or `perf_measurable`
   returning them) and appends " — u-x left out: …" as the full run does; say "one measurable unit
   (u001)" / "no measurable unit" in both the early and the post-build refusal (measure.rs ~1113).
   Update the tests and docs/TESTING-GUIDE.md:3764 if its words change.
3. **(low, one checker, measured 2.4 s)** perf show runs `rustc -V` even after `cc --version`
   failed (`compiler_lines` builds the tuple first). Fix: perf show stops at the first failure
   (perf run keeps the tuple: it needs both). Test: a stand-in cc that fails and a rustc that
   sleeps — rustc is never started (or the time stays under the rustc timeout).
4. **(low, one checker, reproduced)** One bad unit results file now hides every row in perf show
   (it reads every unit file up front with `?`). Fix: collect each unit's read result; print the
   program's rows and the units that read; then report the bad file(s) and exit 1 — as the cockpit
   shows the other rows. Test: a junk `units/u001-katajainen.json` beside a stored C-alone row —
   the C's row is printed, the error names the file, exit 1.

### B. The cockpit (harness-tui `speed.rs`, `app/speed_acts.rs`, `model.rs`, `main.rs`, `spawn.rs`; harness-oracle `perf/measure.rs` `perf_launcher_cached`, `perf/launcher.rs` `existing_launcher`)

5. **(medium, one checker, reproduced)** Without facts the cockpit and harness-mcp claim every held
   unit is "left out now" (Snapshot::load returns early before adding units, so `speed::build`
   passes `Some(&[])`). Fix: pass `measurable: None` when the snapshot has no facts, and say in the
   same header line as the C that the units the program holds are not checked either — the same
   words as A1. Test: Snapshot::load + speed::build without facts → `out_of_date` empty, the header
   says so; with facts and u001 pending → "u001-katajainen is left out now".
6. **(medium, three checkers)** The Measure dialog answers for the cockpit's own build's launcher
   cache, but the run uses whichever `harness` the cockpit resolves (PATH first). When the builds
   differ, the dialog can drop the cache line (and the 25 s) although the run will build the
   launcher, or the reverse. Fix: treat the answer as definite only when the `harness` the act
   runs is the cockpit's own build (the binary next to `current_exe`, compared canonically);
   otherwise use the existing "answer not known" path (the hedged line, the estimate without the
   build). Say it in a comment. Test: a config whose harness is another path → the hedge.
7. **(low, one checker + the open mutation survivor)** `perf_launcher_cached` answers `false` for
   three states (missing/stale: the run builds; unusable: the run refuses; HOME unset), and
   "current" skips the private-folder checks the run makes (no link, mode 0700, owned by you). Fix:
   a three-way answer from the oracle — current / will build / cannot use (with perf's own refusal
   words); the dialog keeps the plain estimate for "cannot use" and says perf will refuse and why;
   "current" requires the same private-folder checks as the run. Also make `measure_words` take
   its probe and its wait as parameters so the dialog's own wait is tested with a slow probe (the
   open survivor: a blocking call on the UI thread must fail a test), and add a test of the
   "will build" side that does not depend on this machine (an inner function taking a HostDirs
   whose cache folder is missing).
8. **(low, one checker)** The dialog labels all 25 s as "builds perf's launcher"; the design and
   the testing guide say the build is about 5 s (the rest is two first starts). Fix: "about 25 s
   of it for perf's launcher (its build and first start)"; the Runs form likewise; tests updated.
9. **(main session)** harness-tui `spawn.rs` test `the_signal_path_interrupts_and_waits_within_its_budget`
   (~line 433; also its `trap '' INT` child) ends its looping child only on the passing path: eight
   loops 7–8 days old were found running. Fix: a kill-on-drop guard (SIGKILL the child's pid when
   the test ends, pass or fail); check every test that spawns a loop (`grep -rn "while :" crates`)
   for the same shape.
10. **(trivial)** harness-tui Cargo.toml's comment calls harness-scan "already here"; it is new to
    the cockpit's tree (DECISIONS states it correctly). Fix the comment.

### C. The results reader and the words (harness-core `perf/results.rs`, `perf/words.rs`, `perf/currency.rs`; harness-oracle `perf/build.rs`, `perf/measure.rs` select/run)

11. **(medium, both core-docs checkers, traced)** The 999 cap on `left_out` can stop a real run on
    a plan of more than 999 accepted units (nothing refuses such a plan up front; only
    `Slot::name` refuses a slot past 999). Fix: `perf run` refuses a plan of more than 999 units
    by name before selecting anything, as PERF-DESIGN §3.2 says ("a plan of more than 999 units
    is refused by name"); harness-oracle's `MAX_SLOTS` reuses `harness_core::perf::results::MAX_UNITS`
    so the two cannot drift. Test: a plan of 1 000 units is refused by name before anything is built.
12. **(medium, one checker, measured: 85 MB peak on a 4 MiB forged file)** The caps run after the
    whole file is parsed, and it is parsed twice (`parse_json` reads `schema_version` through a
    full `serde_json::Value` first); the rows lists are not capped. Fix: read `schema_version`
    through a small struct holding only that field (no Value tree); cap the rows lists (`c_alone`,
    `as_it_stands`, a unit file's `rows`) at `workloads::MAX_WORKLOADS` (16 — the writer already
    prunes rows to the workloads file). No counting visitor (the 4 MiB file cap bounds the rest;
    say so honestly). Tests: a file with 17 rows in a list is refused by name; a forged file is
    refused with one parse (a test that the version read does not build a Value is hard — at least
    assert the peak does not double: skip if not measurable, but keep the rows-cap test).
    SCHEMAS' sentence "The caps bound what the reader accepts and hands on" is reworded by area E.
13. **(open mutation survivor, a real bug)** words.rs `ratio_end`: the "3.7× as slow" low end can
    read "1.0×" when the low end is a float hair past 2 % (probe: "3.0× as slow (1.0–3.5×)" for a
    Rust at exactly +2 % in the pair that sets the low end; right: 1.03×). Fix: decide the crossing
    in percent (`lo > m`) and always round up from that side, as `end()` does; pin "1.03×" in a test
    with the probe's inputs (a C at 1e9 cycles, a bimodal Rust at exactly +2 % and up to +290 %).
14. **(fixer's note)** `check_row` caps `last_try`'s units but never checks their content (clean
    ids, crate digests) unlike `inputs.units`: check them the same way. Test: a forged last_try
    with an odd id is refused by name.
15. **(mutation tester's note)** currency.rs: the accepted arm's guard `!left.contains(id)` is
    always true and the trailing `None => {}` arm can never run — simplify (behaviour unchanged;
    the currency tests stay green).

### D. The launcher tests and nearby comments (harness-oracle `perf/launcher.rs` tests and the `run_measured` doc comment, `perf/tools.rs` test comment, `perf/archive.rs` test)

16. **(low, one checker, demonstrated)** `pid_of_never_takes_another_run_s_program`'s decoy is
    adopted by launchd, so a weaker rule (reject only a launchd child) still passes. Fix: start the
    decoy one level deeper under a living parent that is not ours
    (`sh -c '( "$0" & echo $!; wait ) </dev/null 2>/dev/null & sleep 0.5' decoy`, first line), and
    check the decoy's parent is > 1 and not this process. Show the weaker rule fails it.
17. **(low, one checker)** `perfrun_stays_idle_while_the_program_runs`: `parent_of(program)` runs a
    second `ps` and panics on an empty answer if the 3 s sleeper is gone. Fix: `pid_of` returns the
    parent from the snapshot it already took (or a `pid_and_parent_of`), or an empty ppid read
    counts as a late reading and the run is tried again.
18. **(fixer's note)** Several launcher tests use a short fixed sleep before `!alive(pid)`
    (a_launcher_killed_alone_takes_its_program_with_it 100 ms; cancel_perf_child_body 200 ms;
    after_the_go_ahead_perfrun_holds_the_child_until_the_bye 100 and 300 ms;
    before_the_go_ahead_the_program_never_runs 100 ms). Fix: a bounded "dead within N seconds"
    wait helper (poll up to, say, 5 s), so a busy Mac cannot flake them.
19. **(two checkers)** `run_measured`'s doc comment (launcher.rs ~848) still gives the old step-3
    order. Fix: "check cancelled() first (close the socket without the go-ahead); else retake the
    lock (which refuses once cancelled), register the program's group and write the go-ahead".
20. **(two checkers)** tools.rs test `the_compilers_are_read_as_tool_runs` comment says its Runner
    is "perf run's way" (64 MiB cap, no profile); perf run uses `VERSION_OUTPUT_CAP` under the tool
    profile. Fix: build the test's Runner the way perf run does, or drop the claim; name
    `a_run_over_verified_units_end_to_end` as the real guard in the comment.
21. **(two checkers)** `archive::tests::real_staticlibs_read_right` fails whenever
    `CARGO_TARGET_DIR` is set (its inner `cargo build` writes elsewhere). Fix: the inner build gets
    `--target-dir` (or the env var removed) so the test finds its `.a` either way.

### E. Docs (docs/SCHEMAS.md, docs/PERF-DESIGN.md; no code)

22. **(three checkers)** SCHEMAS' `perf show` entry and PERF-DESIGN §3.9 do not describe: with no
    row stored, nothing is checked and only "nothing measured yet" is said; without facts the C is
    not judged, said once (after A1: the units the program as it stands holds are not checked
    either, in the same line); one bad unit file no longer hides the other rows (after A4: the
    rows that read are shown, then the error, exit 1).
23. **(one checker)** SCHEMAS' `perf run` entry lacks the refusal of a link or non-folder at
    `migration/perf/units` (when the run starts, before anything is built, and before each unit
    row is written). Add it. Also the early `--as-it-stands-only` refusal names the left-out units
    and why (after A2), and a plan of more than 999 units is refused by name before anything is
    selected (after C11).
24. **(one checker)** SCHEMAS:1285 says `crates [{id, digest}]` is on unit rows only; as-it-stands
    rows carry one per held unit. Fix: "(unit rows, one; as-it-stands rows, one per held unit; …)".
25. **(after C12)** SCHEMAS' caps sentence: the caps bound what the reader hands on; the rows lists
    are capped at 16; the version is read without building the whole file; the parse itself is
    bounded by the 4 MiB file cap. No overstatement.
26. **(mutation tester)** PERF-DESIGN §3.5 step 2 says the half-second factor uses the C's CPU
    time; the code uses the smaller of both sides' CPU times (right under the both-legs rule).
    Fix the design.
27. **(one checker)** PERF-DESIGN §3.3 step 3: "the lock refuses the same way once cancelled" is
    not exact — if the cancel handler already holds the lock, `live_lock` waits until the process
    exits instead of refusing; the program still never starts. Say that.
28. **(after B7/B8)** PERF-DESIGN §3.11: the dialog's launcher line is definite only when the
    harness the act runs is the cockpit's own build; otherwise hedged; the three-way answer.

## Dropped, and why

- "No test catches the computer still read when nothing is stored" — already added by the CLI
  mutation tester (`show_checks_nothing_when_nothing_is_stored` now uses an empty home).
- `stale_launchers_go_but_never_one_in_use` failed once at load ~100 and passed 9 of 9 otherwise:
  recorded as a flake to watch, not fixed blind.
- Store::load taken before the builds widens a window for edits outside the harness during a long
  build: the writer lock keeps every harness writer out; an observation, not a defect.
- The CLI and the oracle word the stale-facts refusal differently: both are right and tested.
- A perfrun polling kevent with a zero timeout would pass the idle test: no such code exists.
- The perfrun 5 s exit window after the bye under extreme load: never seen.
- `job-creation 1` printed under every profile: the OS denies it already; only goldens guard it.

## Order after the fixes

A scoped check of this pass (Opus checkers, two per area: "does each fix hold?" and "what did it
break nearby?"), then mutation checks of the new rules (A1's wider line, C11's plan cap, C12's rows
cap and single parse, C13's ratio crossing, B6's own-build rule, B7's three-way answer), then merge,
`bench check --replay` at the merge to main.
