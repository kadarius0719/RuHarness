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

## Open (in order) — the next session starts here

**Session 2026-09-30 (17c366c0…), in progress.** The pause's code review (`wf_833f5d1a-f42`)
had lost two of its four lenses and nearly all verifiers to an expired sign-in. Re-run at
665495b as Workflow `wf_d2c3d9f7-615` (this session's subagents/workflows): the two lost lenses,
a token-comparison lens, two mutation-sweep lenses, two verifiers per finding (old and new), and
the §8 premise re-run. Review files: scratchpad `rv/` (existing.json = the first review's 22
findings; runner-lens.json; premise-*.json).

Fix pass 1, committed on branch claude/rust-migration-harness-f7665f (not yet on main):
3ed89be, 1a0efb0, 31610cf, 6a03a6d, 9bc83d3, 7615200, 667654b — a whole-text tokenizer for the
copy check (raw strings, digit separators), the bounds as designed, link rules (a)/(b) and the
link search, the runtime-names check from the objects' symbol tables (objsyms.rs; no link map),
the hidden #if variant, scratch-folder cleanup on a signal, 0700 and one text in memory, the
scanner's swallowed definitions (sqlite3.c: 349 compiled functions were missing from the facts),
safe text everywhere, the ledger flake's cause, driver validation's first-exec timing. Mutation
check of the fixes: 40 mutants, 39 killed, 1 equivalent (encoding prefixes).

§8 premise re-run (with the fixed harness, 9bc83d3): adversarial repros 24/24 and 25/25 map,
2 567 and 1 999 unwatched (the prototype's numbers, the same functions); corpus 42/42 map, 928
unwatched (465 + 457 sqlite functions inside two misread bodies, 6 signal-hook parser); extensions
35/37 (numpy popcnt ×2 do not build plainly on arm64), 136 in their own files; zopfli's map
byte-identical. Costs: compile +4 % instructions, +16.8 MiB, object +12.5 %; a probed run +1–2 %
(zopfli) to +11 % (sqlite3.c workload).

Next: collect the rest of `wf_d2c3d9f7-615` (two mutation lenses, ~60 verifiers), fix what they
confirm, record §10 in the redesign doc and replace §2's table, then a check of the whole fix
pass (independent agents), then `bench check --replay`, the live chat tests, the handoff and
the push to main. Running beside it: the perf design's review (`wf_fe7fb255-d44`, docs/PERF-DESIGN.md
draft 0b64bba).

Known gaps, judged acceptable (§R5): Next-step rule 6 has no project-menu Re-check item (a
project-level Re-check would be refused: it needs the unit on screen); migrate turns do not
print skip messages (the verdict carries them); the pty test covers Map only (the edit flow is
tested at the app level); the O6/M10 digest-before-build orderings have no test; the probe's
reopen after a failed write can drop notes silently if the reopen fails; `SOURCE_DATE_EPOCH=0`
is not recorded in a verdict's evidence; `naked` given only on an earlier declaration fails the
probed build loudly.
