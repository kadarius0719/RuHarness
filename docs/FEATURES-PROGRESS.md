# Features track — progress (resume here)

On `main` (merged and pushed 2026-09-29, paused at the person's request). Design:
docs/FEATURES-DESIGN.md — four design revisions (§R 84, §R2 58, §R3 37, §R4 11 findings), then
the build's code review and four fix passes (§R5); the rule text governs. Spike and the user's
decision (scenarios + verify): DECISIONS.md 2026-09-29.

## Done (each committed green: fmt, clippy -D warnings, cargo test --workspace)

| step | commit | what |
|---|---|---|
| 1 core | c25bff5, 5e4d1b9 | features.toml loader/snapshot/digests, VerdictInputs + AttemptRecord fields, Coverage marker in UnitReport, SkipReason, starter |
| 2 oracle | 52a0802 | run_scenario, feature step after boundary, verify_with, migrate/replay/promote snapshots, CLI messages |
| bench | — | `bench check --replay` after step 2: 198 reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems, OK no regression |
| 3 map | 1cd6b91 | probe_source, fnprobe runtime, map_features, map.json loader, CLI features init/save/map |
| 4a–4c | 0a9f19b, e29430c, 26362d2 | cockpit: featmap, Features group and Views, markers and lines, Next-step rules 5–6, the Edit flow, Help, pty test (Map) |
| 5 | 9e62e29 | chat brief, harness-mcp's closed `features` field |
| 7 | c5f6891 | SCHEMAS, TUTORIAL, README |
| 6 dogfood | a38c2aa | zopfli's features.toml (7 features, 8 scenarios) + map.json; u001 green with 8 feature checks |
| 8 review | — | 4 lenses → 35 findings, each verified: 33 confirmed, 2 refuted (M4, O8) |
| 8 fix passes | 71d9e1a, 6310410, 2de32ef, e98a2a2 (+ test commits 253fa76, 09c458f, 4ee851c, 474383c) | each pass checked by independent agents; the next pass fixed what the check found (§R5) |
| 8 fix pass 5 | bf75af8 | the two lows + the check of the fourth pass (§R5) |
| 8 fix pass 6 | 950eee3 | the check of the fifth pass: per-compile ordered reads, probe rules (§R5) |
| 8 fix pass 7 | b31f6ef | the check of the sixth pass: reads by name and depth, probe split and macro rules (§R5) |
| mutation | — | 87 mutants of the §12 named rules and every fix: 86 killed, 1 equivalent (the copy that cannot list its includes is refused by the difference check anyway) |

960 workspace tests pass (mutants: 20 of the fifth pass, all killed; 15 of the sixth, 14 killed, 1 equivalent; 19 of the seventh, all killed) (one known flake, `harness-core` `a_reader_never_makes_a_writer_fail`,
fails under load and passes alone — listed in DECISIONS "still separate").

Review reports and verdicts (scratchpad, not in the repo): code-review/<lens>/{findings,verdicts}.md,
fixcheck-*.md. Their substance is in FEATURES-DESIGN §R5.

## Open (in order) — resume here (session 17c366c0…, 2026-09-30 → 10-01; continues after context compactions)

**NOW (2026-10-01, after 56cd090 on main):** the probe review is closed and pushed. Next: BUILD the
C-vs-Rust performance baselines from docs/PERF-DESIGN.md (revision 5 + §10's 30 build notes) in
§5's order (b, c, a, d, e, f, g), with the full process; then the briefing's M5. The probe items
below are history.
- Step (b) DONE (0540415, 8e67a30, and the words commit after): `crates/harness-core/src/perf/`
  — `stats.rs` (exact Mann–Whitney null, Hodges–Lehmann), `workloads.rs` (strict reader with
  line/column, starter, digest, the confined input read and its words), `results.rs` (rows,
  strict reader, atomic writer, the replace rule), `words.rs` (all of §3.8 and notes 10–21),
  `mod.rs` (PERF_LAUNCHER, PERF_RUN_LOCK, PERF_RECIPE, interrupted-Accept words);
  `status::promotion_marker` public; the cockpit's `Cause::PromotionInterrupted {attempt,
  unit}` words = perf's.
- Step (c) DONE: `crates/harness-oracle/src/perf/{build,archive}.rs` (whole_cc_into shared with
  verify, compile_objects, link_side, Slot, Hashed, kept_c_files (verify uses it), perf_scratch /
  perf_out / perf_logs, the ar reader + ArchiveFacts). `#![allow(dead_code)]` in perf/mod.rs
  until step (d) wires it — REMOVE then (also the two `#[allow(dead_code)]` on sandbox.rs's
  PerfSpec / render_perf_profile).
- Step (a) DONE (c418809): `perf/{perfrun.c,perfgo.c,launcher.rs}` — find_compiler (root-owned
  paths), launcher()/launcher_in() (cache in ~/Library/Caches/ruharness/perf, 0700, flock with
  identity re-check, hashes, stale removal), parse_record (strict), computer_facts,
  run_measured (control socket, G/B, SIGTERM→SIGKILL, program group killed first via exec's
  ordered LIVE registry); sandbox: HostDirs.perf_cache, every profile ends with a deny-write on
  it, render_perf_profile. DECIDED here (to record in DECISIONS at the perf close): Linux is
  refused by name ("perf runs on macOS only for now — the Linux launcher is not built yet"):
  no Linux machine to compile or test on, CI's Linux job red since 2026-09-17.
- Step (d) DONE: `perf/measure.rs` — `harness_oracle::perf_run(target, plan, facts, workloads,
  perf_dir, &PerfRequest, &mut dyn PerfProgress) -> PerfSummary` (the CLI holds the writer lock,
  loads plan/facts/workloads, resolves perf_dir). Not yet exercised with real verified units —
  that is step (e)'s end-to-end test (a mini target with two accepted units, zopfli's u001).
  `harness_core::perf::manifest_profile` added (note 25).
- Step (e) DONE: `crates/harness-cli/src/perf.rs` + `tests/perf.rs`; `harness_core::perf::
  currency::out_of_date`; oracle `perf_measurable`, `perf_computer_if_cached`. Real result on
  zopfli (800 KB text, 5 runs): the C 2.20 s / 35.1 MB / 3.30e10 instructions; u001-katajainen
  "about as fast as the C (within 2 %)", 1.9 % more instructions. A two-verified-unit end-to-end
  target does not exist yet (zopfli has one): the as-it-stands path is tested at the oracle level.
- Step (f1) DONE: `harness-tui/src/perfread.rs` (Snapshot.perf: workloads state, results files,
  orphans, inputs hashed off a process-wide cache keyed by (dev, ino, size, mtime, identity),
  never while a `perf run` holds the lock, 256 MiB budget; crates' digests; program_now) and
  `speed.rs` (SpeedModel: group label ≤ 19 cols, rows worded by harness-core with currency,
  units worst first, header, unit_header, summary_line); tree `Selection::Speed` (a leaf after
  Features); the Speed View (golden `tests/golden/speed-54.txt`; each row a link, the focused
  row's full sentence below); the unit header's two Speed lines; the summary's Speed line.
- Step (f2) DONE: `harness_core::perf::estimate` (Job/Estimate per §6, c_clock, crate_cold per
  note 27; the CLI's 31-run figure uses it); `app/features_edit.rs` generalised (DraftKind
  Features | Workloads, Draft, App.workloads_draft + App.editing, start_edit / ask_discard_draft /
  discard_draft, Act::SaveWorkloads → `perf save`), quit names kept workloads drafts;
  `app/speed_acts.rs` (speed_gate, measure_argv: Act::Measure [unit], MeasureProgram,
  MeasureMore `--workload=… --runs=31`; measure_words); menu items on Speed and on a verified
  unit; events `PerfRow` (activity line; narrator ends "Measured N rows — see Speed"; MCP
  ignores it).
- Step (f3) DONE: speed.rs `Advice` (difference facts with their age, "Next: Compare the outputs
  …", the slower row's "perf times the Rust in use…"), `change_words` by provenance (Modify +
  Replace and back / Hand edit / commit-edit-verify; "Connect a model to Modify" without a
  provider), `program_differences` (heading + summary; "no unit's Rust differs alone" or the
  per-unit commands); `compare_outputs` (kept files checked by size and blake3, unified diff of
  text or hex rows, controls escaped) behind "Compare the outputs (w)"; Help's Speed section
  and word list; `harness_core::perf::{PERF_OUT_DIR, KEPT_UNITS, KEPT_PROGRAM,
  kept_outputs_dir}` shared with the oracle; `words::difference_words` public;
  `model::short_id` (app re-exports it).
- Step (f4) DONE: harness-mcp `reads::speed_row` / `speed_head` / `unit_speed` — `harness_status`
  gains `speed` (state, units measured/measurable, measuring, the C alone's rows with
  cpu_seconds and memory_bytes, the program as it stands's units, left-out, rows; null without a
  workloads file) and each unit's worst row; `harness_unit` gains `speed` (all rows). Closed
  sets: `words::ANSWERS` (tested over every outcome), `currency::REASONS` (`currency::reasons`
  gives each reason's token; `out_of_date` its words), results' OUTCOMES / PLATFORM_METRICS /
  LEFT_OUT_REASONS; `environment_checked: false`. (The design's `cant-tell-busy` is not an
  answer the words give: busy is a detail.)
- Step (g) DONE: SCHEMAS "C-vs-Rust speed" (workloads.toml, results files and the replace rule,
  currency tokens, CLI, perf-row, harness-mcp fields, trust boundaries — forgeable and
  non-canonical — and writer rows); TUTORIAL "Speed" part + two words; TESTING-GUIDE Part 11
  (liblzg: big input from the sources, two workloads, save with --expect none, measure, show and
  out of date, the cockpit, commit). Part 11 was NOT run on liblzg here (liblzg is not on this
  machine; downloading it needs the person's OK): its fixed lines come from the code and a real
  run on a zopfli copy (scratchpad zperf/: refusals, init, measure, show, out of date all as
  written); numbers are `<n.nn>` placeholders.
- Perf code review DONE: `wf_f4b82b8e-a61` — 84 findings, 82 confirmed by both verifiers (9 high,
  42 medium, 31 low), 2 split (build note 30's Modify words; unreachable today), 0 refuted. Saved:
  `SP/perfrv/result.json`; every finding in full, numbered by severity: `SP/perfrv/findings.md`
  ("## N [severity] lens — title").
- Fix pass 1 DONE: workflow `wf_4260dd05-3f0` (six agents, own worktrees from d8d6d11), merged
  here as six merge commits (core-files, core-words, launcher, measure, cli, cockpit-mcp; one
  SCHEMAS conflict merged by hand). Every one of the 82 confirmed findings was handled (per-agent
  results: `SP/fix1/result.json`). PERF_LAUNCHER is now perf-launcher-2 (perfrun no longer spins
  a core), PERF_RECIPE perf-recipe-2 (a run is short only under BOTH legs of the floor): every
  stored row reads out of date until measured again. The perf sandbox also refuses starting
  programs through the system (LaunchServices). New: `harness_oracle::perf_compilers` (perf
  show's compilers as sandboxed tool runs), `harness-tui/tests/speed_change.rs` (build note 22).
  Left open, small (do them in fix pass 2, after the reset):
  1. DONE 2026-10-03: verify's tool, run and scenario profiles now end with
     NO_STARTS_THROUGH_THE_SYSTEM (goldens updated; live test
     a_run_cannot_open_or_start_anything_through_the_system; SCHEMAS + PERF-DESIGN §3.4 say so);
  2. `perf show` with no facts says "the C changed" on every row (the cockpit side is fixed);
  3. the results reader should cap the lengths of `replaces`, `units`, `left_out` lists;
  4. `perf show` and the writer's Store::put still follow a linked migration/perf/units folder
     (the cockpit refuses it);
  5. the dialogs' estimate cannot add the launcher build: the cockpit needs a cheap public
     "is the launcher cache current" check in harness-oracle;
  6. optional: `perf run --as-it-stands-only` could refuse before building when fewer than two
     units are measurable;
  7. add `assert_eq!(crash.len(), 1)` (and for "time") to harness-cli tests/perf.rs
     two_units_end_to_end now that the C-fails-in-step-1 rule is merged.
  THE PLAN FOR NEXT WEEK IS docs/NEXT-WEEK-PLAN.md (fix check, fix pass 2, mutation checks, the
  DECISIONS draft, merge, then the project map). In short — AFTER THE WEEKLY RESET (2026-10-07): the
  fix check (re-verify all 82 against the merged code + regressions — or, cheaper, the 9 high
  ones), fix pass 2 (the list above), mutation checks, DECISIONS (incl. the Linux refusal), merge
  to main, push.
- ROADMAP (recorded 2026-10-01, not designed): real project layouts (several folders, several
  programs, the project's own build files; a deterministic project map + link closures, a model
  bins the ambiguous part, the harness checks, the person accepts) and a fuller C–Rust boundary
  (non-leaf units, shared code across tools, one Rust library per program). See
  docs/PROJECT-MAP-ROADMAP.md. Design it after the speed work is merged; weigh against M5. Then (g) SCHEMAS, tutorial, the
  testing guide's Part 11; then the code review, fix passes, mutation checks, DECISIONS, push.

**Where things are.** Branch `claude/rust-migration-harness-f7665f` (worktree
`.claude/worktrees/rust-migration-harness-7d1c42`), NOT yet merged to main or pushed. This
session's commits on top of 665495b: 3ed89be 1a0efb0 31610cf 6a03a6d 9bc83d3 7615200 667654b
b5f1e02 1b19786 a5179d6 (probe fix pass 1, parts 1–6 and tests), 6cf7906 (this file), 0b64bba
and 00f2e23 (perf design draft and revision 1). Scratchpad `SP` = `/private/tmp/claude-501/-Users-beaumorton-code-RuHarness--claude-worktrees-rust-migration-harness-7d1c42/17c366c0-a0e0-4aed-a9a5-7011af990c70/scratchpad`;
workflow journals under `/Users/beaumorton/.claude/projects/-Users-beaumorton-code-RuHarness--claude-worktrees-rust-migration-harness-7d1c42/17c366c0-a0e0-4aed-a9a5-7011af990c70/subagents/workflows/<run>/journal.jsonl`.

**The probe's code review (re-run).** Workflow `wf_d2c3d9f7-615` at 665495b: lenses
runner-runtime-sandbox (10 findings), format-ux (12), token-compare (3), mutants-scan-copy (13),
mutants-build-runtime (14), plus 2 verifiers for each of the first review's 22 findings
(`SP/rv/existing.json`) and each new one. Lens results saved: `SP/rv/{runner-lens,
mutants-scan-copy,mutants-build-runtime}.json`; format-ux and token-compare are in the journal
(labels `review:format-ux`, `review:token-compare`). Verifiers were still running at the
compaction; so far only the Latin-1 finding (silent-wrong-map#2) was not reproduced — 665495b had
already fixed it. When the workflow's notification arrives (or by reading the journal): list
every finding with fewer than 2 confirmations and reconsider its fix.

**Fix pass 1 — done and committed** (each fix with a test; mutation check `SP/mutfix.py` on the
copy `SP/mut1`: 49 mutants, 48 killed, 1 equivalent — encoding prefixes): the whole-text
tokenizer for the copy check (raw strings, digit separators, `.incbin` across lines); bounds as
designed (+ `with_map_bounds` test seam); link rules (a)/(b) by object, the link search, GNU ld
parsing; runtime names from object symbol tables (`objsyms.rs`, no link map); the hidden #if
variant (kind parser); scratch-folder cleanup on a signal, 0700, one text in memory (1.76 GB →
75 MB); listing timeouts worded; tool profile reads its own write dirs/TMPDIR under home; safe text
everywhere (`harness_core::text`); strict map.json reasons; killed compiler worded; main() hint;
the scanner: swallowed definitions recorded (nested, rule 1; outer misread body rule 1), keywords
never functions, heads run together rule 1; the ledger flake's cause (mkfifo fork) moved to
core_tests; driver validation times runs 2–3 only; many §4 tests.

**Still to do for the probe, in order:**
1. DONE: `wf_d2c3d9f7-615` FINISHED (`SP/rv/result.json`): 71 findings confirmed by both verifiers,
   1 by its one verifier, 1 refuted (the Latin-1 one, fixed at 665495b before the review). The build/runtime sweep's missing tests: DONE
   (44bee5d, ten adapted from its kill-tests.diff). Before that these were missing — the search with its restore pass (asm "i" fixture), the weak-function link order,
   placement tests asserting kind `compile` and round counts (renumbering `#line`, two include
   levels, a `..` include), `Cc::detect` parse unit test, gcc-runnable `__label__` cases split out
   of the macOS-only tests, the runtime compiled without `-I` and the probe header left out by exact
   path only, an earlier constructor changing TMPDIR, the progress lines. The reviewer left 14
   working tests: `SP/rv/work/mutants-build-runtime/mut/kill-tests.diff` (written against 665495b:
   adapt, keep only what is still missing). Mutation-check each new test (`SP/mutfix.py`, add
   entries; run `python3 -u SP/mutfix.py SP/mut1 <names>` after `git archive HEAD | tar -x -C
   SP/mut1`).
2. DONE (e846261): design §10 and §2's table. Was: write design §10 in docs/FEATURES-PROBE-REDESIGN.md (the review record and every place the
   build differs from the text — see the "Built (§10)" notes already in §3.3–§3.5, §4, §6) and
   replace §2's table with the build's premise numbers: adversarial repros 24/24 and 25/25 map,
   2 567 and 1 999 unwatched (same as the prototype, all kind compile, ≤ 2 compiles); corpus 42/42,
   928 unwatched (465 + 457 sqlite functions inside two misread bodies — before the scanner fix
   349 of them per copy were missing from the facts — and 6 signal-hook parser); extensions 35/37
   (numpy popcnt ×2 do not build plainly on arm64), 136 in own files (109 nkf parser, 27 rule 3),
   650 in copied Python headers; zopfli byte-identical. Costs: compile +4 % instructions,
   +16.8 MiB, object +12.5 %; a probed run +1–2 % (zopfli) to +11 % (sqlite3.c). Re-run
   `SP/premise-fix1/run.py` with the final binary for the final numbers (it copies the binary
   given in its HARNESS line).
3. The check of the whole fix pass, workflow `wf_35a3625c-b71`: its four lenses FINISHED with 26
   findings (`SP/fc1/lenses.json`; verifiers were still running — read the journal for verdicts).
   FIX PASS 2 DONE (part 1 4feb237: tokenizer, scanner, MAP_PROBE compiler-guided-2; part 2
   the next commit: bounds checked before each compile, the placed-rounds bound only for a round
   that places, cut-short referrer lists and a retry over every unit, GNU/lld forms, gcc #line
   over all reads, killed child compilers + -fno-crash-diagnostics, objsyms ELF e_shnum = 0 and
   checked offsets and a `function` flag, hidden variants (code only, clone names, explained
   names), the signal cleanup made final (race: before 14/21 runs left a folder, after 0/27;
   `SP/race2/race3.py <harness> <scanned zopfli copy> copy|spawn <ms,…> <runs>`), tests on Linux
   (--allow-unsandboxed, -fmax-errors=0), the lld main hint, the incbin-macro test, zopfli's
   map.json regenerated; committed 617f75f, 4fd10a1). Recorded in design §10.1. Mutation check
   DONE: 37 mutants, 34 killed, 3 live and explained in §10.1 (`SP/mutfix2.py`, `SP/mutfix2b.py`,
   logs in SP). §8 premise re-run with this build: identical to fix pass 1's (79 of 79 the same;
   design §10.1). The CHECK of fix pass 2 RUNNING: workflow `wf_b40998a8-a02` (script
   `SP/fix-pass-2-check.js`, base copy `SP/fc2/base` at 4fd10a1's code, work `SP/fc2/work`;
   lenses tokenizer-scanner, build-link, variant-runner; two verifiers each). The fix-pass-1
   check FINISHED (`SP/fc1/result.json`): 25 confirmed by both verifiers, 1 refuted by both (gcc
   #line, kept for agreement with the design, §10.1).
   CHECK OF FIX PASS 2 FINISHED (`wf_b40998a8-a02`, `SP/fc2/result.json`, digest `SP/fc2/digest.md`):
   26 confirmed by both verifiers, 0 refuted; one high (two heads in GNU's layout read as one
   watched function — fix pass 2's narrower rule). FIX PASS 3 COMMITTED (4f79938; design §10.2): the scanner's `heads()` (type word before, K&R after, parameter-shaped
   arguments, keyword-function calls, nested declarators; a macro before the real name), the
   misread-body configuration walk, C23 attributes in declarators, U+180E and the one-pass #line,
   link search (no widening on a cut list, bound before every compile, no-note → None, rounds by
   progress), kill words only for outside signals, ELF entry size and untyped code symbols,
   inline-only and ruled-out namesakes, notes from every listing pass, run folders registered for
   the signal's cleanup, the map's tools' TMPDIR in its folder, older maps loaded safely, tests
   gated to macOS, compile-count seams (`last_map_most_file_compiles`, `last_map_pass_compiles`).
   The 101 bench targets' facts byte-identical; sqlite3.c unchanged; mutation 35/33.
   CHECK OF FIX PASS 3 FINISHED (`wf_6d4c61e3-bf5`, `SP/fc3/result.json`, digest
   `SP/fc3/digest.md`): 21 confirmed, 0 refuted, none high (6 medium). FIX PASS 4 COMMITTED
   (bfb2c7d, tests 7e40ef1; design §10.3): second heads ranked, every misread configuration, the
   spread case (objsyms undefined symbols), inline-only from the listing, ruled namesakes no
   longer explain (conservative), scorer folders registered, newer maps loaded; mutation 23/21
   (2 without effect). 101 bench facts identical; sqlite3.c same. The CHECK OF FIX PASS 4
   FINISHED (`wf_49dab672-ae0`, `SP/fc4/result.json`, digest `SP/fc4/digest.md`): 13 confirmed,
   0 refuted, none high; silent wrong maps remain (a `()` second head after a typedef/tag return;
   a namesake renamed by a macro explains; a K&R gnu_inline namesake escapes the inline check) and
   one regression (added units = every unit skips the every-unit search → refusal). RULE (set
   before it): the last probe review round unless it finds a silent wrong map or a high finding;
   lower findings are fixed directly or named in §6. FIX PASS 5 COMMITTED (8ccd4ad, test 793bdc4;
   design §10.4, §6): all 13 answered (fixed, or named in §6); 101 bench facts identical,
   sqlite3.c same; oracle 145 + features 80, scanner 57 pass. MUTATION CHECK DONE (807ba2b): 28
   mutants, 27 killed (7 after tests added), 1 guarded twice by design (design §10.4). The short
   CHECK OF FIX PASS 5 FINISHED (`wf_37778f58-b30`, `SP/fc5/result.json`): all 13 closed; 4 new,
   narrower (comments in heads; an empty-call macro taking the real head's place; folded first
   heads returning pointers; a namesake holding the name as a parameter or tag) — FIX PASS 6
   COMMITTED (bf6dfec; mutation 13/13 after two listing tests, 1dee223; design §10.5). The CHECK
   OF FIX PASS 6 FINISHED (`wf_d449beca-415`, `SP/fc6/result.json`): all 4 closed; 4 rarer
   spellings new, none in real code. FIX PASS 7 COMMITTED (03927a4; mutation 7/7, 264164b; design
   §10.6). DECIDED (process, §10.6): the review STOPS here — findings fell 26, 26, 21, 13, 4, 4,
   the last two rounds only rarer spellings of one class (two heads run together after a
   body-supplying macro, read without a preprocessor), none in real code; the class is named in
   §6 with "revisit with a preprocessing frontend". The BOUNDED CHECK of fix pass 7 FINISHED
   (`wf_3d782237-429`, `SP/fc7/result.json`): every fix held, tests 327/327, real code unchanged;
   the line-start rank regressed hand-made shapes → REVERTED, comments blanked in the head's own
   parameters too (6b59ed8; mutant killed). THE PROBE REVIEW IS CLOSED (design §10.6).
   Item 4 DONE (6b59ed8): full `cargo test --workspace --no-fail-fast` 1 075 passed, 2 failed
   under load (harness-tui chat_e2e `a_panic_ends_the_chats_group`, `new_chat_twice_ends_both`:
   "timed out waiting for the chat's runtime") — both pass alone (7/7). Item 5 DONE: `bench check
   --replay --suite targets/tractor` (release build, 25 min): 198 reproduce (1 conformant, 197
   drifted), 2 expected divergences, 0 problems; "bench check: OK — no regression". Item 7: the
   DECISIONS entry written (2026-10-01); merged to main and pushed. Was: fix those four,
   the lows (misread walk over all branch combinations, capped; K&R function-typed parameter
   `int cb(int)` outranking `after(cb)`; folded second head's static; non-nested signature ends at
   the call; `static Count (after)(void)` named `Count`), the missing tests (by-shape ranking,
   stray-K&R name check, closed-group misread, references narrowing, SHNDX table, K&R inline
   tests); the verifiers' accepted fixes are in the digest. Then a mutation check and a short
   check of the fix-pass-5 diff (verify findings closed; new silent wrong maps only), then items
   4–7 below.
4. Full `cargo test --workspace --no-fail-fast` on a quiet machine. Load-sensitive tests seen
   failing only under load: validate_driver (fixed: first exec), zopfli_verify u001 driver
   self-validation, harness-tui chat_e2e and `an_exited_leaders_pipes_are_read_first`,
   harness-mcp protocol drains, `a_fifo_named_like_a_c_file_is_a_skip_not_a_wait` (bound now
   100 s), features `a_c_side_that_times_out_is_a_skip_and_costs_one_run`, validate_driver
   `tce_equivalent_mutants_are_discarded` and `a_weak_driver_fails_mutation_adequacy` (all
   three pass alone; failed only with ~18 review agents running). Re-run each alone if it fails.
5. `bench check --replay` (quiet machine, ~29 min; `.bench/` and `.scorer-vendor/` are present in
   this worktree's targets/tractor). Expect 198 reproduce (1 conformant, 197 drifted), 2 expected
   divergences, 0 problems, "no regression". All 101 bench targets re-scan to byte-identical facts
   with the new scanner (checked).
6. Live chat tests: DONE this session, 4/4 inside Claude Code and 4/4 with
   `RUHARNESS_LIVE_CHAT_HOST=plain` (re-run only if the chat's protocol changes).
7. DECISIONS.md entry closing the probe review (plain words; what was found, fixed, checked), then
   merge to main and push (the person's standing rule: commit to main and push at milestones).

**Performance baselines (next track, started).** docs/PERF-DESIGN.md: the draft (0b64bba) had
its adversarial review — workflow `wf_fe7fb255-d44`, 4 lenses, 55 findings (lens results:
`SP/perf/design-review-lenses.json`), 2 verifiers each, still finishing at the compaction; so far
all confirmed but three wording points. Revision 1 (00f2e23) answers them (§7, §9), with a second
premise (`SP/perf/premise2/launch.c`: counting from the program's exec under sandbox-exec via
kqueue NOTE_EXEC works; RUSAGE_INFO_V6 P-core fields filled) and a link check (two Rust staticlibs
link into one C program on macOS: `SP/perf/twolibs`). The review finished: 46 confirmed, 2 by one
verifier, 1 refuted (`SP/perf/design-review-result.json`). The check of revision 1 FINISHED (`wf_0f90338d-b1d`; result `SP/perf/revision1-check.json`): of
the draft's findings 7 resolved, 41 in part, 1 made worse; 33 new confirmed (launcher built through
the shared temp folder and run unsandboxed; the program can setsid out of the group kill;
argv[0] cannot be set through sandbox-exec; NOTE_EXEC coalescing races the baseline; memory's
minimum-of-n calls identical programs different; P-core-normalised cycles unsound; the 5e8 floor
where Rust's start-up equals the margins; …). Premise 3 for revision 2 (`SP/perf/premise3/`): a harness-owned trampoline `perfgo` run by
sandbox-exec inside the scenario-like profile (exec of perfgo and the program only, fork denied)
signals the launcher through a pipe, waits for its go, sets RLIMIT_CPU, then execs the program with
argv[0] = its bare name — the baseline is read while perfgo waits (no race; ≈ 7.5e7), argv[0] is
`prog` on both sides from different folders, an empty program ≈ 1.1e7 instructions, a busy loop
6.13–6.16e8; the launcher and trampoline built with TMPDIR inside their own folder. REVISION 2 WRITTEN (cd1f79b, from `SP/perf/rev2-changes.md`, 118 changes). The check of revision 2 is RUNNING:
workflow `wf_89198fcc-3e6` (dispositions of the 118 changes, three fresh lenses, two verifiers per
new finding; result → save to `SP/perf/revision2-check.json`). Then revise, then build. Was: revision
2 from a condensed change list
(`SP/perf/rev2-changes.md`, written by an agent from both result files), then check it again.
Was: CHECK THE REVISION — running as
workflow `wf_0f90338d-b1d` (per-lens dispositions of every finding against rev 1, three fresh lenses
on the new mechanisms, two verifiers per new finding); then revise (revision 2), then build in the §5 steps. Spike notes, code survey and launcher prototype:
`SP/perf/`. REVISION 2's CHECK FINISHED (`wf_89198fcc-3e6`; `SP/perf/revision2-check.json`, digest
`SP/perf/rev2-digest.md`): of 118 changes 36 done, 73 in part, 2 not done, 7 wrong; 35 new
confirmed (two high: the every-run metric rule answers almost no loaded row; the shared C pair
undoes C, other, C), 1 disputed, 0 refuted. REVISION 3 written (1b27d10); its check FINISHED (`wf_d21b13f6-ea1`;
`SP/perf/revision3-check.json`, digest `SP/perf/rev3-digest.md`): 74 done, 43 in part, 1 wrong; 30
new (1 high: perfrun's own timeout read as a crash), 1 disputed (taken), 0 refuted. REVISION 4
written (f7a4905); its check FINISHED (`wf_8aea486a-f12`; `SP/perf/revision4-check.json`, digest
`SP/perf/rev4-digest.md`): 38 done, 35 in part, 2 wrong; 31 new (one high: the go-ahead pipe took
perfrun's stdout), 2 by one verifier, 1 refuted. By the rule below, REVISION 5 written (728e82e);
its check FINISHED (`wf_2b416222-f83`; `SP/perf/revision5-check.json`, digest
`SP/perf/rev5-digest.md`): 39 done, 25 in part, 6 wrong; 19 new, NONE high. So the design rounds
are over: §10's 30 build notes (f80d0f1) answer that check, and the design is READY TO BUILD (after
the probe's milestone: its fix-pass-3 check, bench check, DECISIONS, merge and push). DECIDED (process, recorded in DECISIONS at the probe's close):
revision 4's check is the last design round — what it confirms below high severity goes into the
build's revision 5 notes and the build's tests; a high finding (a mechanism that fails) gets one
more revision and check. Then build in §5's order (b, c, a, d, e, f, g) with the full process.

Known gaps, judged acceptable (§R5): Next-step rule 6 has no project-menu Re-check item (a
project-level Re-check would be refused: it needs the unit on screen); migrate turns do not
print skip messages (the verdict carries them); the pty test covers Map only (the edit flow is
tested at the app level); the O6/M10 digest-before-build orderings have no test; the probe's
reopen after a failed write can drop notes silently if the reopen fails; `SOURCE_DATE_EPOCH=0`
is not recorded in a verdict's evidence; `naked` given only on an earlier declaration fails the
probed build loudly.
