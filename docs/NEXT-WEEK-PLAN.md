# Plan for the week of 2026-10-07 — close the speed work, then start the project map

Written 2026-10-02 on branch `claude/rust-migration-harness-f7665f` (pushed to GitHub). Main is at
56cd090 (the probe milestone). Everything below runs on this branch until step 6 merges it.

**Order, revised 2026-10-03 (agreed with the person): prove before planning.** Finish what is
started (fix pass 2's leftovers, the mutation checks), then **run the testing guide on liblzg end
to end** — nobody has taken the harness through a real migration outside zopfli, and the rough
edges that run finds in the core loop matter more than any new design — then **one real-project
spike for the map** (the closure script from the investigation on a real multi-tool C download:
see what breaks), and only then the design review of docs/PROJECT-MAP-DESIGN.md. No new design
sections until the spike has run. The picture (§3.9) stays a one-day throwaway on zopfli, and may
turn out to add nothing over `harness map`'s text. The model-advice features (§3.10) are last and
not to be built before matching works across a whole project. Steps below renumbered to this
order: 1, 2, 3, 6 (merge: DONE 2026-10-03), then 4 (liblzg), then 7 (the spike, then review).

**Status 2026-10-07 (session 03b25fd0, Opus 5.5):** step 2 (fix pass 2) DONE — all twelve items,
merged (c92f6ec), 1 251 tests green; its check round and step 3's mutation checks RAN together as
workflow `wf_6812de33-51b` (results: docs/reviews/2026-10-07-perf-fix2-check.md when saved). Step 4
(liblzg) — Parts 0–10 DONE, the guide corrected (e29ec56); Part 11 Steps 11.1–11.2 done, 11.3–11.6
wait for a quiet machine. Step 7 — the spike DONE (lz4 and liblzg; the investigation note), the
person's answers to the design's §7 recorded; the design review is next after fix pass 3 and
Part 11. The next session runs on Fable with every handed-off agent on Opus (docs/NEXT-SESSION.md).

Rule for the week: the person stopped at 75 % of the weekly limit; the costly steps (the fix
check, mutation checks) were deferred to the reset. Check `get_usage` before each large workflow
and say roughly what it will cost. Report in plain words, no finding codes.

## 1. The fix check — re-verify fix pass 1

**Scoped check DONE 2026-10-03** (`wf_04985eae-c28`, `SP/perfrv/check1.json`): all 9 high
findings clear — both verifiers each: the fix holds, a test guards it (undone in a copy, the
test fails), no regression. Optional tidy-ups they noted, added to fix pass 2 below (items 8–13).
The **full** check of the remaining 73 stays optional (about 150 agents).


What: 82 findings were fixed by six agents and merged (fix pass 1, `SP/fix1/result.json`;
findings in full: `SP/perfrv/findings.md`, numbered 0–81 by severity). None has been checked by a
fresh pair of eyes yet.

Two sizes — choose at the start of the week:
- **Scoped (recommended first):** verify the 9 high findings (numbers 0–8) — two skeptical
  verifiers each, against the merged code: does the fix hold, does its test fail when the fix is
  undone, did it regress anything nearby. About 18 agents.
- **Full:** the same for all 82, plus two regression hunters over `git diff d8d6d11..HEAD`
  (the merge). About 170 agents — the size of the review itself. Do it only if the scoped check
  finds trouble, or the budget is comfortable mid-week.

Workflow shape: `pipeline(findings, verify×2)` as `perf-code-review` did (its script is in the
session's `workflows/scripts/`, resumable by run id). Save the result to `SP/perfrv/check1.json`.

## 2. Fix pass 2 — the seven leftovers, in this order

From docs/FEATURES-PROGRESS.md "Fix pass 1 DONE"; each with a test; one agent per group or by
hand (they are small):

1. **Security first — verify's own sandbox profiles share the LaunchServices gap** the perf
   profile closed: `render_profile`, `render_run_profile`, `render_scenario_profile` in
   `crates/harness-oracle/src/sandbox.rs` must end with `NO_STARTS_THROUGH_THE_SYSTEM` too (it
   is `pub(crate)`, ready to append). Update their golden tests. Live test: a C that calls
   `open`/`LSOpen…` under the run profile is denied. Note in DECISIONS: pre-existing, found by
   the perf review. Also fix PERF-DESIGN §3.4 / §3.12 / §6 to say the rule (the launcher agent
   left the design text untouched).
2. `perf show` with no facts says "the C changed" on every row (`crates/harness-cli/src/perf.rs`;
   the cockpit side is fixed — mirror it: no judgement without facts, say "not checked").
3. The results reader caps the lengths of `replaces`, `units`, `left_out` lists
   (`crates/harness-core/src/perf/results.rs` check_inputs) — a forged file otherwise grows
   many times on parse.
4. `perf show` and the writer's `Store::put` (`measure.rs`) must not follow a linked
   `migration/perf/units` folder (the cockpit's perfread already refuses it).
5. The dialogs' estimate cannot add the launcher build: add a cheap public "is the launcher
   cache current" check in `harness-oracle` launcher.rs; `estimate_building_launcher` exists in
   core-words' work and waits for it.
6. Optional: `perf run --as-it-stands-only` refuses before building when fewer than two units
   are measurable.
7. `harness-cli/tests/perf.rs` two_units_end_to_end: add `assert_eq!(crash.len(), 1)` (and for
   "time") now that the C-fails-in-step-1 rule is merged.
8. measure.rs keeps its own copy of the compilers' first-line code: call `tools::first_line`.
9. Tests: assert the stored `short` on the C alone (`Some(false)`) and on a full measured row
   in the measure end-to-end test (swapping the Short/Full arms must fail a test); the
   second-run case (a too-short workload whose input is then removed: exit 0, `last_try`
   input-unusable, the next workload still measured) in measure.rs or tests/perf.rs.
10. Launcher tests: `pid_of` must pick only a process descended from the test (two worktrees
    running the tests at once can read each other's perfrun); `cpu_time` must tolerate an empty
    `ps` read; `every_way_to_start_a_process_is_killed` flaked once under a full parallel run.
11. perf show: skip the two compiler checks when no row is stored (harmless, saves two runs).
12. SCHEMAS: "the computer and the compilers are checked only by perf show (when the launcher
    cache is current)" — the parenthesis is the computer's only.
13. PERF-DESIGN §3.3 step 3: the cancel check comes before the lock is retaken (as the code
    does); note 22 quotes §1 as "verified" where §1 says "Re-checked".

Then `cargo fmt`, clippy clean, full tests of the five perf crates, commit per group.

## 3. Mutation checks

As the probe did (`SP/mutfix.py` pattern: switch each named rule off in turn in a copy of the
tree; a test must fail). Rules to mutate — one list to write first, then run:
- the floor: `under_both` back to `under_either` (words and measure);
- check_row: the set-up `last_try` acceptance; the new list caps;
- share_rule: too-few only when raw is missing;
- rounding: `away_from`'s tolerance; the margin on both ends; the close call's far end;
- the replace rule's three branches;
- perfrun: the ready-watch delete (rebuild perfrun without it; the "stays idle" test must fail);
- the perf profile: drop `NO_STARTS_THROUGH_THE_SYSTEM`; drop the fork SIGKILL;
- currency: each reason token;
- the cockpit: `change_words` hand-edit branch; the input cache key's change time; the linked
  units folder refusal.
Record killed / survived / equivalent in docs/FEATURES-PROGRESS.md.

## 4. Real run on liblzg (needs the person's OK to download) — BEFORE the project map

`git clone https://gitlab.com/mbitsnbites/liblzg.git ~/code/liblzg-upstream`, then Parts 1–6 of
docs/TESTING-GUIDE.md (two translations through the chat), then Part 11 as written. Fix the guide
where reality differs. Worth it before the person uses the guide; skip if the budget is tight.

## 5. DECISIONS entry — draft (paste and adjust at the close)

```
## 2026-10-0X — C-vs-Rust performance baselines: built, reviewed, fixed; closed

**Built** from docs/PERF-DESIGN.md revision 5 in seven steps, each committed green (b, c, a, d,
e, f1–f4, g): the statistics and words (harness-core perf), the launcher and trampoline
(perfrun, perfgo; a private per-user cache; the perf sandbox profile), the measurement
(harness-oracle perf), `harness perf init|save|run|show`, the cockpit's Speed group, View, acts,
the behaves-differently fact and Compare the outputs, harness-mcp's Speed fact, and the docs
(SCHEMAS, the tutorial, the testing guide's Part 11). Real result on zopfli: u001-katajainen
about as fast as the C (within 2 %).

**Reviewed** at d8d6d11: six reviewers (statistics and words; files and currency; launcher and
sandbox; the measurement; CLI, cockpit and MCP; tests against the design), 84 findings, 82
confirmed by two verifiers each, 2 split, 0 refuted. **Fix pass 1** handled all 82 in six
worktrees and merged; checked by <the fix check's result>. **Fix pass 2**: the seven leftovers,
<result>. **Mutation checks**: <killed/survived/equivalent>.

**What was wrong, in plain words, and is fixed:** the launcher kept a core busy while the program
ran (it skewed the load it then reported); a run over one leg of the floor but not the other was
called short, so common workloads could never read "about as fast"; a missing input after a
too-short row stopped the whole run; `perf show` ran the project's compilers outside the
sandbox; the hand-edit advice told the person to measure the unchanged crate; the perf sandbox
(and, pre-existing, verify's) let a program have macOS start another program for it; several
rounding and wording edge cases; many tests the design lists did not exist.

**Decided: perf runs on macOS only for now.** `perf run` refuses by name elsewhere ("perf runs
on macOS only for now — the Linux launcher is not built yet"). Reason: no Linux machine to build
or test the launcher's Linux half (perf_event_open, epoll, PR_SET_PDEATHSIG) on, and CI's Linux
job has been red since 2026-09-17 (a newer clippy lint). The design's Linux mechanisms (§3.3) are
written and marked unchecked; building them is its own item once a Linux machine is available.

**Decided: stored rows read out of date after this close** — PERF_LAUNCHER is perf-launcher-2
and PERF_RECIPE perf-recipe-2 — because rows measured by the spinning launcher, or judged short
by either leg, must not read current. Measuring again brings a row back.

**Recorded, not decided:** the project map (docs/PROJECT-MAP-ROADMAP.md, -INVESTIGATION.md,
-DESIGN.md draft 0, with the interactive picture §3.9) — to design properly next, weighed
against M5.
```

## 6. Merge to main and push — DONE 2026-10-03 (9af3fcf); repeat at the next milestone

Checklist: full `cargo test --workspace` green; clippy clean; `bench check --replay` (expect 198
reproduce, 2 expected divergences, 0 problems); docs/FEATURES-PROGRESS.md "NOW" updated; the
DECISIONS entry in; then on `main`: `git merge --no-ff claude/rust-migration-harness-f7665f`,
`git push`. Then update memory `ruharness-project-state`.

## 7. Then: the project map — spike first, review second

A §15-style spike BEFORE any design review: run the investigation's closure script
(`SP/pmap/closure.py`, 40 lines; rewrite it if the scratchpad is gone) on one or two real
downloads (the person must OK downloads — candidates: liblzg if already here, and one
multi-tool C project such as zlib (one library, `example.c`, `minigzip.c`) or lz4 (library +
`programs/`)), then review rounds of docs/PROJECT-MAP-DESIGN.md draft 0 (its §7 has five
questions for the person — ask them first), then a closed revision. Weigh against the briefing's
M5 before building.
