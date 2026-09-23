# Evidence-first replay and the prompt lock — design

Status: **approved direction (user, 2026-09-23); reviewed (4 lenses, all sound-with-fixes).
§R below is AUTHORITATIVE and supersedes §1–§11 where they conflict.**
Background and the ranked alternatives: DECISIONS.md "PROPOSAL … prompt edits vs
recorded-trace replay".

## 1. What changes, in one paragraph

Replay today re-renders every turn's request from HEAD and looks the recorded response up
by the re-rendered key, so one edited prompt sentence orphans every recorded attempt.
Replay is split into two tiers. **Strict (evidence) replay** reads each turn's RECORDED
request/response pair by the key the attempt recorded, checks the bytes are intact, and
re-runs today's parser and judge (oracle / driver validation) on the recorded replies: the
per-turn results, the candidate and the outcome must reproduce. **Conformance** compares
what HEAD would send with what was recorded and names the sections that differ — reported,
never a replay failure. Prompt text itself is guarded by committed **prompt fixtures**
(snapshot files `cargo test` compares against), so every prompt edit is a reviewed diff.

## 2. Strict tier (normative)

For a FINISHED attempt `A` whose `unit_source` and `driver` equal the current tree's
(otherwise: `skipped (bound to superseded inputs)`, as today):
1. **Integrity.** For each recorded turn `i`: load `<traces>/<key_i>.request.json` and
   `.response.json` (the sample's own trace dir when it has one, else the root); the
   request must re-serialize (compact JSON) to a blake3 whose first 8 hex are `key_i` and,
   when the turn records `request_hash` (new, full digest), equal it; turn 1 must also
   match `A.prompt_digest`; the response text must hash to `response_hash_i`; the request's
   `model` must equal `A.model`. The attempt id must re-derive from `A`'s recorded fields
   and turn-1 key (the stage's frozen derivation; `.rN` suffix preserved).
2. **Re-judge.** Drive the trajectory with the RECORDED request/response of each turn
   (never sending anything), parsing and judging with HEAD code in the scratch dir (path
   alias as today). Budget = recorded turn count.
3. **Compare** (unchanged list): turn count, per-turn `result`, `candidate_digest`,
   `outcome`. Request keys are no longer compared here (tier 1 checks integrity; tier 3
   reports drift).
Strict failure = the attempt "does not reproduce" (a PROBLEM in `bench check --replay`, an
error for `--provider replay`).

## 3. Conformance tier (reported)

While driving, HEAD also renders the request it WOULD send at each turn (turn 1 from the
tree; repair turns from the replayed state). Per turn: `conformant` (byte-identical to the
recorded request) or `drifted: <sections>` — the names of HEAD-rendered sections
(`[NAME]` blocks HEAD produced, so model text can never forge one) whose exact text does
not occur in the recorded request, plus `system` / `model` / `max_tokens` when those
differ. `bench check --replay` prints per attempt `strict: reproduces|expected-divergence|
FAILS` and `prompt: conformant|drifted(turn: sections…)`, and a total line.

## 4. Supersession: explicit, strict "expected divergence"

`migration/units/<unit>/superseded.jsonl` (new, append-only, one JSON object per line):
`{"schema":"ruharness-superseded","schema_version":1,"attempt","stage":"migrate"|"driver",
"divergences":[…exact strict-tier divergence strings…],"reason","superseded_by"}`.
A superseded attempt must diverge EXACTLY as recorded: reproducing fully, or diverging
differently, is a PROBLEM (strict xfail). Written by: (a) `--retry` under a trace-backed
provider when it records a new sample because the latest does not reproduce (reason:
`judge-changed`, `superseded_by` = the new sample); (b) `harness supersede <UNIT>
--attempt ID --reason TEXT [--stage driver] [--target]`, which evidence-replays the
attempt, refuses if it reproduces, and appends the observed divergences. The implicit
`.rN` rule in `bench check --replay` (post-M4) is REMOVED: a newer attempt never silently
excuses an older one. First entry: 014's `a-4adfe6d56ab0` (stdout-only oracle).

## 5. Running a stage: existing evidence wins; only HEAD's prompt sends

- **Only HEAD's renderer ever sends a request.** Verification only reads files. (Holds
  today; stated as an invariant with a test.)
- **Trace-backed providers (`external`), default run:** if the base id computed from HEAD's
  turn-1 request has no record, but the unit has a FINISHED attempt for this stage bound to
  the current `unit_source`/`driver`, `model` and provider kind, and not superseded, the run
  evidence-verifies that attempt (deterministic choice: conformant first, then lowest
  base id, then sample number) and returns it — zero tokens, with a note naming the
  prompt drift and `--new-trial`. A suite-wide re-run after a prompt edit therefore queues
  no hand-offs.
- **`--new-trial`** (new flag, `migrate` and `gen-driver`): always record a new attempt
  under HEAD's prompt — the HEAD base id if unused, else its next `.rN` sample. The older
  attempt is NOT superseded by this (different prompts are different trials, both valid
  evidence).
- `--retry` keeps its post-M4 meaning (re-sample only when the latest sample no longer
  reproduces — now under the strict tier) and additionally writes the supersession record.
- Live providers: unchanged (a finished attempt is refused without `--retry`/`--new-trial`).

## 6. Prompt fixtures (CI-enforced) replace "prompt bytes FROZEN"

- `crates/harness-llm/tests/prompt-fixtures/`: the exact rendered requests (system + user,
  as text) for fixed synthetic units per stage and turn kind (translate; repair after
  build, oracle, format; driver generate + repair), plus the REAL u001 translate request
  rendered from the zopfli tree (whose key and prompt_digest are first checked against its
  recorded attempt). A test renders each and compares byte for byte;
  `RUHARNESS_UPDATE_PROMPT_FIXTURES=1` rewrites them. Every prompt edit is thus a diff of
  these files in the same commit.
- `PROMPT_EDITION` = blake3 over the fixture files (sorted), exposed by the crate; new turns
  record nothing extra for it, but `attempt.json` gains optional `prompt_edition` for new
  attempts (display/grouping only).
- R11 is amended: the attempt-ID DERIVATION stays frozen (test: every committed attempt's
  id re-derives from its recorded turn-1 key); prompt BYTES are locked by fixtures, not
  frozen.

## 7. Scores per prompt

`scores.json` `cases[].pipeline.prompt` (optional): `conformant` | `drifted` — whether the
promoted migrate attempt's recorded turn-1 request equals HEAD's render. Totals gain
`verified_drifted`. A score is "HEAD's pipeline" only for conformant cases; the rest are
evidence recorded under an earlier edition (disclosed, never pooled silently).

## 8. Records

`Turn.request_hash` (optional, full `blake3:` digest of the canonical request JSON) for new
turns; `AttemptRecord.prompt_edition` (optional). Both additive, omitted when absent;
`schema_version` stays 1.

## 9. Migration (no rewritten files, zero tokens)

1. Land §2–§4, §8 with the renderer UNCHANGED: `bench check --replay` must report every
   bound attempt `strict: reproduces`, `prompt: conformant`, except 014's `a-4adfe6d56ab0`
   → recorded via `harness supersede` → `expected-divergence`.
2. Land §6 fixtures (renderer unchanged; fixtures = today's bytes), §5, §7.
3. Then, each its own commit with its fixture diff: the stderr sentence for every printing
   unit (delete the `names_stderr` workaround), the `[ABI CONTRACT]` fence. Replay stays
   strict-green; conformance shows exactly the intended section drift.

## 10. What replay proves afterwards — and what it no longer does

Proves: recorded bytes are intact and self-consistent; HEAD's parser and judge reach the
recorded results, candidate and outcome for every attempt, through any prompt edit;
supersessions are explicit and exact. No longer proves, for DRIFTED turns only: that HEAD
would have asked the same question. Consequence recorded in §7: scores of drifted cases are
evidence about an earlier prompt edition.

## 11. Not doing

Keeping old renderers (versioned templates: re-proves committed bytes, keeps old prompts
able to send), semantic/fuzzy matching, body-blind cassettes, re-keying old replies,
hosted registries, new crates.

## §R — Adversarial design review (4 lenses) and resolutions (AUTHORITATIVE)

Verdicts: evidence semantics, security, migration/contracts, simplicity — all
**sound-with-fixes**; 10 serious findings, verified against the code by a triage pass.
The confirmed defects that shaped the resolutions: verification can SEND a hand-off today
(a missing response file under the `external` adapter files a new request); both 014
attempts carry `promoted: true` and scoring picks the superseded one; the §5 fallback
binds only `unit_source`/`driver`, so evidence recorded WITHOUT a newly confirmed hazard or
a re-planned ABI line would be reused silently; the strict tier alone loses M4's
evidence-determinism net (repair-key comparison found three M4 bugs).

- **R-1 Deferred: §5 bullets 2–3 (default-run evidence reuse) and `--new-trial`.** A run
  keeps today's semantics: HEAD's base id unused → a new trial; used → strict
  verification. `--retry` applies only to HEAD's base. Unpinned `--provider replay` with no
  key match refuses and lists the finished attempts (use `--attempt`). A changed prompt is
  a new trial when someone explicitly runs the stage; nothing is reused silently.
- **R-2 Verification never touches a provider adapter.** Recorded pairs are read by a
  read-only loader: key must match `^[0-9a-f]{8}$` before any path join; files opened only
  as regular files (symlinks refused), size-capped; the request must re-serialize to its
  key; zero-turn finished records refused; every turn's integrity checked before judging.
  Test: the external adapter with a response file deleted — nothing created, no "awaiting".
- **R-3 Keep the evidence-determinism net.** HEAD still renders every turn; its key is
  compared with the recorded one. **If turn 1 is conformant, any repair-turn key drift is a
  strict failure** (today's detection, unchanged until a template changes). After a
  template change (turn 1 drifted), repair drift is reported only; the net is then held by
  (a) a determinism test (judge in two scratch dirs of different path lengths, byte-equal
  repair renders), (b) render invariants on every HEAD repair render (no scrub-list path;
  printable ASCII + `\n`; `[EVIDENCE]` lines `| `-quoted or a fixed lead-in), (c) the M4
  regression tests retargeted at HEAD's render.
- **R-4 Conformance = `conformant | drifted(turns …)`** per attempt plus totals; no
  section naming (forgeable; the fixture diff is the review surface).
- **R-5 The promoted attempt** = the unique finished-green attempt whose
  `candidate_digest` equals the unit crate's content hash; none or several → a PROBLEM.
  `bench` stops relying on the `promoted` flag (`migrate_turns`, `migrate_outcome`).
- **R-6 Deferred: §7** scores-per-prompt and `verified_drifted`.
- **R-7 Supersession** = hand-written `migration/units/<unit>/superseded.jsonl`, verified by
  `bench check --replay`: `{schema:"ruharness-superseded", schema_version:1, attempt,
  stage, reason, superseded_by, loosening?}`; no stored divergence strings. Checks: the
  superseded attempt does NOT strict-reproduce; its failure is a re-judge divergence (never
  an integrity failure or an error); a tightening (recorded green → replayed non-green)
  unless `"loosening": true` (always listed); `superseded_by` is finished, same unit,
  stage, `unit_source`, `driver`, and reproduces; unknown ids are a PROBLEM; the latest
  line for an attempt wins; reader refuses symlinks, validates ids, echoes printable. No
  CLI command, no auto-write. The implicit same-base `.rN` rule is KEPT (sound: `.rN` is
  created only after non-reproduction). First entry: `a-4adfe6d56ab0` → `a-bf33266e0112`.
- **R-8 Prompt fixtures by branch matrix** (migrate: printing unit naming stderr / not
  naming it / non-printing, non-empty hazards, repair after build / oracle / check /
  crash-timeout / format / format-after-parse, emission notes; driver: generate, repair),
  plus a guard test that every prompt constant occurs in some fixture. No real-u001
  fixture.
- **R-9 Golden test split**: (a) permanent binding test — u001's HEAD `unit_source`,
  `driver`, hazards equal the recorded ones and the id re-derives from the recorded turn-1
  key; (b) the render-equality part, retired visibly in the first prompt-edit commit.
- **R-10 Cut** `PROMPT_EDITION`, `prompt_edition`, `Turn.request_hash` (no records
  change). Integrity = corruption detection (32-bit legacy keys); authenticity rests on
  git review.
- **R-11 §9 step 0**: in-flight state first — float2half's in-progress attempt is left
  as is (a new trial after a prompt edit; the in-progress record is skipped by replay);
  no prompt edit may land between export and import of a hand-off round.
- **R-12** SCHEMAS.md/README: replay semantics, the unpinned-replay refusal,
  `superseded.jsonl` (schema + writer table), exit codes.

## §C — Code review (3 lenses) and fix pass (AUTHORITATIVE where it amends §R)

17 findings, all confirmed by a verifying triage, all fixed with regression tests (each
mutation-checked where it guards a rule): **R-3 amended** — the strict evidence rule
keys on WHAT differs, not on turn-1 conformance: a drifted repair turn is strict only
when it equals the recorded request once both `[EVIDENCE]` sections are blanked
(otherwise an edit to a repair-only template — `REPAIR_TASK`, an explanation — would
have failed every multi-turn attempt); **R-7 amended** — tightening = recorded green AND
replayed not green (the replayed outcome travels in `Diverged`); the successor of a
green attempt must be green; the scored artifact can never be superseded; legitimately
skipped attempts make an entry moot, not a problem; **driver-diff evidence is
path-scrubbed** (C printing `__FILE__`, or `/tmp` on Linux, would otherwise trip the
leak guard and stall a hand-off forever); R-5 counts only attempts bound to the current
inputs and reports ambiguity once; unpinned replay prefers bound attempts and marks
stale ones in its refusal; attempt ids must equal their directory names; at most 64
recorded turns and a 1 MiB `superseded.jsonl` are read; ids are echoed printable;
fixtures cover every prompt branch and every explanation constant (guard tests), and
a structural test keeps every `[EVIDENCE]` line quoted or a known lead-in (a runtime
ASCII-only rule was rejected: harness lead-ins legitimately contain `—`); a
determinism test poses the same trajectory under roots of different path lengths.
Known limit (recorded): u001's hazards are bound only through its turn-1 request,
whose traces are gitignored — once R-9(b) is retired, nothing re-checks them.
