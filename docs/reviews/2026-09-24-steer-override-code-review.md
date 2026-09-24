# Code review — steer attempts, human attempts, provenance in core (2026-09-24)

Adversarial review of docs/TUI-DESIGN.md §8 steps 1–2 (uncommitted tree on top of b8c87a2):
four lenses (replay & identity, the human attempt & its trust boundary, the benchmark &
provenance, the CLI contract & consumers); every finding attacked by an independent verifier
against the code. **16 confirmed, 0 refuted. All RESOLVED** in the fix pass (2026-09-24;
the resolutions are in docs/TUI-DESIGN.md §R2 — the one design decision, finding BENCH-M2,
went to "a steered crate is a bench PROBLEM"). Each entry: the verified claim, the
verifier's corrected fix, and the regression test that must fail without the fix. Line numbers
refer to the tree as committed with this file.

| id | severity | finding |
|---|---|---|
| BENCH-H1 | high | A steer seeded from a human attempt launders the hand edit into Pipeline provenance |
| BENCH-M2 | high | A steered crate is indistinguishable from unassisted pipeline output in scores.json; migrate_turns counts only the steer's turns |
| CLI-H1 | high | A steer attempt seeded from a human attempt passes a hand edit off as pipeline provenance |
| HUMAN-H1 | high | A steer seeded from a human attempt turns hand-written code into Pipeline provenance |
| RI-1 | high | A steer attempt seeded from a human attempt gets pipeline provenance, so a hand edit is scored as pipeline output |
| BENCH-H2 | medium | A human attempt can become the case's scored 'unverified candidate' and feed migrate_outcome |
| CLI-M1 | medium | Notes starting with '-' are rejected by clap (or print help and exit 0), and the resume hint's `--steer '<note>'` form breaks the same way |
| CLI-M2 | medium | An interrupted `harness override` leaves an in-progress human attempt that blocks recording the same edit, with no way out short of deleting it by hand |
| HUMAN-M1 | medium | Ctrl-C or kill during override's judge leaves an in-progress human attempt that blocks the same edit permanently |
| HUMAN-M2 | medium | bench scores a red human attempt's candidate as the pipeline's candidate_attempt |
| HUMAN-M3 | medium | A deny-scan red human attempt records and reports no evidence, not even what was submitted |
| RI-2 | medium | The steer fields (seeded_from, steer_note) and the seed's stored verdict are not integrity-bound to the recorded turn 1 |
| RI-3 | medium | bench scores a human red attempt as the pipeline's unverified candidate |
| RI-4 | medium | An interrupted `harness override` leaves an in-progress human attempt that blocks recording the same edit for good |
| BENCH-M1 | low | The §7-required test of bench's human-provenance PROBLEM (and the replay skip) is missing |
| CLI-M3 | low | The TUI hand-edit flow (§4 `e`) is refused whenever the editor leaves a backup file in src/ |

## BENCH-H1 — A steer seeded from a human attempt launders the hand edit into Pipeline provenance

Severity: high (claimed high).

**Claim.** provenance() only collapses a steer attempt into its seed when the seed is itself one of the digest matches. A steer attempt whose seed is a human attempt, but whose candidate differs from the seed, is a lone model match, so it scores as Pipeline. That breaks the §5.2/§0 invariant 'the benchmark never counts a hand edit as the pipeline's', which SCHEMAS.md now also states ('the benchmark scores pipeline output only').

**Evidence.** crates/harness-core/src/attempts.rs:308-322: `seeded_by_a_match` looks only inside `matches`, and a record counts as human only when its own `provider_kind == HUMAN_KIND`. crates/harness-llm/src/migrate.rs:517-527: first_turn() checks the seed's stage, in-progress state, binding, candidate digest and verdict, but not its provider_kind, so a human attempt is an accepted seed. TUI-DESIGN §4 `m` Modify also allows any finished, bound attempt with a candidate/. Nothing in bench.rs reads `seeded_from`. Scenario: (1) `harness override U edit/` records human attempt a-h, green, digest X. (2) `harness migrate U --no-promote --from a-h --steer 'add the comment // reviewed at the top of logic.rs; change nothing else'`: the model gets a-h's files as [CURRENT RUST], echoes them with the comment, and the result is a-s, green, digest Y, seeded_from a-h. (3) `harness promote U a-s`. (4) `bench score --write`: matches = {a-s}; its seed a-h is not a match, so model = [a-s] → Pipeline(a-s). There is no PROBLEM; the case is strict-pass with migrate_turns 1. Two-hop variant: a-h (X) → steer s1 (Y) → steer s2 from s1 that reverts to exactly X. matches for X = {a-h, s2}; s2's seed s1 is not a match, so s2 is not collapsed → Pipeline(s2), even though the crate is byte-for-byte the human's.

**Verifier.** Confirmed against the code.

1. A human attempt is accepted as a steer seed. In crates/harness-llm/src/migrate.rs:515-557, first_turn() checks the seed's stage (517), in-progress state (520), binding (525), candidate_digest and candidate/ (533-541) and verdict (548-557). It never checks provider_kind. record_human_attempt (migrate.rs:615-736) runs the one judge, which writes candidate/, sets candidate_digest and writes attempt-verdict.json, so a green human attempt passes every seed check. The design allows this too: TUI-DESIGN §4 enables `m` Modify on any finished, bound attempt that has a candidate/. The implementer clearly expected human seeds, since the test at attempts.rs:707-712 ("A model steer that reproduced a human seed: the human is the source") covers them.

2. provenance() (attempts.rs:297-332) collapses a match only when its direct seed is also a match (`seeded_by_a_match`, 308-312). It puts a record in the human bucket only when that record's own provider_kind is human (319). Trace of the scenario, where the crate digest is y:
   - records = [a-h {human, green, x}, a-s {external, green, y, seeded_from a-h}]
   - matches = [a-s]; a-h is not a match, so a-s is not collapsed
   - model = [a-s], so the result is Pipeline(a-s)
   In bench.rs:611-630, promoted is Some(a-s), promoted_count is 1, migrate_turns is 1, and no PROBLEM is raised, so `bench score --write` records the crate as pipeline output. Nothing in bench.rs or promote.rs reads seeded_from or the seed's kind.

3. The two-hop case is also confirmed. For a-h (X) → s1 (Y) → s2 (X) with crate X, matches = {a-h, s2}. s2's seed s1 is not a match, so model = [s2] and human = [a-h]. The (1, _) arm then returns Pipeline(s2). The existing rule that a model match outranks a human one (test at 702-706) actually makes this worse.

This breaks the invariant stated in TUI-DESIGN §0 and §5.2 ("never counted as the pipeline's"), the §R LEDGER-H3/SCOPE-1 resolution, and the new SCHEMAS.md text ("the benchmark scores pipeline output only"). The code follows the letter of the §2 rule, which only collapses a steer that reproduced its seed. But the design gives that rule to serve the stated invariant, and here the invariant fails.

The path is not only adversarial. In the planned TUI a reviewer would naturally make a hand edit, then Modify it ("also rename x"), then Accept. The rail would then show `*` (pipeline), and scores.json would silently get a strict-pass pipeline result for code that is mostly human-written. That is exactly the high finding LEDGER-H3 was meant to close, so high is the honest severity.

One point on the proposed fix: making a steer collapse into its nearest ancestor that is a match goes beyond this finding. It would also turn today's model-only Ambiguous result for a-1 X → s1 Y → s2 X into Pipeline, which the design did not decide. Lineage taint alone already gives Human for both the one-hop and two-hop cases.

**Fix (corrected).** Minimal change, in harness-core only, so bench and the TUI share the rule. Add `fn human_origin<'a>(records: &'a [AttemptRecord], r: &'a AttemptRecord) -> Option<&'a AttemptRecord>`. It follows `seeded_from` through all of `records`, not just `matches`, and stops after records.len() steps as a cycle guard. It returns the first record whose provider_kind == HUMAN_KIND; the starting record counts as its own origin when it is human.

In provenance() (attempts.rs:315-324), put a match in the human bucket when `human_origin(records, r).is_some()`, not only when `r.provider_kind == HUMAN_KIND`. Keep the existing direct-seed collapse unchanged; do not add the collapse into the nearest matching ancestor.

Keep Provenance::Human carrying the matched record, whose candidate is the crate, so the TUI diff stays correct. In bench.rs, at the Human arm (around line 678), name the hand edit via `human_origin`, for example: "the verified crate was promoted from human (override) attempt {origin}, or from an attempt seeded from it — not pipeline provenance".

Optional hardening: if a seeded_from id is not found in `records`, the lineage cannot be proved, so do not count that match as Pipeline.

Regression test, in attempts.rs provenance_is_the_one_r5_rule:
- human = prov_rec("a-h", HUMAN_KIND, "green", "blake3:x")
- s = AttemptRecord{seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:y")}
- assert!(p(&[human.clone(), s.clone()], Some("blake3:y")).starts_with("human:")). Today this returns "pipeline:a-s".
- Two-hop case: s1 = seeded from a-h with digest y; s2 = seeded from s1 with digest x. assert!(p(&[human, s1, s2], Some("blake3:x")).starts_with("human:")). Today this returns "pipeline:s2".
- Add a bench unit test showing the Human PROBLEM fires for the one-hop synthetic case.

**Regression test.** harness-core attempts test: human = prov_rec("a-h", HUMAN_KIND, "green", "blake3:x"); steer = AttemptRecord{seeded_from: Some("a-h"), ..prov_rec("a-s", "external", "green", "blake3:y")}; assert p(&[human, steer], Some("blake3:y")) starts with "human:". Today it returns "pipeline:a-s". Add the two-hop case (a-h X → s1 Y → s2 X, crate X) expecting human.

## BENCH-M2 — A steered crate is indistinguishable from unassisted pipeline output in scores.json; migrate_turns counts only the steer's turns

Severity: high (claimed medium).

**Claim.** provenance() treats every non-human attempt as Pipeline, steer attempts included. Bench publishes the steer's own turn count as migrate_turns and never discloses `seeded_from` or `steer_note`. A reviewer's note (up to 2000 bytes) can carry the fix itself, and the case still scores as a first-shot pipeline success. This is a gap in the design: §5.2 extended R-5 by provider kind only.

**Evidence.** crates/harness-core/src/attempts.rs:328: `(1, _) => Provenance::Pipeline(model[0])` covers seeded attempts. crates/harness-cli/src/bench.rs:631: `migrate_turns = promoted.map(|r| r.turns.len())`. CasePipeline (harness-core/src/bench.rs:823-847) has no steer field. Scenario: the seed a-1 took 4 turns and went red. `harness migrate U --from a-1 --steer '<corrected loop body pasted>'` goes green in 1 turn and is promoted. scores.json then reports strict-pass with migrate_turns 1, the same as an unassisted translation that passed first time.

**Verifier.** CONFIRMED in the code, and one variant is worse than the reviewer claimed.

1. Every factual point checks out.
- crates/harness-core/src/attempts.rs:308-328: a green steer attempt whose candidate differs from its seed is pushed into `model`, which gives `Provenance::Pipeline(steer)`. Only a steer that reproduced its seed's candidate (`seeded_by_a_match`, line 316) is collapsed.
- crates/harness-cli/src/bench.rs:631: `migrate_turns = promoted.map(|r| r.turns.len())`, so only the steer's own turns are counted.
- CasePipeline (harness-core/src/bench.rs, around lines 823-847) has no steer or seed field.
- A grep shows nothing in harness-cli reads `seeded_from` or `steer_note`. The only hit is the test constructor at bench.rs:1442.
- The steer note may be up to 2000 bytes (migrate.rs:211 `MAX_STEER_NOTE_BYTES`) and reaches the model verbatim as `[GUIDANCE]`.

2. The general case is a gap in the design, not a deviation from it. §2 and §5.2 of TUI-DESIGN deliberately treat a steer attempt as a model attempt. On its own this would be medium. It still conflicts with M4-DESIGN §2, which says the layers are "each unable to see the next" and the TRACTOR vectors are "never in any prompt". A steer note is free text written by a human who can read scores.json `vectors[]`. That human can steer a `blind-spot` case to `strict-pass`, and the result scores as unassisted pipeline output.

3. The reason I raise this to high: a hand edit can be laundered through one steer, which breaks the explicit §5.2 invariant "The benchmark never counts a hand edit as the pipeline's".
- `load_steer_seed` (migrate.rs:513-583) refuses unclean, unfinished, unbound, candidate-less, modified or verdict-less seeds. It never refuses a seed whose `provider_kind` is human. The §5.1 refusal list does not exclude one either.
- Scenario: `harness override U dir` records human attempt a-h. It can be red, or green but not promoted. Then `harness migrate U --from a-h --steer 'fix the off-by-one'` runs. The model gets the human's Rust as `[CURRENT RUST]` and returns it with a small change, which goes green. `harness promote` follows.
- provenance(): a-h is not in `matches` because its digest or outcome differs. So `seeded_by_a_match` is false, the steer attempt is counted as a model match, and the result is `Pipeline`. bench raises no PROBLEM, `--write` accepts it, and the case scores strict-pass with migrate_turns 1.
- The core test at attempts.rs:706-713 ("A model steer that reproduced a human seed: the human is the source") shows the intended rule is lineage. It covers only the byte-for-byte reproduction case; any one-byte change escapes it.
- This is the same failure the design review rated high as LEDGER-H3/SCOPE-1 ("a hand edit would score strict-pass"). The resolution was left incomplete.

No committed ledger holds a steer attempt yet, so no current scores.json is wrong. But the CLI path ships as it stands, and the TUI's Modify and hand-edit acts are built around exactly this flow.

**Fix (corrected).** Minimal change, in three parts.

(1) In harness-core `attempts::provenance` (attempts.rs, around lines 312-322), resolve lineage before sorting a match into `model`. Follow `seeded_from` through `records` by id, for at most `records.len()` steps so a cycle cannot loop. If any ancestor has `provider_kind == HUMAN_KIND`, push the match into `human` instead, as `Human` of that human ancestor, so the bench PROBLEM names the override attempt. Update the doc comment, the SCHEMAS.md provenance paragraph and the TUI-DESIGN §5.2 sentence to say "a crate whose lineage includes a human attempt is Human".

(2) In harness-core bench.rs `CasePipeline`, add `#[serde(default, skip_serializing_if = "String::is_empty")] pub migrate_seeded_from: String`. This is additive, and existing scores.json files stay byte-identical. At harness-cli bench.rs:631, set it to `promoted.and_then(|r| r.seeded_from.clone()).unwrap_or_default()`, and print a `bench: <case>: promoted attempt <id> is a steer attempt seeded from <seed> (human guidance)` line.

(3) Record in DECISIONS.md whether a model-lineage steered crate counts toward strict_pass (disclosed) or is a PROBLEM under `--write`, citing M4-DESIGN §2 (vectors never in any prompt; a note is unconstrained human text).

Regression tests:
- In the core `provenance_is_the_one_r5_rule` test, call `p(&[prov_rec("a-h", HUMAN_KIND, "red", "blake3:h"), AttemptRecord { seeded_from: Some("a-h".into()), ..prov_rec("a-7", "external", "green", "blake3:c") }], Some("blake3:c"))`. Assert it returns "human:a-h". Today it returns "pipeline:a-7".
- In bench, run a synthetic case whose Pipeline record has `seeded_from: Some("a-1")`. Assert `pipeline.migrate_seeded_from == "a-1"` and that the disclosure line is printed. Also assert that `serde_json::to_string(&CasePipeline::default())` has no `migrate_seeded_from` key.

**Regression test.** Synthetic score test: a Pipeline provenance attempt with seeded_from Some("a-1") → CasePipeline.migrate_seeded_from == "a-1", and the disclosure is printed. It fails today because the field does not exist and nothing reads seeded_from.

## CLI-H1 — A steer attempt seeded from a human attempt passes a hand edit off as pipeline provenance

Severity: high (claimed high).

**Claim.** `--from` accepts a `human` (override) attempt as a seed. `provenance()` only folds a steer attempt into its seed when the candidate digests are byte-equal. So a steer over a hand edit that changes even one byte is scored as `Provenance::Pipeline`, and the benchmark ends up counting the hand edit as pipeline output. That breaks §5.2 ('The benchmark never counts a hand edit as the pipeline's') and LEDGER-H3/SCOPE-1.

**Evidence.** crates/harness-llm/src/migrate.rs:514-535 (first_turn): the seed checks cover stage, in-progress, binding, candidate, digest and verdict, but never `seed.provider_kind == HUMAN_KIND`. crates/harness-core/src/attempts.rs:308-328: `seeded_by_a_match` skips a steer attempt only if its seed is itself in `matches`, meaning the same digest. A steer with a different digest lands in `model`, and `(1, _) => Provenance::Pipeline`. The core test at attempts.rs (steer_on_human) covers only the byte-identical case. TUI-DESIGN §4 `m` Modify is enabled on any finished, bound attempt that has a candidate/, human attempts included. Scenario: `harness override u /tmp/edit` records green H (a hand fix). `harness migrate u --no-promote --from H --steer 'keep it as is; rename tmp to acc'` gets a model reply that is H's code with one rename, so S is green with digest != H. `harness promote u S`, then `bench score --write`: provenance = Pipeline(S), no PROBLEM, and scores.json records a strict pass for the pipeline on human-written code.

**Verifier.** Confirmed against the code.

1. A human attempt passes every seed check. `first_turn` (crates/harness-llm/src/migrate.rs:515-557) checks the stage, in-progress, the R-5 binding, candidate presence and digest, and the stored verdict, but never `seed.provider_kind`. `record_human_attempt` (migrate.rs:615-735) produces exactly such a seed: `stage: None`, a finished green/red outcome, and a record bound to the current unit_source and driver. The one judge sets `candidate_digest`, writes `candidate/` and writes `attempt-verdict.json`. `run_migration` passes `steer: Some(..)` through (main.rs:1133). TUI-DESIGN §4 enables `m` Modify on any finished, bound attempt that has a `candidate/`, and §5.1's refusal list has no human exclusion. So selecting a human attempt and pressing `m` is a supported one-key path.

2. `provenance` (crates/harness-core/src/attempts.rs:297-332) builds `matches` from green, bound attempts whose digest equals the crate's. `seeded_by_a_match` (308-312) drops a steer attempt only when its seed is itself in `matches`, meaning the digest is byte-equal. Scenario: human attempt H (digest c1) and steer attempt S seeded from H (model provider, green, digest c2); S is promoted, so the crate digest is c2. Then `matches = [S]`, S's seed a-h is not in `matches`, and S has a model provider_kind, so `model = [S]` and `(1, _) => Provenance::Pipeline(S)`.

3. bench.rs (the `score_one` diff) maps `Pipeline(r)` to `promoted = Some(r)` with no problem pushed. The human PROBLEM fires only for `Provenance::Human`, and nothing in bench.rs, promote.rs or SCHEMAS.md looks at `seeded_from`. So `bench score --write` records a strict pipeline pass for code a human wrote, and `migrate_turns` counts only S's turns.

4. The existing test (attempts.rs:707-712, `steer_on_human`) covers only the byte-identical case. For a green seed, the steer prompt says "the guidance below asks for a change anyway", so the usual outcome is a different digest. The rule therefore catches the rare case and misses the common one. A crate identical to the hand edit is labelled Human, while one that differs from it by a single rename scores as the pipeline's.

This is a design gap as much as an implementation gap. The implementation follows §2/§5.2's rule to the letter ("collapsing a steer attempt that reproduced its seed's candidate"). But it breaks the invariant §5.2 states: "The benchmark never counts a hand edit as the pipeline's". That invariant is the resolution of the high-rated LEDGER-H3/SCOPE-1, so this silently corrupts benchmark evidence (scores.json) and belongs at high. The same hole applies to `--retry` samples of such a steer (`.rN` records carry `seeded_from`) and to chains, such as a steer over a steer over H. The problem is limited to the provenance and benchmark side: replay of these attempts is unaffected.

**Fix (corrected).** Fix it in the one R-5 implementation, so every surface (bench, the TUI `*`/`*h` glyph, the `d` target) inherits the fix. Keep the steer workflow over a hand edit; only its credit changes.

In crates/harness-core/src/attempts.rs `provenance`, add a helper that walks `seeded_from` through ALL of `records`, not just `matches`. Bound the walk with a visited set, since a hand-edited ledger could contain a cycle. The helper returns the first ancestor whose `provider_kind == HUMAN_KIND`. In the classification loop, a match with a human ancestor goes in the `human` bucket as that ancestor, not in `model`. Deduplicate `human` by id before sorting:

    fn human_ancestor<'a>(r: &AttemptRecord, records: &'a [AttemptRecord]) -> Option<&'a AttemptRecord> {
        let mut seen = std::collections::BTreeSet::new();
        let mut cur = r.seeded_from.as_deref();
        while let Some(id) = cur {
            if !seen.insert(id) { return None; }
            let rec = records.iter().find(|x| x.id == id)?;
            if rec.provider_kind == HUMAN_KIND { return Some(rec); }
            cur = rec.seeded_from.as_deref();
        }
        None
    }
    ...
    if r.provider_kind == HUMAN_KIND { human.push(r) }
    else if let Some(h) = human_ancestor(r, records) { human.push(h) }
    else { model.push(r) }
    human.sort_by(..); human.dedup_by(|a, b| a.id == b.id);

With this change, a model attempt with the same digest still outranks the human-derived one, which matches the existing "model outranks human" case. bench's existing message then names the human attempt.

Docs: extend the provenance sentence in SCHEMAS.md (~line 882) and TUI-DESIGN §2/§5.2: "a steer attempt descended (via `seeded_from`) from a human attempt is human provenance".

Optionally, first_turn could refuse a human seed instead. That is not required, and it would remove a useful workflow.

Regression test, in the harness-core `provenance_is_the_one_r5_rule`:

    let h = prov_rec("a-h", HUMAN_KIND, "green", "blake3:c1");
    let s = AttemptRecord { seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:c2") };
    assert_eq!(p(&[h.clone(), s.clone()], Some("blake3:c2")), "human:a-h");
    let s2 = AttemptRecord { seeded_from: Some("a-s".into()), ..prov_rec("a-t", "external", "green", "blake3:c3") };
    assert_eq!(p(&[h, s, s2], Some("blake3:c3")), "human:a-h");

Both assertions currently return "pipeline:a-s" and "pipeline:a-t".

**Regression test.** harness-core attempts tests: H = prov_rec("a-h", HUMAN_KIND, "green", "blake3:c1"); S = AttemptRecord{ seeded_from: Some("a-h"), ..prov_rec("a-s", "external", "green", "blake3:c2") }; assert_eq!(p(&[H, S], Some("blake3:c2")), "human:a-h"). This currently returns "pipeline:a-s". Optionally add a migrate.rs test: run_steer with `from` set to a record_human_attempt id is refused before anything is sent.

## HUMAN-H1 — A steer seeded from a human attempt turns hand-written code into Pipeline provenance

Severity: high (claimed high).

**Claim.** provenance() only folds a steer into its seed when the steer reproduced the seed's digest exactly. first_turn() accepts a human attempt as --from. So a model steer that changes even one byte of a human edit is classified Pipeline(steer), scores strict-pass, and bench raises no PROBLEM. That breaks §5.2's rule that the benchmark never counts a hand edit as the pipeline's.

**Evidence.** crates/harness-core/src/attempts.rs:308-313: seeded_by_a_match only fires when the seed is itself in `matches` (green, same digest as the crate). Lines 316-330 then put every other match with a non-human provider_kind into `model`, and `(1, _) => Pipeline`. crates/harness-llm/src/migrate.rs:515-540: the seed checks cover stage, in-progress, binding, candidate, digest and verdict, but not provider_kind == HUMAN_KIND. docs/TUI-DESIGN.md §4 enables `m` Modify on any finished, bound attempt that has a candidate, so human attempts are included. crates/harness-cli/src/bench.rs:617-633 returns promoted = Some(steer) with no PROBLEM. Scenario: (1) `harness override u ./edit` records H (green digest Dh, or red class oracle). (2) `harness migrate u --from H --steer 'tidy the comments' --no-promote` makes the model return H's code plus one comment, giving S green with digest Ds != Dh and seeded_from H. (3) `harness promote u S`. (4) `bench score --write`: matches=[S], S's seed H is not in matches, so the result is Pipeline(S). The case is strict-pass with no problem, and `bench check --replay` also passes because S replays and H is checked intact. A red H ('fix the failing check') works the same way. The existing test only covers a steer that reproduced its human seed (attempts.rs:707-713).

**Verifier.** Confirmed against the code.

1. crates/harness-core/src/attempts.rs:299-310. `matches` holds only green, bound attempts whose candidate_digest equals the crate's. `seeded_by_a_match` collapses a steer only when its seed is itself in `matches`, so only a steer that reproduced its seed byte for byte is collapsed.
2. attempts.rs:316-330. Every other match whose provider_kind is not "human" goes into `model`, and `(1, _) => Pipeline`. seeded_from is never followed past one hop, and records outside `matches` are never consulted.
3. crates/harness-llm/src/migrate.rs:469-590. first_turn() checks the seed's stage, in-progress state, binding, candidate plus digest, and verdict. It has no provider_kind check. crates/harness-cli/src/main.rs has no restriction either. So `--from <human attempt>` is accepted. TUI-DESIGN §4 enables `m` Modify on any finished, bound attempt with a candidate, which includes human attempts, so hand edit → Modify → Accept is a designed flow.
4. crates/harness-cli/src/bench.rs:617-631 gives `promoted = Some(S)` for Pipeline(S), so promoted_count = 1. The Human PROBLEM at bench.rs:673-692 only fires when promoted_count == 0. The replay loop skips only records whose own provider_kind is human, so S replays conformantly, with its seed H checked intact.

Scenario:
- `harness override u ./edit` records H, green with digest Dh (or red).
- `harness migrate u --from H --steer 'tidy comments' --no-promote` makes the model return H's code plus one comment, giving S, green, with digest Ds != Dh and seeded_from H.
- `harness promote u S`.
- In bench, matches = [S]. H is not in matches, so nothing collapses, model = [S], and the result is Pipeline(S). The case scores strict-pass with S's turn count, raises no PROBLEM, and `--write` accepts it.

The two-hop case also fails: S2 is seeded from S, S from H, and S2 reproduces S. Then S2 collapses into S and the result is still Pipeline(S).

The implementer clearly meant derived-from-human to stay human. The test at attempts.rs:707-712 pins the exact-reproduction case ("the human is the source"), but the rule stops at one exact hop.

This breaks §5.2's headline rule, "The benchmark never counts a hand edit as the pipeline's", which was the resolution of LEDGER-H3/SCOPE-1. The code does follow the letter of the §2 rule, "exactly one model attempt → Pipeline", so this is a design gap the implementation inherited, not a deviation. It is still a provenance and benchmark-integrity break in a reachable, designed workflow, so high stands.

Caveat: a steer from a model seed can also carry human-dictated code through its note. The design accepts that as pipeline. The human-seed case is different in kind, because the whole [CURRENT RUST] is hand-written.

Refusing `--from` a human attempt would contradict §4 Modify, so following the ancestry chain is the right fix.

**Fix (corrected).** In provenance() (crates/harness-core/src/attempts.rs:316-324), add one step for each match that is not collapsed.

1. Walk the match's seeded_from chain through `records` by id, for at most records.len() steps so a cycle cannot loop. Stop at a missing id.
2. If any ancestor has provider_kind == HUMAN_KIND, push that human ancestor into `human`, deduplicated by id, instead of pushing the match into `model`.

Report the human ancestor, not S. That keeps `Provenance::Human(r)` naming the actual override attempt, and bench's existing text ("promoted from human (override) attempt {id}") stays accurate with no change to bench.rs. The binding needs no extra check, because first_turn already requires every seed to be bound to the current inputs.

Update the docs to match:
- the doc comment on Provenance::Human;
- the provenance() doc;
- docs/SCHEMAS.md;
- TUI-DESIGN §2. Suggested wording: "a match whose seeded_from chain reaches a human attempt counts as that human attempt".

Do not refuse `--from <human>`: §4 designs Modify-after-hand-edit.

Regression test, in provenance_is_the_one_r5_rule:
- h = prov_rec("a-h", HUMAN_KIND, "green", "blake3:h"); s = AttemptRecord{seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:c")}. Assert p(&[h.clone(), s.clone()], Some("blake3:c")) == "human:a-h". Today it is "pipeline:a-s".
- Two hops: s2 = AttemptRecord{seeded_from: Some("a-s".into()), ..prov_rec("a-t", "external", "green", "blake3:c")}. Assert p(&[h, s, s2], Some("blake3:c")) == "human:a-h", both for s2 reproducing s and for s2 with a different digest.
- A red h: prov_rec("a-h", HUMAN_KIND, "red", "blake3:h") as the seed must also give "human:a-h".
- An independent model attempt m with digest "blake3:c" plus s must still give "pipeline:<m>".
- Cycle: a-x seeded from a-y and a-y seeded from a-x must terminate.

**Regression test.** In attempts.rs provenance_is_the_one_r5_rule: h = prov_rec("a-h", HUMAN_KIND, "green", "blake3:h"); s = AttemptRecord{seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:c")}. Assert p(&[h.clone(), s.clone()], Some("blake3:c")) starts with "human:"; today it is "pipeline:a-s". Add a two-hop chain (s2 seeded from s) and a red h. Both must also be human.

## RI-1 — A steer attempt seeded from a human attempt gets pipeline provenance, so a hand edit is scored as pipeline output

Severity: high (claimed high).

**Claim.** `provenance` folds a steer attempt into its seed only when the steer reproduced the seed byte for byte (the seed is in `matches`). A steer seeded from a human (override) attempt that changes even one byte is classed as a model attempt, i.e. `Pipeline`. `first_turn` also accepts a human attempt as a seed, because it never checks `provider_kind`. The result is that a hand edit revised by the model reaches the benchmark as strict-pass pipeline output and `bench score --write` accepts it. This contradicts §5.2: "The benchmark never counts a hand edit as the pipeline's."

**Evidence.** crates/harness-core/src/attempts.rs:308-329: `seeded_by_a_match` only looks inside `matches` (green, bound, digest == crate), so human ancestry outside that set is never seen, and a steer with a new digest lands in `model` and becomes `(1, _) => Pipeline`. crates/harness-llm/src/migrate.rs:514-547: the seed checks cover stage, outcome, binding, candidate and verdict, but not the human kind. crates/harness-cli/src/bench.rs:612-626 maps `Pipeline` to the promoted attempt and raises no PROBLEM. The unit test at attempts.rs:707-713 covers only a steer that reproduced its human seed exactly.

**Verifier.** Confirmed against the code.

1. A human attempt passes every check `first_turn` makes on a seed (crates/harness-llm/src/migrate.rs:469-580). It rejects a driver stage (517), an in-progress outcome (520), superseded binding (525), a missing or modified candidate, and a missing attempt-verdict.json (548-555). It never looks at `provider_kind`. `record_human_attempt` writes candidate/ through `stage.judge`, and any judge run that reaches the oracle stores attempt-verdict.json (migrate.rs:823). So a green human attempt, or one that went red at the oracle, is a valid `--from`. Nothing in the CLI blocks it either: the harness-cli main.rs steer/from plumbing only checks that both flags are present.

2. `provenance` (crates/harness-core/src/attempts.rs:288-333) folds a steer into its seed only when the seed is itself in `matches`, meaning green, bound, and with a digest equal to the crate (308-312, 316). Take a steer seeded from human attempt a-h whose candidate differs from a-h by even one byte. Then a-h is not in `matches`, and the steer's own `provider_kind` is the model's (e.g. "external"). At 319-323 it goes into `model`, and at 328 `(1, _)` returns `Provenance::Pipeline(steer)`.

3. bench.rs:617-618 maps `Pipeline` to the promoted attempt. The only human PROBLEM (674-683) fires when `promoted_count == 0`, so nothing is flagged and `bench score --write` records the crate as a strict pass for the pipeline. The TUI read model calls the same function (crates/harness-tui/src/model.rs:205) and would mark it `*` (pipeline) rather than `*h`.

Concrete scenario: `harness override U dir/` records a-h (green, or red at the oracle because it is nearly right). Then `harness migrate U --steer 'fix the off-by-one' --from a-h` gives the model a-h's code as `[CURRENT RUST]`. The model changes one line and the result is green (a-s, new digest). Next, `harness promote U a-s`, followed by `bench score --write`. The crate, almost entirely hand-written, is scored as pipeline output. This breaks the invariant §5.2 states: "The benchmark never counts a hand edit as the pipeline's."

The design does not forbid a human seed. §0 describes Modify as a steer "seeded from the one being reviewed" (possibly a human attempt), and §5.1 accepts any finished attempt. So this is a real gap, not intended behaviour. The implementation's own test (attempts.rs:707-712) covers only the exact-reproduction case. The design's collapse wording ("reproduced its seed") follows the letter, but it only works for model seeds. The path is reachable in normal use (hand edit, then "model, finish this"), and it corrupts the committed benchmark evidence. High is justified.

A fix in `provenance` is better than refusing `--from <human>` in `first_turn`. Refusing would take away a Modify workflow the design allows, and it would need a design and SCHEMAS change. Keeping the rule in the one R-5 function also fixes the TUI and bench together. Human records only ever appear as chain roots, since `override` takes no `--from`, so walking up the `seeded_from` chain is enough. An ancestor missing from `records` is already an integrity error on the replay path (§5.1), so treating it as not-pipeline is optional hardening, not part of the minimal fix.

**Fix (corrected).** In crates/harness-core/src/attempts.rs `provenance`, a match is human-derived if it is a human attempt or if its `seeded_from` chain inside `records` reaches one. Add a closure that walks the chain with a bound (and is therefore cycle-safe):

```rust
let human_derived = |r: &AttemptRecord| {
    let mut cur = r;
    for _ in 0..=records.len() {
        if cur.provider_kind == HUMAN_KIND { return true; }
        match cur.seeded_from.as_deref()
            .and_then(|s| records.iter().find(|x| x.id == s && x.stage.is_none())) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    false
};
```

In the loop at 319, replace `if r.provider_kind == HUMAN_KIND` with `if human_derived(r)`, so these matches land in the `human` bucket and never in `model`.

Keep the existing collapse at 316 and the `(1, _)` rule, under which an independent, purely model attempt with the same digest still wins.

`Human(r)` then names the attempt whose candidate is the crate. Optionally, reword the bench.rs:678 message to say "promoted from human (override) attempt X, or a steer attempt seeded from one".

Update the rule text in docs/SCHEMAS.md (~882) and docs/TUI-DESIGN.md §2 to: "a model attempt with no human ancestor in its seeded_from chain".

Optional hardening: treat a seed that cannot be resolved as not-pipeline.

Regression test: add these cases to `provenance_is_the_one_r5_rule`.
- h = prov_rec("a-h", HUMAN_KIND, "green", "blake3:h") and s = AttemptRecord{seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:s")}. Assert that p(&[h, s], Some("blake3:s")) starts with "human:". Today it returns "pipeline:a-s".
- The same case with h red.
- A two-hop chain: h, then s1 (blake3:s1) seeded from a-h, then s2 (blake3:s2) seeded from s1. Assert that p(&[h, s1, s2], Some("blake3:s2")) starts with "human:".

**Regression test.** In `provenance_is_the_one_r5_rule`, set `h = prov_rec("a-h", HUMAN_KIND, "green", "blake3:h")` and `s = AttemptRecord{seeded_from: Some("a-h".into()), ..prov_rec("a-s", "external", "green", "blake3:s")}`, then assert `p(&[h, s], Some("blake3:s")) == "human:a-h"`. Today it returns "pipeline:a-s". Add the same case with a red human seed (the model fixed a nearly-right hand edit).

## BENCH-H2 — A human attempt can become the case's scored 'unverified candidate' and feed migrate_outcome

Severity: medium (claimed high).

**Claim.** Only the verified-crate path goes through provenance(). The other per-case summaries in score_one still read every migrate attempt, human ones included. A red human attempt that failed on behaviour gets scored as the pipeline's candidate and counts toward vector_pass_oracle_red, and human outcomes feed migrate_outcome. None of this raises a PROBLEM, so `bench score --write` records it.

**Evidence.** crates/harness-cli/src/bench.rs:705-722: the candidate filter is `outcome == "red" && last turn result == "oracle" && candidate_digest non-empty`, then `.min_by(id)`, with no provider_kind check. record_human_attempt (migrate.rs) records a turn whose result is the judge's class ('oracle' for a differential failure), writes candidate/ and sets candidate_digest, so a human attempt qualifies. crates/harness-cli/src/bench.rs:632-647: `finished` collects every attempt's outcome. crates/harness-core/src/bench.rs:985-993 (finalize): an unverified case whose candidate passes every vector counts in vector_pass_oracle_red. Scenario: unit U is unverified, with one model attempt red at 'oracle' (id a-9f…). The reviewer runs `harness override U edit/`, judged red at 'oracle', id a-0c…. `bench score --write` now scores a-0c (lowest id) as pipeline.candidate_attempt, puts the hand edit's vector results in each vector's `candidate` column, and, if those vectors all pass, adds 1 to vector_pass_oracle_red. Before the override the model's candidate was scored. Separately, model attempts all red plus one unpromoted green human attempt turns migrate_outcome from 'red' into 'mixed'.

**Verifier.** Confirmed in the code. The severity drops to medium because the headline numbers are not affected.

1. A red human attempt qualifies as the unverified candidate. In record_human_attempt (crates/harness-llm/src/migrate.rs:704-722) the one MigrateStage::judge runs. For a behavioural failure, judge writes candidate/ (write_candidate, migrate.rs:793) and sets record.candidate_digest before it classifies the failure (migrate.rs:827). The turn's result is then the judge's class, e.g. 'oracle' (migrate.rs:712-715), and the outcome is 'red' (migrate.rs:721). The filter in bench.rs:705-714 checks only `outcome == "red" && last turn result == "oracle" && candidate_digest non-empty`, then takes `.min_by(id)`. It never checks provider_kind, so the human attempt is eligible. Attempt ids are hashes, so which attempt wins is arbitrary. If no model attempt reached 'oracle' (for example all red at build or check), the hand edit is the only eligible attempt and is always the one scored. It then sets pipeline.candidate_attempt and inputs.candidate (bench.rs:718-720) and fills each vector's `candidate` column (bench.rs:748-756). If every vector passes, finalize adds 1 to vector_pass_oracle_red (crates/harness-core/src/bench.rs:988-993).

2. `finished` and the `m_attempts.first()` fallback (bench.rs:632-648) read every attempt, human ones included. Scenario: model attempts are all red and there is one unpromoted green human attempt. migrate_outcome becomes 'mixed' instead of 'red'. With only human attempts, it reports the hand edit's outcome as the pipeline's.

3. Nothing raises a PROBLEM on this path. In bench.rs, HUMAN_KIND is checked only in the replay loop (bench.rs:1258). The Human-provenance PROBLEM sits inside `verification == Verified` (bench.rs:673-693), so `bench score --write` records the values silently.

This departs from docs/TUI-DESIGN.md §5.2, which says the benchmark never counts a hand edit as the pipeline's. CasePipeline documents these fields as pipeline facts (crates/harness-core/src/bench.rs:820-846).

Why medium rather than high: the verified path goes through core `provenance` and is guarded. So strict_pass, verified and blind_spots cannot be contaminated, and replay is unaffected. What does get corrupted is a triage lead (vector_pass_oracle_red, which the struct docs call "a lead to triage by hand"), the per-vector candidate column, candidate_attempt and the descriptive migrate_outcome. That is a real, recorded misattribution that a user hits after any `harness override` on a suite target. It does not corrupt the headline score.

**Fix (corrected).** In score_one (crates/harness-cli/src/bench.rs), after `let m_attempts = attempts::load_unit_attempts(...)`, build a pipeline-only view: `let pipeline_attempts: Vec<&attempts::AttemptRecord> = m_attempts.iter().filter(|r| r.provider_kind != attempts::HUMAN_KIND).collect();`. Keep passing the full `m_attempts` to `attempts::provenance`, because it needs the human records to return `Human`. Use `pipeline_attempts` for three things:
(a) the `finished` set (bench.rs:632-636);
(b) the `(None, 0)` fallback that reads `.first()` (bench.rs:639-642);
(c) the unverified-candidate selection (bench.rs:705-714).

Optionally add a one-line core helper `attempts::is_pipeline(&AttemptRecord) -> bool` (`provider_kind != HUMAN_KIND`), so the replay skip (bench.rs:1258) and these summaries share one definition. Update the CasePipeline doc comments for migrate_outcome and candidate_attempt to say they cover pipeline (non-human) attempts only.

Regression test: factor out `fn unverified_candidate<'a>(rs: &[&'a AttemptRecord]) -> Option<&'a AttemptRecord>` and `fn migrate_outcome(promoted, rs) -> String`, then unit-test both with the existing `rec()` test builder. First case: [model red, last turn 'build'; human red, last turn 'oracle', candidate_digest set] must give candidate None. Today it returns the human attempt. Second case: [model red, human green] must give migrate_outcome 'red'. Today it gives 'mixed'.

**Regression test.** Factor the candidate selection into `fn unverified_candidate(&[AttemptRecord]) -> Option<&AttemptRecord>` and unit-test it: records = [model red whose last turn is 'build', human red whose last turn is 'oracle' with a candidate_digest] → None. Today it returns the human attempt. Also test migrate_outcome for [model red, human green] → 'red'.

## CLI-M1 — Notes starting with '-' are rejected by clap (or print help and exit 0), and the resume hint's `--steer '<note>'` form breaks the same way

Severity: medium (claimed medium).

**Claim.** `--steer` and override's `--note` are plain `#[arg(long)]` options. When an option is pending, clap 4.6.7 still parses a following token that starts with `-` as a flag, so a note like '- use iter()' or '--no, keep the loop' is a usage error (exit 2), and '-h…' prints the help text to stdout and exits 0. `validate_note` accepts all of these as printable notes. So the TUI's `m`/`e` argv (`--steer <note>`, `--note <text>` as separate elements, TUI-DESIGN §4) fails for them. And if a user got past this with `--steer=-x`, `resume_command` rewrites it as `--steer '-x'`, which clap refuses on resume.

**Evidence.** crates/harness-cli/src/main.rs:132-136 (steer/from) and 147-148 (override note): no `allow_hyphen_values`. main.rs:1043-1045 renders ` --steer {q(note)}` as a separate word. clap_builder-4.6.7/src/parser/parser.rs:147-260: `to_long()`/`to_short()` are tried before the pending `ParseState::Opt` value (line ~285); parse_short_arg:882-1013 returns NoMatchingArg for an unknown char and runs the Help action for 'h'. Scenarios: (a) TUI Modify with note "- prefer iter() over indexing" spawns [..,"--steer","- prefer…"]; clap says "unexpected argument '- ' found", exit 2, and no attempt is made. (b) Note "-h: keep the wrapping add" makes clap print help on stdout (a non-NDJSON line under --json) and exit 0, so the TUI reads success. (c) `harness migrate u --from a-… --steer='-keep the loop'` reaches awaiting; the resume hint `harness migrate u … --steer '-keep the loop'` exits 2 under sh.

**Verifier.** Confirmed against the code and the vendored clap_builder-4.6.7 source. I did not run cargo; the parser source settles the question.

1. The options take hyphen-leading values only in attached form. crates/harness-cli/src/main.rs:130-136 (`steer`, `from`) and :147-148 (override `note`) are plain `#[arg(long)]`. No `allow_hyphen_values` appears anywhere in crates/harness-cli/src.

2. Clap checks for a flag before it uses a pending value.
   - In clap_builder parser.rs:147-260 the loop tries `to_long()` and `to_short()` before the pending `ParseState::Opt` value branch at ~:285.
   - `parse_long_arg` (:775-780) and `parse_short_arg` (:893-898) skip the flag parse only when the pending arg `is_allow_hyphen_values_set()`.
   - Otherwise an unknown short char returns `NoMatchingArg` (:1009-1011). The main loop turns that into `unknown_argument` (:270-281), which exits 2.
   - A known flag with no value, such as the auto-generated `-h`, goes through `react` (:949-960). `ArgAction::Help` at :1302-1310 returns the DisplayHelp error, so `Cli::parse()` prints help to stdout and exits 0.
   - The attached form avoids all of this. In `parse_long_arg` a `long_value` goes straight to `parse_opt_value` (:824-830), so `--steer=-x` is accepted.

3. `validate_note` accepts these notes. crates/harness-llm/src/migrate.rs:283-309 refuses only empty, oversize, control-character and `[WORDS]`-header notes, and TUI-DESIGN §4/§5.1 promise "1..2000 printable bytes".

4. The resume hint is a real bug in code that ships now. main.rs:1043-1045 renders ` --steer {q(note)}` as a separate shell word. `shell_quote` (report.rs:42-51) treats `-` as safe, so the note `-x` comes out as `--steer -x` and `-keep the loop` as `--steer '-keep the loop'`. The shell passes that value as its own argv element, so re-running the hint exits 2.

5. The authoritative design specifies the failing shape. TUI-DESIGN.md:154 gives the Modify argv as `... --steer <note>` and :155 gives `[--note <text>]`, both as separate elements, and the `r` retry at :156 rebuilds `--steer` from the record.

Failure scenarios:
- (a) A note like "- prefer iter()" fails with "unexpected argument '- '" and exit 2, and no attempt is made.
- (b) A note like "-h: keep the wrapping add" prints clap help on stdout and exits 0. Under `--json` that is a non-NDJSON line plus a success code with no events, which breaks the `--json` stdout contract a client relies on.
- (c) `migrate u --from a-… --steer='-keep the loop'` reaches `awaiting`, but its `resume` hint exits 2 under sh.

The `awaiting.args` array holds argv verbatim, including `--steer=-…`, so a machine client that re-runs `args` is fine. Only the human hint and the §4-shaped TUI/CLI invocations break.

Why medium and not high: nothing is written to the ledger, and replay and provenance are untouched. The failures are loud (exit 2) except the `-h` case. Why not low: the hint breaks now, and the TUI's specified Modify, hand-edit and retry argv will hit this for any bullet-style note.

The reviewer's warning about the alternative fix is also correct. With `allow_hyphen_values`, `--steer --json` would make the note "--json". The literal filter at main.rs:301 (`filter(|a| a != "--json")`) would then drop that token from `args` and the header, leaving `[..., "--steer"]`.

**Fix (corrected).** Use the attached form, the minimal fix with no parser change.
- (1) In crates/harness-cli/src/main.rs:1044 change the steer rendering to `cmd.push_str(&format!(" --steer={}", q(note)));`. It then renders as `--steer='-keep the loop'`, or `--steer=-x` for a safe string, and the shell passes it as one argv element. Optionally apply the same change to `--target`, since a relative target dir named `-x` hits the same split.
- (2) In docs/TUI-DESIGN.md §4 (lines 154-156) specify single argv elements: `--steer=<note>` for `m`, `--note=<text>` for `e`, and `--steer=<steer_note>` for the `r` retry of a steer attempt. State the same in docs/SCHEMAS.md / §5.1-5.2 ("a note beginning with '-' must use the `--steer=`/`--note=` form").

Do NOT add `allow_hyphen_values` unless `report::set_args` also stops filtering by string equality. It would have to drop only the `--json` index clap actually matched (via `ArgMatches::indices_of("json")`); otherwise the literal filter at main.rs:301 would drop a note equal to "--json" from `args`.

Regression tests:
- (1) steer_override.rs (fails today): run migrate with ["--from", &seed, "--steer=-keep the wrapping add"]. Assert exit 1 with an `awaiting` event whose `resume` contains `--steer=`. Write the response, then run `sh -c <resume>` with the test's harness on PATH. Assert exit 0, the same steer_id finished with steer_note == "-keep the wrapping add", and exactly two attempt dirs. Today the resume exits 2.
- (2) A unit test in main.rs: build `MigrateArgs { steer: Some("- x".into()), from: Some("a-1".into()), .. }` and split `resume_command()` the way sh would; for this string, split on the first space after `harness`. Feed the words to `Cli::try_parse_from` and assert Ok with `steer == Some("- x")`.

**Regression test.** In steer_override.rs, run migrate with ["--from", &seed, "--steer=-keep the wrapping add"], expect exit 1 with an awaiting event, write the response, then run `sh -c <resume>`. Expect exit 0, the same steer_id finished, and exactly two attempt dirs. This currently exits 2 on resume. Also add a clap unit test: `Cli::try_parse_from(["harness","migrate","u","--from","a-1","--steer=- x"])` is Ok and yields note "- x".

## CLI-M2 — An interrupted `harness override` leaves an in-progress human attempt that blocks recording the same edit, with no way out short of deleting it by hand

Severity: medium (claimed medium).

**Claim.** `record_human_attempt` stores an `in-progress` attempt.json and writes candidate/ before the oracle build. It cleans up only when `judge` returns Err. On SIGINT/SIGTERM/SIGHUP the signal thread makes the process die by the signal within 250 ms, so that cleanup usually never runs. Re-running the same override is then refused: 'identical to attempt … (in-progress)' or 'already recorded'. Unlike a model attempt, nothing resumes or finishes it. The TUI's `x` cancel (SIGINT to the child) during an `e` hand edit reaches exactly this state.

**Evidence.** crates/harness-llm/src/migrate.rs:685 stores the in-progress record, then judge → write_candidate (migrate.rs:793) → oracle build/run (794). migrate.rs:704-709 has the only cleanup path, `Err(e) => remove_path`. crates/harness-cli/src/main.rs install_signal_handler (~262-292) calls `emulate_default_handler(sig)` / `process::exit` from the signal thread without waiting for the main thread. migrate.rs:655-660 returns 'this edit is already recorded as attempt {id}' whenever the dir exists. crates/harness-cli/src/hand_edit.rs:42-56 has an identical-source loop that also matches in-progress records. Scenario: `harness --json override u /tmp/edit`, Ctrl-C (or TUI `x`) during the cargo build, leaves attempts/a-…/attempt.json {outcome: in-progress} plus candidate/src/*. Re-run `harness override u /tmp/edit` gets 'identical to attempt a-… (in-progress); nothing to record', exit 1, and does so on every retry. `state status` shows a-…:human:in-progress indefinitely.

**Verifier.** I checked the finding against the code and it holds.

1. The in-progress record is written before the long step. crates/harness-llm/src/migrate.rs:663-688 creates attempts/<id>/ and stores an attempt.json with outcome IN_PROGRESS. Only then does judge run (migrate.rs:704). judge writes candidate/ (migrate.rs:793) and runs the oracle build (migrate.rs:794), which takes seconds.

2. The only cleanup is `Err(e) => remove_path(&work_dir)` (migrate.rs:706-709). On a signal it usually does not run:
   - The signal thread (crates/harness-cli/src/main.rs:267-293) calls `kill_live_process_groups()` and sets CANCELLED.
   - Its helper thread writes one stderr line plus `report::result`, which normally takes microseconds.
   - It then calls `emulate_default_handler(sig)` / `process::exit`. The 250 ms is only an upper bound: `recv_timeout` returns once the helper sends.
   - The main thread only notices the killed child on its next `try_wait` poll. POLL is 50 ms (crates/harness-oracle/src/exec.rs:49, sleep at ~503). After that it returns `Err(Interrupted)` (exec.rs:511-517), which unwinds to the remove_path.
   - So the process almost always dies by the signal before remove_path runs, leaving attempts/<id>/{attempt.json (in-progress), candidate/...}.
   - Killed between prepare_dir and the first store, it leaves an empty dir.

3. A re-run cannot get past it:
   - The writer lock is flock-based, and the kernel releases it on death (crates/harness-core/src/ledger.rs:137-199), so the re-run proceeds.
   - `load_unit_attempts` → `load_records` (crates/harness-core/src/attempts.rs:343-366) loads every record, in-progress ones included.
   - So the identical-source loop at crates/harness-cli/src/hand_edit.rs:42-56 refuses: "identical to attempt a-… (in-progress); nothing to record".
   - If candidate/ was never written, or the dir is empty, migrate.rs:657-660 refuses "already recorded" instead.
   - The id is derived from the content (migrate.rs:649-656), so the same edit hits the same dir on every retry.

4. Nothing recovers it. Unlike a model attempt, which the same migrate re-run resumes, no path resumes, finishes or reclaims an in-progress human attempt. I grepped for HUMAN_KIND: the only uses outside the constant, record_human_attempt and tests are provenance (attempts.rs:319) and the bench replay skip (bench.rs:1258). There is no discard command.

5. The design does not cover this. TUI-DESIGN §5.2 says a re-run of the same source "is refused as identical", which is meant for finished attempts, and that a judge harness error "records nothing and removes the attempt dir". The interrupt case is `Error::Interrupted` reaching exactly that branch, but the process dies first. So the code misses the stated "records nothing" intent. The TUI's `x` (SIGINT to the child, §4) during an `e` hand edit reaches this state directly.

6. No existing test covers it. The migrate.rs:5360-5370 test only checks the finished duplicate.

Severity stays medium. There is no provenance, replay or trust break:
- An in-progress record cannot be promoted (promote.rs:403 requires green).
- bench skips human attempts.
- A trivially different edit still records, under a new id.

It is still a real dead end a TUI user hits. Retrying the same edit is refused on every retry, and the stale a-…:human:in-progress entry stays on every surface that lists attempts until someone runs rm by hand.

The proposed fix is safe. The id pins (unit, unit_source, driver, human, -, hash(logic‖NUL‖ffi)), so an in-progress human record under that id is this same edit, unfinished and never promoted. Reclaiming it rewrites nothing finished.

**Fix (corrected).** 1. In crates/harness-llm/src/migrate.rs `record_human_attempt`, replace the bare `attempt_dir(..).exists()` refusal at lines 657-661 with a reclaim step. It runs under the caller's writer lock.

```rust
let dir = attempts::attempt_dir(&ledger, &unit.id, &id);
if dir.exists() {
    match attempts::load_pinned(&ledger, &unit.id, &id)? {
        // An interrupted override of this same edit (the id is content-derived): never
        // finished, never promotable, so reclaim it and judge again.
        Some(rec) if rec.provider_kind == attempts::HUMAN_KIND && rec.outcome == IN_PROGRESS => remove_path(&dir)?,
        // Killed between prepare_dir and the first store: an empty dir, no record.
        None => remove_path(&dir)?,
        Some(_) => return Err(Error::Invariant(format!("this edit is already recorded as attempt {id}; nothing to record"))),
    }
}
```

`load_pinned` already refuses a record whose id or unit does not match its directory.

2. In crates/harness-cli/src/hand_edit.rs, the identical-source loop (lines 42-56) should `continue` for `rec.provider_kind == attempts::HUMAN_KIND && rec.outcome == "in-progress"`. The narrower alternative is to skip only `rec.id == attempts::attempt_id(&unit_id, &unit_source, &driver, attempts::HUMAN_KIND, "-", &harness_llm::human_edit_hash(&logic, &ffi))`. Finished duplicates, and in-progress MODEL attempts, are still refused.

3. Add one sentence to TUI-DESIGN §5.2 / SCHEMAS.md and to the fn doc: an interrupted override leaves an in-progress human record, and re-running the same override reclaims it.

Regression tests:
- **migrate.rs unit test.** Compute `id = attempts::attempt_id(UNIT, unit_source, driver, "human", "-", &human_edit_hash(LOGIC, FFI))`. Create attempts/<id>/ with an AttemptRecord {id, provider_kind: "human", outcome: "in-progress"} and candidate/src/{logic,ffi}.rs equal to the edit. Assert that `record_human_attempt(&oracle(vec![green()]), .., &edit(LOGIC))` returns Ok with outcome green, and that `load_unit_attempts` holds exactly one attempt.
- **Empty-dir variant.** attempts/<id>/ with no attempt.json gives the same result.
- **Finished duplicate.** Existing record green gives Err "already recorded"; the current assertion stays.
- **CLI e2e in steer_override.rs.** Same pre-seeded in-progress dir. `harness override u <dir>` exits 0 (not 1 with "identical to attempt"), and the attempt is now green.

**Regression test.** migrate.rs test: compute id = attempts::attempt_id(UNIT, src, drv, "human", "-", &human_edit_hash(LOGIC, FFI)). Create attempts/<id>/ with an AttemptRecord{provider_kind:"human", outcome:"in-progress"} and candidate/src/{logic,ffi}.rs equal to the edit. Call record_human_attempt(&oracle(vec![green()]), …, &edit(LOGIC)) and assert Ok with outcome green and exactly one attempt dir. This currently fails with 'already recorded'. Add a CLI e2e with the same setup that expects exit 0 rather than the 'identical to attempt' refusal.

## HUMAN-M1 — Ctrl-C or kill during override's judge leaves an in-progress human attempt that blocks the same edit permanently

Severity: medium (claimed medium).

**Claim.** record_human_attempt stores an in-progress attempt.json and writes candidate/ before the long oracle run. It removes the dir only when judge() returns Err inside the process. The designed cancel path (SIGINT from Ctrl-C, the TUI's `x`, or its SIGHUP forwarding) makes the process die by the signal before any cleanup runs. Human attempts have no resume path, so every re-run of that edit is refused: first as 'identical to attempt a-… (in-progress)', then as 'already recorded'. The only way out is deleting files under migration/ by hand.

**Evidence.** crates/harness-llm/src/migrate.rs:685-689 stores the in-progress record and emits turn-start. Lines 704-710 remove the dir only on an in-process Err. Lines 655-661 treat an existing dir as 'this edit is already recorded as attempt {id}; nothing to record' without looking at its outcome. crates/harness-cli/src/hand_edit.rs:42-56: the identical check covers every bound record, in-progress ones included, and write_candidate has already produced candidate/src/*.rs before verify runs. crates/harness-cli/src/main.rs:266-293: the signal thread kills the groups, lets the helper write, then calls emulate_default_handler, so main never unwinds. docs/CLI-HARDENING.md §3 justifies dying with 'the ledger is crash-consistent … a kill anywhere is re-runnable'. That holds for model attempts (trajectory.rs:1076 reset_unfinished) but not here. The final `record.store(&work_dir)?` at migrate.rs:722 leaves the same state if it fails. Scenario: `harness override u ./edit`, then press Ctrl-C while cargo builds the candidate. `state status` shows a-x:human:in-progress indefinitely. Re-running the same override exits 1 with 'identical to attempt a-x (in-progress); nothing to record'.

**Verifier.** Confirmed against the code. The finding is medium: it is a real dead end a user hits, but nothing in replay or provenance breaks.

1. **The in-progress record is written before the judge runs.** crates/harness-llm/src/migrate.rs:664-688 stores attempt.json with outcome IN_PROGRESS and turns=[] in attempts/<id>/. Then judge() runs: the deny scan, write_candidate at :793, and the oracle build at :794. The directory is removed only when judge returns Err in-process (:704-710).

2. **The in-process cleanup usually loses the race to the signal.** On SIGINT/SIGTERM/SIGHUP, main.rs install_signal_handler (~:267-293) calls kill_live_process_groups. It waits at most 250 ms for a helper that finishes in about a millisecond, then calls emulate_default_handler. The main thread only notices the dead child on its next try_wait poll (exec.rs POLL = 50 ms, :49). Only then does it return Err(Interrupted) and reach remove_path at migrate.rs:708. So the process normally dies first and attempts/<id>/ survives with candidate/src/*.rs. It always survives in these cases:
   - SIGKILL;
   - a signal that arrives while no child is live;
   - a failure of the final `record.store(&work_dir)?` at :722.

3. **Human attempts have no recovery path.** Model attempts recover on re-run via trajectory.rs:435 reset_unfinished. record_human_attempt never calls it, and migrate.rs:657-661 refuses whenever the directory exists, whatever its outcome is. It even refuses a directory with no attempt.json, which happens after a kill between prepare_dir and store. On top of that, hand_edit.rs:42-56 checks every bound record, in-progress ones included, via load_unit_attempts (core attempts.rs:181, no outcome filter). Once candidate/ exists, a re-run of the same edit exits 1 with "identical to attempt a-x (in-progress); nothing to record".

4. **No other command cleans it up.** There is no cleanup or gc command. The TUI acts do not cover it either: `R` needs an awaited response file, `m`/`r` need a finished attempt, `a` needs a green one. So the record stays in-progress forever. The only ways out are deleting ledger files by hand or changing a byte of the edit, which gives a different hash and a different id.

5. **This contradicts the documented contract.** CLI-HARDENING.md §3 justifies dying by the signal with "the ledger is crash-consistent". TUI-DESIGN.md §4 has `x` send SIGINT to exactly this child. TUI-DESIGN.md §5.2 does not address interrupting override.

Why medium and not high: bench filters out in-progress records (bench.rs:634, :1242), provenance needs a finished green digest, and promote refuses a non-green attempt. No invariant breaks. It is a user-facing dead end on a common action (Ctrl-C or `x` during a slow cargo build). None of the existing tests cover it; steer_override.rs has no kill case, and the migrate.rs human test only checks the finished "already recorded" path.

One correction to the proposed fix: do not skip every in-progress record in hand_edit.rs. A killed MODEL attempt whose candidate matches the edit must stay refused. That attempt can resume, finish with the same candidate_digest, and make R-5 provenance ambiguous. Only in-progress HUMAN records should be skipped. Override holds the writer lock, so such a record can only be left over from a killed override. And because the id is derived from human_edit_hash, a record with identical bytes is the same id that record_human_attempt will now reset.

**Fix (corrected).** crates/harness-llm/src/migrate.rs:657-663: replace the bare `attempt_dir(..).exists()` refusal with:

```rust
let existing = attempts::load_pinned(&ledger, &unit.id, &id)?;
if let Some(rec) = &existing {
    if rec.outcome != IN_PROGRESS {
        return Err(Error::Invariant(format!("this edit is already recorded as attempt {id}; nothing to record")));
    }
}
let work_dir = prepare_dir(&ledger, &unit.id, &work_rel)?;
// A killed override (override holds the writer lock, so no live writer owns it): start over.
reset_unfinished(&work_dir, &id)?;
```

Then keep the existing record.store and the rest. reset_unfinished already refuses a finished record, and a directory with no attempt.json (load_pinned returns None) now proceeds too.

crates/harness-cli/src/hand_edit.rs:42-45: in the identical-source loop, also `continue` when `rec.provider_kind == attempts::HUMAN_KIND && rec.outcome == "in-progress"`. Keep refusing in-progress MODEL attempts, whose resume would produce a candidate with the same digest.

Regression tests:
- **harness-llm** (a_human_attempt_… test): after the green LOGIC record, load attempt.json, set outcome "in-progress", turns=[] and candidate_digest "", store it, then call record_human_attempt(green, LOGIC) again. Assert Ok, outcome green, one human turn, and candidate_digest equal to crate_content_hash(candidate). Today it returns Err "already recorded".
- **harness-cli** (steer_override.rs): after a green override, rewrite that record's outcome to "in-progress" and re-run the same override. Assert exit 0 and that the record is green again. Today it exits 1 with "identical to attempt … (in-progress)".
- Optionally, a spawn-and-`/bin/kill -INT` e2e on the sleeping-driver fixture, then a re-run of the same override that exits 0.

**Regression test.** harness-llm: record LOGIC green, rewrite its attempt.json with outcome "in-progress" and turns=[] (the state after a kill), then call record_human_attempt(green, LOGIC) again. Assert Ok with outcome green; today it returns Err 'already recorded'. CLI (steer_override.rs): after a green override, set the record's outcome to in-progress and re-run the same override. Assert exit 0; today it exits 1 with 'identical to attempt …'.

## HUMAN-M2 — bench scores a red human attempt's candidate as the pipeline's candidate_attempt

Severity: medium (claimed medium).

**Claim.** For an unverified unit, score_one picks the scored candidate from every red attempt whose last turn is 'oracle', with no provider_kind filter. A red human attempt (its one turn gets the judge's class, e.g. 'oracle') can therefore become pipeline.candidate_attempt. Its vectors then feed the `candidate` counts and the vector_pass_oracle_red total, and `--write` accepts this without a PROBLEM.

**Evidence.** crates/harness-cli/src/bench.rs:706-721: the filter is `r.outcome == "red" && r.turns.last().is_some_and(|t| t.result == "oracle") && !r.candidate_digest.is_empty()`, then .min_by(id). Human records meet all three conditions (migrate.rs:712-721). crates/harness-core/src/bench.rs:842-846 documents candidate_attempt as pipeline output, and :989-994 counts vector_pass_oracle_red from it. Scenario: unit u has model attempt a-9… (red, oracle). The user records `harness override u ./edit` as a red human attempt a-1…. The lower id wins, so scores.json shows pipeline.candidate_attempt = the human id, and its vectors can raise vector_pass_oracle_red. Because inputs.candidate changes, `bench check` also reports the case's inputs changed (incomparable) purely because of a hand edit. pipeline.migrate_outcome (bench.rs:633-645) likewise folds human outcomes into 'mixed'.

**Verifier.** Confirmed against the code. In crates/harness-cli/src/bench.rs:706-721, the unverified-candidate selection filters only on `r.outcome == "red" && r.turns.last().is_some_and(|t| t.result == "oracle") && !r.candidate_digest.is_empty()`, then `.min_by(id)`, and needs `attempts/<id>/candidate` to be a directory. Nothing filters on provider_kind.

A red human attempt passes every one of these checks. record_human_attempt (crates/harness-llm/src/migrate.rs:615-740) runs the same MigrateStage::judge. That judge writes `candidate/`, sets `candidate_digest` after the build (migrate.rs ~826), and returns `classify(&verdict)`, which is "oracle" for any behavioral difference (migrate.rs:1061-1074). The record then gets a single turn with `result` set to that class and `outcome = "red"` (migrate.rs ~720).

Attempt ids are blake3-derived (`a-` + 12 hex, attempts.rs:104-119), so min_by(id) is effectively arbitrary. A human red attempt displaces a model red attempt about half the time, and it is the only candidate when no model red/oracle attempt exists.

Consequences:
- scores.json `pipeline.candidate_attempt` names a human id. CasePipeline is documented as "Pipeline facts" in crates/harness-core/src/bench.rs:821-846.
- The human candidate's vectors feed `candidate` Counts and `vector_pass_oracle_red` (core bench.rs:989-994).
- `inputs.candidate` changes, so `bench check` lists the case under input_changes (core bench.rs:1145) purely because of a hand edit.
- migrate_outcome (cli bench.rs:632-645) tallies human outcomes into `finished`. Example: a model red plus an unpromoted human green gives "mixed" instead of "red".
- No PROBLEM is raised on any of these paths, so `--write` records them.

This contradicts docs/TUI-DESIGN.md §0 and §5.2 ("The benchmark never counts a hand edit as the pipeline's"). It also contradicts the implementation's own comment at bench.rs:674-676 ("the benchmark scores pipeline output only"). The design spells out the verified-crate mechanism, and that part is implemented correctly through core `provenance`. It says nothing about the unverified-candidate path, and the code left that path unfiltered.

Severity stays medium. The headline metrics (strict_pass, verified, blind_spots) are unaffected, and the human id stays visible in scores.json. But a secondary pipeline metric, the per-case candidate counts and migrate_outcome are misattributed. A user who records an override on an unverified unit also gets a spurious bench-check input change.

**Fix (corrected).** In crates/harness-cli/src/bench.rs score_one, filter human attempts out of the records used for pipeline stats in one place and use that at both sites.

After loading `m_attempts` (bench.rs:599), add:

    let pipeline_records: Vec<&attempts::AttemptRecord> =
        m_attempts.iter().filter(|r| r.provider_kind != attempts::HUMAN_KIND).collect();

(1) migrate_outcome (bench.rs:632-645): build `finished` from `pipeline_records`, and take the `(None, 0)` fallback's `.first()` from `pipeline_records`.

(2) Candidate selection (bench.rs:706-715): iterate `pipeline_records` instead of `m_attempts`, or add `&& r.provider_kind != attempts::HUMAN_KIND` to the existing filter.

`provenance()` still receives all of `m_attempts`, because it needs the human records to return `Provenance::Human`.

For testability, pull the choice out into `fn scored_candidate<'a>(records: &'a [AttemptRecord]) -> Option<&'a AttemptRecord>` holding the red / last-turn-oracle / non-empty-digest / non-human filter plus min_by(id). Keep the `dir.is_dir()` check at the call site.

Regression test in bench.rs `mod tests`, using the existing record helper there with provider_kind and turns set:
- [human red, last turn result "oracle", digest "blake3:x", id "a-0"; external red/oracle, id "a-1"] → Some("a-1").
- [human red/oracle "a-0"] alone → None.

Today's filter returns a-0 in both cases.

**Regression test.** Move the candidate choice into `fn scored_candidate(records: &[AttemptRecord]) -> Option<&AttemptRecord>` and unit-test it. With [human red/oracle "a-0", external red/oracle "a-1"] it must return a-1; with [human red/oracle] alone it must return None. Today it returns a-0.

## HUMAN-M3 — A deny-scan red human attempt records and reports no evidence, not even what was submitted

Severity: medium (claimed medium).

**Claim.** When the hand edit fails the deny scan, judge() returns before write_candidate, and record_human_attempt keeps only the failure class. The violation list is thrown away, no candidate/ or verdict is written, and the two files are stored nowhere. The ledger ends up with a red attempt whose only content is response_hash, a hash of text nobody holds. The user just sees 'RED (check)' and cannot find out why. Since the same edit is then refused as 'already recorded', running it again does not help. For a model attempt, the reply sits in traces and the scan can be re-run on replay; a human attempt has nothing equivalent.

**Evidence.** crates/harness-llm/src/migrate.rs:780-792: the deny-scan branch builds Failure{evidence: listed} with wrote_candidate false and verdict None. migrate.rs:712 keeps `judged.failure…class` only, and the evidence is dropped. crates/harness-cli/src/hand_edit.rs:67-79: with verdict None nothing but `override: u human attempt a-… -> RED (check)` is printed, and --json mode emits no `check` events. Scenario: a hand edit adds `unsafe { … }` to logic.rs (the test at migrate.rs:5386-5398). The output is 'RED (check)' with no reason, attempts/<id>/ holds only attempt.json, and a TUI or reviewer has nothing to show or diff.

**Verifier.** The claim holds, and medium is the right severity. The storage part matches the design. The missing reason is the gap a user actually hits.

What the code does:
- **The violations are dropped.** At crates/harness-llm/src/migrate.rs:780-791 the deny-scan branch of `judge` returns `Judged{wrote_candidate: false, failure: Some(Failure{class:"check", evidence: listed.concat()}), verdict: None}` before `write_candidate`. `record_human_attempt` keeps only the class (migrate.rs:712, `judged.failure.as_ref().map_or("green", |f| f.class)`). At migrate.rs:726-734 it returns a `MigrationOutcome` with `candidate_dir` None and `verdict` None. `MigrationOutcome` (migrate.rs:313-334) has no field that could carry the evidence, so the violation list is lost.
- **Nothing explains the red.** `cmd_override` (crates/harness-cli/src/hand_edit.rs:67-79) calls `report::verdict` only when `outcome.verdict` is Some. That function also returns early outside `--json` (report.rs:259-262). So the user sees only `override: <unit> human attempt a-… -> RED (check)`. No verdict file, `check` event or `message` line gives the reason.
- **Re-running does not help.** The identical-source refusal (hand_edit.rs:42-57) looks at `candidate/`, which a deny-scan red never writes, so it does not fire. `record_human_attempt` then refuses at migrate.rs:657-661 with "this edit is already recorded as attempt {id}; nothing to record", again without the reason. The only way to learn anything is to change the edit and guess which of the deny-scan rules fired (emission.rs:451ff has 11 substring rules plus others).
- **Tests do not cover it.** The unit test (migrate.rs:5386-5398) checks only `result == "check"` and that there is no candidate. The CLI e2e test (steer_override.rs) has no red human attempt.

Why this is not a design deviation, and so not high:
- §5.2 says the judge runs "exactly as for a model turn". A model's deny-scan tail also stores no verdict or candidate (main.rs comment near line 1197), so not persisting the edit files matches the design.
- A red human attempt affects no provenance (only green promoted attempts count) and bench skips human attempts.
- The reviewer overstates "text nobody holds": the user's DIR is never touched by `override`.

The real gap is that the edit's author, the one person who needs the feedback, gets a red with no reason. The design's TUI client has no verdict detail to show for it either. That is a real gap a user or client hits.

**Fix (corrected).** Minimal fix: carry the judge's failure evidence out when no verdict was stored, and print it.

1. In crates/harness-llm/src/migrate.rs, add a field to `MigrationOutcome`:
   `pub failure_evidence: Option<String>`
   It holds the final turn's failure evidence when that turn stored no verdict (the deny scan). In `record_human_attempt` (around migrate.rs:726), set it to `judged.verdict.is_none().then(|| judged.failure.as_ref().map(|f| f.evidence.clone())).flatten()`. Set it to None at the `run_migration` construction site (migrate.rs:455), or fill it the same way from the last turn if parity is wanted.

2. In crates/harness-cli/src/hand_edit.rs, after the verdict block (around line 73), add:
   `if let Some(ev) = &outcome.failure_evidence { for line in ev.lines() { out(format!("override: deny scan: {}", harness_llm::printable(line, 300))); } }`
   `out()` goes through `report::line`, so `--json` clients get `message` events without a new event kind. Nothing new is written to the ledger, so SCHEMAS.md needs no change.

Optional, and a schema addition rather than a bug fix: also write the submitted files with create_new to `attempts/<id>/edit/src/{logic.rs,ffi.rs}` and add a line to the SCHEMAS.md writer table. That makes a red human record re-checkable. It is not required for parity with model attempts, whose deny-scan replies live only in the gitignored traces.

Regression tests:
- In `a_human_attempt_is_judged_like_a_model_reply_and_labelled`, for the UNSAFE edit, assert `denied.failure_evidence.as_deref().is_some_and(|e| e.contains("unsafe"))`. For the green and oracle-red runs, assert `failure_evidence.is_none()`.
- In steer_override.rs, run `override` on a `src/logic.rs` containing `unsafe {}` and check two things: the exit code is 10, and a `message` event, or plain stdout without `--json`, contains the deny-scan violation line. Both assertions fail before the fix.

**Regression test.** In a_human_attempt_is_judged_like_a_model_reply_and_labelled, for the UNSAFE edit, assert the outcome's evidence contains the deny-scan violation and that attempts/<id>/edit/src/logic.rs equals UNSAFE. CLI: an override whose logic.rs contains `unsafe {}` must print the violation line to stdout.

## RI-2 — The steer fields (seeded_from, steer_note) and the seed's stored verdict are not integrity-bound to the recorded turn 1

Severity: medium (claimed medium).

**Claim.** When a pinned steer attempt is verified, turn 1 is rendered from `record.seeded_from`/`steer_note` and from the seed's `attempt-verdict.json`. No integrity check ties any of these to the recorded request: the id re-derivation and `prompt_digest` bind only `turns[0].request_key`. So adding, removing or editing the steer fields changes only sections outside `[EVIDENCE]`, which counts as drift, and `divergences()` never compares `Turn.kind`. The attempt therefore still "reproduces", only drifted. Provenance's steer collapse keys on that unchecked `seeded_from`. Likewise, a modified seed verdict shows up as an evidence-only divergence, which `superseded.jsonl` can excuse, or as plain drift. §5.1 says a modified seed is an integrity error, never a divergence. The trace-backed re-run and `--retry` never compare the record's steer fields with `--from`/`--steer` either, although §5.1 calls a mismatch an error.

**Evidence.** crates/harness-llm/src/migrate.rs:482-513: the pinned path trusts the record's fields as they are. crates/harness-llm/src/trajectory.rs:614-683: `recorded_pairs` checks nothing about the steer fields. trajectory.rs:1252-1257: `evidence_only` is false whenever `[CURRENT RUST]`, `[HISTORY]` or `[GUIDANCE]` differ, so the result is drift only. trajectory.rs:917+ `divergences`: no `kind` comparison. crates/harness-cli/src/bench.rs:1384-1393: drift is not a problem. crates/harness-core/src/attempts.rs:308-317: the collapse keys on `seeded_from`. Nothing hashes or checks the seed's verdict (only its `candidate/`, at migrate.rs:534-541).

**Verifier.** The finding holds. I checked it against the code; nothing verifies the new steer fields against the recorded evidence.

**1. The pinned path trusts the record.**
- migrate.rs:478-509 (`first_turn`) builds turn 1 straight from `record.seeded_from`/`steer_note`. If `steer_note` is present but `seeded_from` is absent, the record is quietly treated as a translate attempt.
- trajectory.rs:614-684 (`recorded_pairs`) binds only `turns[0].request_key`: the id is re-derived from it and `prompt_digest` is checked against the recorded request. Neither the steer fields nor `turns[].kind` is checked (`divergences`, trajectory.rs:917-961).
- Scenario (a): remove both fields from a steer attempt's attempt.json. The job renders a translate turn 1. At trajectory.rs:1252-1257 `evidence_only` is false, because `steer()` is None and index is 0. Turn 2 also differs outside `[EVIDENCE]`, since `[GUIDANCE]` and `[HISTORY]` changed. Every result, `candidate_digest` and `outcome` still match, so the call returns Ok with `drifted` [0,1].
- Scenario (b): add fake `seeded_from`/`steer_note` to a translate record whose seed is finished, bound and intact. Turn 1 renders as a steer, the difference is not evidence-only, and the call returns Ok with `drifted` [0].
- In `bench check --replay`, a record that reproduces with drift is not a problem (bench.rs:1384-1394).

**2. The trace-backed paths deviate from §5.1.**
- §5.1 says the verification paths (finished trace-backed re-run, `--retry` re-verification, `replay_divergences`) build turn 1 from the record, and that a mismatch with `--from`/`--steer` is an error.
- The code builds turn 1 from the CLI in every case except a pinned replay: `first_turn` loads the record only when the provider kind is `replay` and `--attempt` is given (migrate.rs:478).
- The trace-backed re-run (trajectory.rs:386-404), `trace_backed_sample` (trajectory.rs:570) and the unpinned replay (`find_recorded`) never compare the record's steer fields with the CLI. `seeded_from`/`steer_note` are otherwise used only at trajectory.rs:429-430 (when writing) and at attempts.rs:309.
- Because the id binds the turn-1 key, a record whose fields disagree with the CLI must have been edited. Today such a record verifies conformant, with not even a drift flag.

**3. Why this matters: provenance.**
- R-5 used to depend only on fields that are either bound into the id or re-checked by replay (outcome, unit_source/driver, candidate_digest, provider_kind).
- The steer collapse (attempts.rs:308-322) now also depends on `seeded_from`, which nothing verifies.
- Scenario: two green model attempts A and B share the crate digest, so `bench score` reports "ambiguous provenance" (bench.rs:619-626). Add `seeded_from: A` plus any valid note to B. Provenance becomes `Pipeline(A)`, the problem disappears, and `bench check --replay` reports only "B reproduces; prompt: drifted". An edited `steer_note` likewise makes the ledger claim guidance the model never received, and every check still passes.

**Severity: medium, not high.** Reaching it requires hand-editing committed attempt.json. It cannot make a human edit count as pipeline output. But the project defends against exactly this class of edit elsewhere: the id re-derivation, the misfiled-record refusal in `load_records` (whose comment cites provenance), and the candidate-digest check.

**4. The seed-verdict sub-claim is real but weaker.**
- attempt-verdict.json is not integrity-bound for any attempt; that predates this change.
- If the `detail` of a red seed's verdict is edited, the steer's turn 1 differs only in `[EVIDENCE]`. That yields `Error::Diverged`, which is a bench problem by default and can be excused only by a reviewed superseded.jsonl entry. It is misclassified relative to §5.1's "integrity, never a divergence", but it is not missed.
- The proposed "evidence-only difference at index 0 → integrity" fix is wrong. A legitimate HEAD change to how `oracle_evidence`/`classify` render a stored verdict produces the same signal. Turning that into an integrity error, which superseded.jsonl can never excuse, would stop recorded steer attempts from replaying. Drop that part of the fix.

**Fix (corrected).** A minimal fix in two parts. Neither changes the frozen id or any prompt.

**(1) Record vs job, on every verification path.** At the top of `Job::replay_divergences` (trajectory.rs:690), which every verification path passes through, check that the record's steer fields equal the job's first turn:
```rust
let job = self.steer().map(|s| (s.seed_id.as_str(), s.note.as_str()));
let rec = match (recorded.seeded_from.as_deref(), recorded.steer_note.as_deref()) {
    (Some(f), Some(n)) => Some((f, n)),
    (None, None) => None,
    _ => return Err(integrity("seeded_from and steer_note must be recorded together")),
};
if rec != job {
    return Err(integrity("its steer fields do not match the first turn being verified"));
}
```
In the pinned path the two sides are equal by construction. In the trace-backed re-run, `--retry` and unpinned replay, the id binds the recorded turn-1 key to the CLI's rendering, so any mismatch means the record was edited.

**(2) Record vs recorded request.** In `recorded_pairs`, after the `prompt_digest` check on turn 1, test the RECORDED turn-1 user text (pairs[0].0.user):
- For a steer record: it must contain `\n[GUIDANCE]\n{note}\n\n[TASK]\n` and `seeded from attempt {seed}`.
- For a non-steer record: it must contain no `\n[GUIDANCE]\n`.
- Any failure is an integrity error.

This is robust because `validate_note` refuses lines that look like section headers, and it tests only what the record itself fixed, so older recordings keep replaying.

**Seed verdict.** Do NOT reclassify an index-0 evidence-only difference as integrity; a legitimate change to the evidence renderer produces the same signal. If verdict integrity is wanted, add an additive steer-record field `seed_verdict: blake3(attempt-verdict.json bytes)`. Set it at creation and check it in `first_turn` when loading the seed; a mismatch is an integrity error.

**Regression tests** (extend `a_steer_attempt_carries_the_note_on_every_turn_and_replays_conformant`):
- **(a)** Strip `seeded_from`/`steer_note` from `done.record`'s attempt.json. A pinned replay must return an Err containing "integrity"; today it returns Ok with `drifted` Some([0,1]). Separately, a trace-backed re-run (hand-off provider plus the same `SteerArgs`) against the stripped record must return an integrity Err; today it passes conformant.
- **(b)** Give the seed record `seeded_from = done.record.id` and `steer_note = NOTE`. A pinned replay of the seed must return an integrity Err; today it returns Ok with `drifted` Some([0]).
- **(c)** Only if the `seed_verdict` field is added: edit the red seed's verdict `detail`. A pinned replay of the steer must return an integrity Err, not `Error::Diverged`.

**Regression test.** Extend `a_steer_attempt_carries_the_note_on_every_turn_and_replays_conformant`:
(a) Remove `seeded_from`/`steer_note` from `done.record`'s attempt.json. A pinned replay must return an Err containing "integrity"; today it returns Ok with drifted Some([0,1]).
(b) Set the seed's (translate) record to `seeded_from = done.record.id`, `steer_note = NOTE`. A pinned replay of the seed must be an integrity Err; today it reproduces, drifted.
(c) In the red-seed test, edit the `detail` of the seed's attempt-verdict.json. A pinned replay of the steer must return an integrity Err, not `Error::Diverged`.

## RI-3 — bench scores a human red attempt as the pipeline's unverified candidate

Severity: medium (claimed medium).

**Claim.** `bench score` picks the unverified candidate from every red attempt whose last turn failed on `oracle`, and it does not exclude human attempts. A red human attempt has outcome red, turn result `oracle` and a `candidate_digest`, so it qualifies. Its candidate is then built and scored, `pipeline.candidate_attempt` holds the human id, and it can count toward `vector_pass_oracle_red`. That reports a hand edit as pipeline output.

**Evidence.** crates/harness-cli/src/bench.rs:705-720: the filter is `outcome == "red" && last.result == "oracle" && !candidate_digest.is_empty()`, then `min_by(id)`. It has no `provider_kind` check, and `pipeline.candidate_attempt = cand.id` is set at line 719. crates/harness-core/src/bench.rs:842-846 documents `candidate_attempt` as a pipeline field. migrate.rs's human test asserts `red.record.turns[0].result == "oracle"`.

**Verifier.** I confirmed this against the code.

1. The selection has no human check. `m_attempts` comes straight from `attempts::load_unit_attempts` at crates/harness-cli/src/bench.rs:601, which loads every record under `attempts/` with no filter. The unverified-candidate branch at bench.rs:705-721 keeps any record with `outcome == "red"`, a last turn of `oracle` and a non-empty `candidate_digest`, then takes `min_by(id)`. It never looks at `provider_kind`. The diff leaves this block unchanged. The only human handling added to bench.rs is the verified-crate PROBLEM (bench.rs:674-693) and the replay skip (bench.rs:1258-1266).

2. A red human attempt passes that filter. `record_human_attempt` (crates/harness-llm/src/migrate.rs:615-740) runs the shared `stage.judge`. The judge sets `record.candidate_digest` after the oracle runs (migrate.rs:827) and writes `attempts/<id>/candidate/`. The single turn's `result` is the failure class, and `outcome` is `red`. The human test checks `turns[0].result == "oracle"` (migrate.rs:5384).

3. The effect on scores. The case goes through `build_crate_staticlib` and is scored. `pipeline.candidate_attempt` is set to the human id (bench.rs:719). That field sits in `CasePipeline` and is documented as "the unverified candidate scored for this case" (crates/harness-core/src/bench.rs:842-846). If the edit passes every non-UB vector, `Scores::finalize` adds 1 to `SplitTotals.vector_pass_oracle_red` (core bench.rs:989-993).

Concrete scenario: unit U is unverified, and its model attempts are red on `build`, or it has none. The user runs `harness override U dir` with an edit that builds but differs from the C on the driver, so the attempt is recorded red on `oracle`. `bench score --write` then commits a scores.json in which this case's candidate and its triage-lead counter come from the hand edit, attributed to the pipeline. The same thing happens when a model attempt is also red on `oracle`: ids are content hashes, so `min_by(id)` picks the human one whenever its id sorts first, which is arbitrary. The model's candidate is then dropped. §5.2 of the design states "The benchmark never counts a hand edit as the pipeline's", and this path breaks that rule.

Why medium, not high: the verified/strict-pass headline is already protected by the Human-provenance PROBLEM. What leaks is the unverified-candidate column and `vector_pass_oracle_red`, which is documented as "a lead to triage by hand ... NOT a false negative by itself".

I reject the second half of the proposed fix, excluding steer attempts with human ancestry. The core `provenance` function (crates/harness-core/src/attempts.rs:288-331) counts a steer attempt as model output. It is folded into its seed only when it reproduced the seed's candidate. §5.2 does not say to exclude steer attempts. Excluding them here would disagree with the one R-5 rule. It should wait on RI-1 and not be part of the minimal fix.

**Fix (corrected).** In crates/harness-cli/src/bench.rs:709-713, add `r.provider_kind != attempts::HUMAN_KIND &&` to the candidate filter, so a labelled human attempt can never be the scored unverified candidate. The simplest way to test it is to move the predicate into a pure helper, `fn unverified_candidate(records: &[AttemptRecord]) -> Option<&AttemptRecord>` (filter + `min_by(id)`), and call that at line 705. Leave steer attempts in, which matches core `provenance`. Regression test in bench.rs `mod tests`: build `rec("a-1", HUMAN_KIND)` with outcome "red", one turn with result "oracle" and candidate_digest "blake3:x". `unverified_candidate(&[h])` must return None; without the fix it returns a-1. Then add `rec("a-2", "anthropic")`, also red on "oracle" with a digest. `unverified_candidate(&[h, m])` must return a-2; without the fix it returns a-1 because a-1 sorts first. Optionally add a doc line on `CasePipeline.candidate_attempt` saying it is never a human attempt.

**Regression test.** Move the selection into a helper, `unverified_candidate(&[AttemptRecord]) -> Option<&AttemptRecord>`, and unit-test it in bench.rs: for records that are only `rec("a-1", "human")` (red, last turn `oracle`, with a digest), expect None; today it returns a-1. For a human a-1 plus a model a-2 (both red on `oracle`), expect a-2.

## RI-4 — An interrupted `harness override` leaves an in-progress human attempt that blocks recording the same edit for good

Severity: medium (claimed medium).

**Claim.** `record_human_attempt` stores an `in-progress` record before the judge runs, and the judge builds and runs the crate, which can take a long time. The signal handler dies by the signal without unwinding, so nothing is removed after Ctrl-C, the TUI's `Q`, SIGTERM or a crash. On the next run, the same edit is refused by one of two checks:
- the `exists()` check reports "already recorded";
- hand_edit's identical-source check, which also matches in-progress records, reports "identical to attempt … (in-progress)".
Nothing was ever recorded, `promote` refuses the attempt (not green), and no command clears it. Unlike the engine, which starts an unfinished attempt over, this path never recovers.

**Evidence.** crates/harness-llm/src/migrate.rs:657-661: `attempt_dir(..).exists()` refuses with "already recorded". migrate.rs:677 and 685: the in-progress record is stored before the judge at line 704. crates/harness-cli/src/main.rs:245-296: `emulate_default_handler` means no cleanup runs. crates/harness-cli/src/hand_edit.rs:42-56: in-progress records are not skipped.

**Verifier.** I confirmed this against the code. The edit gets stuck, but no invariant is broken, so it stays medium.

**How the stuck record arises**
- `record_human_attempt` (crates/harness-llm/src/migrate.rs:615) builds the id from `(unit, unit_source, driver, "human", "-", human_edit_hash)`. The same edit therefore always maps to the same `attempts/<id>/`.
- It stores an `IN_PROGRESS` record at lines 663-686, before `stage.judge` runs at line 704.
- Inside the judge, `write_candidate` runs at about line 793, and then `self.verify` builds the crate and runs the oracle. That second step is the long window.
- The attempt dir is removed only when the judge returns `Err` (lines 706-709).
- A SIGINT, SIGTERM or SIGHUP ends the process through `emulate_default_handler` (crates/harness-cli/src/main.rs:294). That path runs no cleanup. The design's TUI `Q` sends SIGINT to the child (TUI-DESIGN §6), so an ordinary user action reaches this path.

**Why the same edit is then refused for good**
- If the interrupt lands during the build or oracle (the likely case), `candidate/src/{logic.rs,ffi.rs}` already exist. hand_edit.rs:42-56 does not filter on outcome, so it bails with "identical to attempt a-… (in-progress); nothing to record".
- If the interrupt lands in the short window before `write_candidate`, the check at migrate.rs:657-661 refuses because the dir exists: "this edit is already recorded".
- Nothing ever cleans this up:
  - The engine's `reset_unfinished` (trajectory.rs:1076) only runs for engine-derived ids, and those never equal a human id.
  - Only reads refer to `HUMAN_KIND` (attempts.rs:319, bench.rs:1258, harness-tui model.rs:70); nothing removes or resets a human attempt.
  - `promote` refuses the attempt because it is not green.

**Why medium, not high**
- Both bench paths filter out in-progress records (bench.rs:634, 1242), and `provenance` requires `outcome == "green"` (attempts.rs:300). No false evidence enters scoring.
- A workaround exists: change any byte of the edit, or delete the dir by hand. But the orphan dir stays in the ledger, and the refusal message misleadingly says "nothing to record".
- TUI-DESIGN §5.2 describes refusing the same edit only as "a re-run on the same source is refused as identical", which is about a finished attempt. It says nothing about an interrupted override, so this is a real gap, not a deliberate choice.

**The fix is safe**
- Both refusals run under the writer lock (hand_edit.rs:27), so any in-progress human record seen there is an orphan with no live writer.
- `reset_unfinished` itself refuses to touch a finished record.

**Fix (corrected).** 1. In crates/harness-llm/src/migrate.rs, `record_human_attempt`, replace the check at 657-661, `if attempts::attempt_dir(..).exists() { refuse }`, with a load of the record in that dir. Refuse only when the record exists and is finished; an in-progress record means an interrupted override and should be started over:
```rust
let dir = attempts::attempt_dir(&ledger, &unit.id, &id);
match AttemptRecord::load(&dir) {
    Ok(r) if r.outcome != IN_PROGRESS => return Err(already recorded),
    Err(e) if !e.is_not_found() => return Err(e),
    _ => {} // none, or an interrupted override
}
```
You could instead make `trajectory::load_record` `pub(crate)` and call it; it also checks that the stored id matches.

2. After `prepare_dir` (line 663), call `reset_unfinished(&work_dir, &id)?;`. It is already imported at line 53. It removes the stale `candidate/` and judge records, and it refuses a finished record.

3. In crates/harness-cli/src/hand_edit.rs:42-45, also `continue` when `rec.outcome == "in-progress"`, so the identical-source refusal only compares against finished attempts.

**Regression test (migrate.rs)**
1. Compute `id = attempts::attempt_id(UNIT, us, drv, "human", "-", &human_edit_hash(LOGIC, FFI))`, taking `us` and `drv` from `attempts::current_binding`.
2. Write `attempts/<id>/attempt.json` as an in-progress human record with no turns, plus `candidate/src/logic.rs`.
3. Call `record_human_attempt(&oracle(vec![green()]), …, &edit(LOGIC))`.
4. Expect `Ok`, outcome green, the same id, and exactly one attempt dir. Today it returns `Err` "already recorded".

A CLI test in steer_override.rs should run the same setup through `harness override` and expect exit 0, not "identical to attempt … (in-progress)".

**Regression test.** In migrate.rs: compute `id = attempts::attempt_id(UNIT, us, drv, "human", "-", &human_edit_hash(LOGIC, FFI))`, write attempts/<id>/attempt.json as an in-progress human record with no turns plus a candidate/src/logic.rs, then call `record_human_attempt(&oracle(vec![green()]), …, &edit(LOGIC))`. Expect Ok with outcome green; today it returns Err "this edit is already recorded".

## BENCH-M1 — The §7-required test of bench's human-provenance PROBLEM (and the replay skip) is missing

Severity: low (claimed medium).

**Claim.** TUI-DESIGN §7 requires `bench`'s human-provenance PROBLEM, as a unit test on a synthetic case. No test covers the Provenance::Human arm in score_one or the `skipped (human)` path in replay_all. A regression that lets a human-promoted crate score as strict-pass would pass the whole suite.

**Evidence.** crates/harness-cli/src/bench.rs:673-692 (the Human problem) and 1258-1267 (the replay skip) are not reached by any test. bench.rs `mod tests` (1420+) covers only supersession, tally, sample_of and superseding_sample. crates/harness-cli/tests/steer_override.rs:317-333 promotes a human attempt and checks `state status`, but never runs `bench score`/`check`. The provenance unit tests in harness-core check the enum only, not bench's reaction to it. Scenario: someone reorders the match so that `Provenance::Human(r) => Some(*r)`, or drops the Human arm into `_ =>` (unknown text); cargo test stays green.

**Verifier.** Confirmed against the code. docs/TUI-DESIGN.md §7 (lines 272-285) lists "`bench`'s human-provenance PROBLEM (unit test on a synthetic case)" as a required CLI test, and §8 step 2 makes "the CLI tests above" part of this step's deliverable. None of the tests exercise it:
- The Human problem is only in score_one (crates/harness-cli/src/bench.rs:673-691; the Human arm is at 678-682). Its only caller is bench.rs:852.
- The replay skip is only in replay_all (bench.rs:1258-1267). Its only caller is bench.rs:1011.
- bench.rs `mod tests` has four tests: supersession_entries_are_checked_strictly, excused_vectors_are_tallied_apart, sample_numbers and only_later_external_samples_supersede. None touches provenance.
- No file under crates/harness-cli/tests mentions `bench` (grep finds nothing). steer_override.rs:317-333 promotes the human attempt and checks only `state status`.
- harness-core's provenance_is_the_one_r5_rule (attempts.rs:639-713) tests the enum outcomes only, not how bench reacts to them.

So the reviewer's scenario holds. If `Provenance::Human(r)` at bench.rs:628 became `Some(*r)`, or the Human arm at 678 were folded into `_ =>`, cargo test would still pass. The first change would let a human-promoted crate score as pipeline strict-pass and pass `--write`. Deleting the HUMAN_KIND skip at 1258 would also go unnoticed.

Why low and not medium: the code is correct today. Verified plus Human pushes the "not pipeline provenance" problem, those problems reach the --write gate through score_all (bench.rs:888), and the replay skip is in place. No user or client hits a wrong behaviour now. This is a missing regression guard that the design mandates, on a trust invariant (the benchmark never counts a hand edit), not a shipped bug.

The proposed regression test needs one correction. The ambiguous text is not pushed in the Verified block. It is pushed earlier (bench.rs:617-627) whatever the verification state, and the Verified block's Ambiguous arm (684) pushes nothing. A helper that covers only the 673-691 block must return nothing for Ambiguous. The reviewer's "Verified plus Ambiguous gives only the ambiguous text" is true only if the helper also absorbs the earlier match.

**Fix (corrected).** Make both decisions pure and unit-test them in bench.rs `mod tests` on synthetic AttemptRecords.

(1) Move the body of bench.rs:673-691 into `fn verified_provenance_problem(case_path: &str, verification: Verification, provenance: &attempts::Provenance) -> Option<String>`:
- Verified plus Human(r) returns the "promoted from human (override) attempt {id} — not pipeline provenance" text.
- Verified plus None returns the "provenance unknown" text.
- Verified plus Ambiguous or Pipeline returns None, because the ambiguous problem is already pushed at 617-627.
- Any non-Verified state returns None.
score_one then calls `problems.extend(verified_provenance_problem(&case.path, verification, &provenance))`.

(2) Move the skip chain at bench.rs:1243-1280 into `fn replay_skip(rec, &records, stage, unit_source, driver_now) -> Option<String>`. It returns "bound to superseded inputs", "human" or "superseded by sample …", checked in that order, and replay_all prints and inserts `ReplayResult::Skipped` from it.

Regression tests:
(a) human_provenance_is_a_problem: build a green record with provider_kind = HUMAN_KIND, candidate_digest "blake3:c", and matching source and driver. Pass it through `attempts::provenance`, then check:
- Verified gives Some(text) containing "not pipeline provenance" and the id.
- Stale gives None.
- A crate digest no record matches gives Provenance::None, and Verified then gives "provenance unknown".
- Adding a model record with the same digest gives Pipeline, and Verified then gives None.
(b) human_attempts_are_skipped_not_replayed: `replay_skip` on a current-input human record returns Some("human"). The same human record with a stale driver returns Some("bound to superseded inputs").

Both tests fail if Human is mapped to Some(*r) at 628, folded into `_ =>` at 678, or if the HUMAN_KIND check is dropped.

**Regression test.** #[test] human_provenance_is_a_problem: Verified plus Provenance::Human(&human_rec) gives exactly one problem containing 'not pipeline provenance'; Stale plus Human gives none; Verified plus Ambiguous gives only the ambiguous text; Verified plus None gives 'provenance unknown'. The test fails if Human is mapped to Some or folded into the unknown text.

## CLI-M3 — The TUI hand-edit flow (§4 `e`) is refused whenever the editor leaves a backup file in src/

Severity: low (claimed medium).

**Claim.** `override` refuses any `src/` entry other than logic.rs, ffi.rs and an identical lib.rs. TUI-DESIGN §4 has the user edit the files in place in the directory it then hands to `override`. Editors routinely leave files next to what they save: Emacs's default `make-backup-files` writes a persistent `logic.rs~` on first save and `#logic.rs#` autosaves, and vim with `set backup` leaves `logic.rs~`. So a user of those editors cannot record a hand edit through the cockpit at all.

**Evidence.** crates/harness-cli/src/hand_edit.rs:148-165: every other name ends in `bail!("src/{} is not accepted: a hand edit is exactly src/logic.rs and src/ffi.rs")`. docs/TUI-DESIGN.md:155 (`e` row): it copies the two files into a fresh temp dir, runs `sh -c '$EDITOR "$@"' -- <dir>/src/logic.rs <dir>/src/ffi.rs`, then spawns `override <unit> <dir>` on that same dir. Scenario: EDITOR=emacs, the user edits and saves logic.rs, leaving `<dir>/src/logic.rs~`. The TUI spawns `harness --json override u <dir> --target …`, gets 'src/logic.rs~ is not accepted', exit 1, and the edit is lost to the ledger. Retrying after re-editing hits the same refusal.

**Verifier.** The claim is correct, but the defect is in the TUI design, not in the code under review.

What the code does: crates/harness-cli/src/hand_edit.rs:147-163 lists src/ and bails on every name other than logic.rs, ffi.rs and a lib.rs equal to CANDIDATE_LIB_RS. That is what docs/TUI-DESIGN.md §5.2 (lines 235-236) requires: "refuses (exit 1) when <DIR>/src holds any other file". It is also the resolution of LEDGER-H4. So steps 1-2 follow the design and are not wrong.

Where it breaks: docs/TUI-DESIGN.md:155, the `e` row. The editor runs on <dir>/src/logic.rs and <dir>/src/ffi.rs, and then `override <unit> <dir>` is spawned on that same dir. Nothing in §4 or §R handles files the editor leaves beside the two it saved.

I tested this on this machine with /usr/bin/vim and `set backup`, on a dir from `mktemp -d` under $TMPDIR (where a std::env::temp_dir-based TUI would put it). Vim wrote src/logic.rs~ next to logic.rs. Vim's default backupskip (`/private/tmp/*,$TMPDIR/*`) does not stop it: the file is reached through the /var -> /private/var symlink, so the pattern does not match. Without backupskip, a file under /private/tmp got no backup. Scenario:
1. The user has `set backup`, presses `e`, edits and saves logic.rs.
2. The TUI spawns `harness --json override u <dir> --target …`.
3. It exits 1 with "src/logic.rs~ is not accepted".
4. Every retry copies into a new fresh dir and hits the same error, so these users can never record a hand edit through the cockpit.

The Emacs part of the claim is plausible but I could not check it: emacs is not installed. Emacs's normal-backup-enable-predicate skips files in temporary-file-directory, and whether the macOS symlink defeats that check too is unverified.

Why low and not medium: the TUI is step 4 and has not been built, so no user or client can hit this today. The CLI's refusal is clear and does not damage anything. Only editors set to leave artefacts that outlive a clean exit are affected; a default vim (writebackup only, swap file removed on exit), nano and `code --wait` are not. The real work is a one-row amendment to the design before step 4 starts.

The CLI-side alternative, a list of ignored editor-artefact names in hand_edit.rs, should be rejected. It weakens the strict rule that §5.2 and LEDGER-H4 chose, and it is an open-ended list of patterns. The consumer should fix it.

**Fix (corrected).** Make no CLI change. Amend docs/TUI-DESIGN.md §4, the `e` row (line 155), and implement it in step 4:
- The editor runs on `<tmp>/edit/logic.rs` and `<tmp>/edit/ffi.rs`, which is where the before/after hashes are taken.
- After the editor exits 0 with changed hashes, the TUI copies exactly those two files into a new, empty `<tmp>/stage/src/`.
- It spawns `override <unit> <tmp>/stage`, and the argv it shows is the one it spawns.
- It removes `<tmp>` only after the spawned command has been reaped, so a refused override does not lose the edit.

Add a §R row noting the change: editor artefacts such as `*~`, `.*.swp` and `#*#` never reach DIR.

Regression test (step-4 actions test): set $EDITOR to a shell script that appends to logic.rs and also creates `logic.rs~` and `.logic.rs.swp` next to it. Run the `e` flow. Assert that the DIR in the spawned argv has a src/ containing exactly {logic.rs, ffi.rs}, and that `harness override` on it exits 0. This fails with the current design, where DIR is the edited dir itself.

**Regression test.** If the fix is CLI-side: in steer_override.rs, write `edit/src/logic.rs~` next to a real edit and expect override exit 0 with the artefact named on stderr. It currently exits 1. If the fix is TUI-side (step 4): an actions test where a fake $EDITOR (a shell script) modifies logic.rs and creates logic.rs~, and the spawned override argv's DIR contains only src/logic.rs and src/ffi.rs.
