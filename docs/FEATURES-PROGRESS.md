# Features track — progress (resume here after a context reset)

Branch: `claude/rust-migration-harness-ee89d6` in worktree `.claude/worktrees/rust-migration-harness-5d0aef`
(not yet merged to main / pushed — push at the end of the track, per the solo-dev rule).

Design: docs/FEATURES-DESIGN.md (four revisions: §R 84, §R2 58, §R3 37, §R4 11 findings; the
rule text governs). Spike + user decision (scenarios + verify): DECISIONS.md 2026-09-29.

## Done (each committed green: fmt, clippy -D warnings, cargo test --workspace)

| step | commit | what |
|---|---|---|
| 1 core | c25bff5, 5e4d1b9 | features.toml loader/snapshot/digests, VerdictInputs + AttemptRecord fields, Coverage marker in UnitReport, SkipReason, starter |
| 2 oracle | 52a0802 | run_scenario (one path, cwd = temp dir, `$`-escaped rewrite, scenario profile, group kill, ExecFailed), feature step after boundary, verify_with, migrate/replay/promote snapshots, CLI messages |
| bench | — | `bench check --replay` after step 2: 198 reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems, OK no regression, hidden 16/17 public 70/77 |
| 3 map | 1cd6b91 | probe_source (harness-scan), fnprobe runtime, map_features, map.json loader, CLI features init/save/map |
| 4a | 0a9f19b | featmap model, Features tree group, Features + feature Views |
| 4b | e29430c | verdict marker, unit/function feature lines, feature chip, overlay notes, summary line, Next-step rules 5-6, Map act |
| 4c | 26362d2 | Edit flow (draft, editor dialog, validation, `features save` on stdin, kept draft), Help, pty test Map end to end |
| 5 | 9e62e29 | chat brief (feature checks apart, none ≠ passing, scenario ids, menu items), harness-mcp closed `features` field |
| 7 | c5f6891 | SCHEMAS, TUTORIAL, README |
| 6 | a38c2aa | dogfood: zopfli features.toml (7 features, 8 scenarios) + map.json, u001 re-verified green with 8 feature checks; counts in the shown form (`$$` undone, paths as tokens — found by the dogfood: 206 for 205); `same_result` for run comparisons; zopfli-loading tests adjusted, a dogfood view test, MCP `features: current` |

Verified on a copy of zopfli: `features map` (8 scenarios, 10 s) separates gzip/zlib/deflate/-v/--i1 as
the spike found; `verify u001` runs 8 feature checks, green; `state status` shows features=current.

## Next (in order)
| 8a review | — | 4 lenses → 35 findings, each verified independently: 33 confirmed, 2 refuted (M4, O8) — scratchpad code-review/<lens>/{findings,verdicts}.md; FEATURES-DESIGN §R5 |
| 8b fix pass | (this) | every confirmed finding fixed or its residual named (§R5 table); 940 tests pass |

Live chat tests after step 5: every failure was the `claude` sign-in expiring ("OAuth session
expired and could not be refreshed" — the chat said so in words); re-run both after the person
signs in again (the fix pass changes nothing in the chat protocol, but the brief changed).

5. (done) The chat brief (§9: feature checks reported separately; no feature checks ≠ passing; name
   scenarios by id; menu items) + harness-mcp unit report's closed `features` field; then the
   live chat tests (`RUHARNESS_LIVE_CHAT=1 cargo test -p harness-tui --test chat_live -- --ignored
   --test-threads=1`, inside Claude Code and with `RUHARNESS_LIVE_CHAT_HOST=plain`).
6. (done) Dogfood: `targets/zopfli/migration/features/features.toml` (gzip text/rand, zlib, deflate,
   verbose, quick --i1, help -h, no-file) + map.json; re-verify u001; adjust the zopfli-loading
   tests (zopfli_verify exact check list — run it on a copy without the file; others).
7. (done) SCHEMAS.md (ruharness-features v1, ruharness-features-map v1, the new fields/events/CLI,
   trust boundaries), TUTORIAL "Features" section, README line.
8. (review + fix pass done) check the fix pass →
   mutation checks of the named rules (§12 end) → bench check --replay again (CLI changed) →
   DECISIONS handoff → merge to main, push. Then memory update.

Known gaps to review: Next-step rule 6 has no project-menu Re-check item (text only, act None);
migrate turns do not print skip messages (verdict carries them); the pty test covers Map only.
