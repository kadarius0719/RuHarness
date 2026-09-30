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

1. ~~The two low items~~, ~~the checks of passes 4–6~~ (passes 5–7: bf75af8, 950eee3, b31f6ef).
   The check of pass 7 (45 findings) led to a redesign, decided by the person: **the
   compiler-guided probe**, docs/FEATURES-PROBE-REDESIGN.md revision 3 (reviewed, checked twice;
   DECISIONS 2026-09-30). Build it in steps, each committed green:
   a. the runner: a call returning a failed tool's whole stderr and how it ended; tool runs
      wait 1 ms doubling to 8 ms, built and scenario runs keep 50 ms; `timeout_secs` clamped;
      the overflow-then-sleep tests; the two flaky tests' timeouts;
   b. harness-scan: the guessing rules removed; rule 3's depth form and directive scanner;
      rule 4 (gcc); `probe_source` with the skip list, body ranges, the listing copy's end
      tokens, the store note;
   c. the runtime (shared mapping, attach byte, `environ`, pinned imports) and confine.rs
      creating the notes file before every scenario run; the strict read;
   d. featuremap.rs: listings with `-E -MD -MF -H` into the random folder, data reads,
      stringized notes, the end-token count, the same-code comparison, the plain build there;
   e. the compile-and-retry pass: placement, the search and restore, the link and its map,
      bounds, progress, the runtime-names refusal;
   f. map.json `unwatched_reasons` and `inputs.probe`, SCHEMAS.md, the CLI lines, the cockpit's
      reason; the small fixes of §5;
   g. §8: the premise re-run through `harness features map`, replacing §2's table; then the
      code review, fix passes (each checked), mutation checks; update docs/TESTING-GUIDE.md
      Part 8 (the map's output lines).
2. **The C-vs-Rust performance baselines** — spike done (DECISIONS 2026-09-30; scratchpad
   `spike-perf/`, the `perfrun` launcher premise holds under the sandbox); the design next.
3. **`bench check --replay`** (the CLI changed since step 2): needs the gitignored
   `targets/tractor/.scorer-vendor/` and `.bench/` and a quiet machine; expect `198 reproduce
   (1 conformant, 197 drifted), 2 expected divergence(s), 0 problem(s)`, `OK — no regression`.
4. **The live chat tests** (the brief changed in step 5; every earlier failure was the `claude`
   sign-in expiring): `RUHARNESS_LIVE_CHAT=1 cargo test -p harness-tui --test chat_live --
   --ignored --test-threads=1`, once inside Claude Code and once with
   `RUHARNESS_LIVE_CHAT_HOST=plain` — after the person signs in to `claude` again.
5. DECISIONS: close the features entry's "open" line; then the next track — the C-vs-Rust
   performance baselines, then the briefing's M5.

Known gaps, judged acceptable (§R5): Next-step rule 6 has no project-menu Re-check item (a
project-level Re-check would be refused: it needs the unit on screen); migrate turns do not
print skip messages (the verdict carries them); the pty test covers Map only (the edit flow is
tested at the app level); the O6/M10 digest-before-build orderings have no test; the probe's
reopen after a failed write can drop notes silently if the reopen fails; `SOURCE_DATE_EPOCH=0`
is not recorded in a verdict's evidence; `naked` given only on an earlier declaration fails the
probed build loudly.
