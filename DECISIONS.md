# Harness Engineering Decisions

Running log, newest last. Each entry: decision, rationale, alternatives rejected, revisit-when trigger.

## 2026-09-17 — M0 kickoff: research spike (§15)

Time-boxed landscape check (run in a cheap-tier subagent, sources verified live):

- **google/zopfli**: archived by Google 2025-10-14, read-only, Apache-2.0, last release 1.0.3
  (2019). *Implication:* frozen upstream is a feature for us — a stable, reproducible target.
  https://github.com/google/zopfli
- **tree-sitter**: 0.27.0 (Aug 2026), active. Grammars expose `LANGUAGE: LanguageFn`;
  `set_language(&Language)` takes a reference; callers need `.into()`. tree-sitter-c 0.24.x.
- **c2rust (Immunant)**: actively maintained (v0.21, Oct 2025). Reference point for
  rule-based translation, not a dependency.
- **DARPA TRACTOR public corpus**: available —
  https://github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus/ (Battery 01; P00 Perlin
  noise, P01 SPHINCS; new battery every 6 months). Target for M4.
- **SOTA systems to re-check at M2+**: Syzygy (dual code+test translation, ICSE LLM4Code
  2025), EvoC2Rust (skeleton-guided, ICSE 2026 SEIP), ORBIT, His2Trans.

**Revisit when:** M2 observer design starts (re-check translation-system literature), M4
benchmark setup (re-check TRACTOR battery status).

## 2026-09-17 — M0 target: Zopfli, unit: katajainen.c

- **Target library:** google/zopfli, vendored at commit
  `ccf9f0588d4a4509cb1040310ec122243e670ee6` (2024-04-11, upstream HEAD at archive time)
  under `targets/zopfli/`. The vendored copy's `.git` was removed so this repo can track
  the migration ledger and Rust units inside the target tree; provenance is the pinned
  hash above (re-fetchable; verify by re-cloning and diffing).
- **Leaf unit:** `katajainen.c` — single public symbol `ZopfliLengthLimitedCodeLengths`,
  callees are only libc (`malloc`/`free`/`qsort`) and same-file statics. Narrowest real
  interface in the codebase; pure-function semantics (no globals, no I/O).
- **Oracle (exact command):** `cargo run -p harness-m0 -- oracle` from the repo root. It:
  1. builds a differential driver (`driver.c`) twice — linked against original
     `katajainen.c` and against the Rust staticlib — runs both over ~500 deterministic
     generated cases, and byte-diffs stdout;
  2. builds the full `zopfli` binary twice (all-C vs. mixed C/Rust with `katajainen.c`
     replaced) and byte-compares compressed output over sample inputs;
  3. additionally runs the C-side driver under ASan+UBSan.
- **M0 scan:** `cargo run -p harness-m0 -- scan` parses `src/zopfli/*.c` with
  tree-sitter, builds a function-level call graph, and ranks leaf units. Known gap
  (recorded deliberately): calls made through function pointers (e.g. the qsort
  comparator) are invisible to naive call extraction — this is exactly gotcha-taxonomy
  material for M2 detectors.

## 2026-09-17 — M0 scope reductions (deliberate, to be lifted at M1+)

- **Sandboxing (§12.2):** M0 runs builds/tests as plain subprocesses (explicit argv, no
  shell, paths confined to the checkout). Containerized/network-disabled sandboxing is
  deferred to M1. Revisit when: first LLM-generated code runs without a human reading it
  first.
- **No `facts.db` yet:** M0 state is ad-hoc files under `targets/zopfli/migration/`.
  M1 promotes to the neutral schema after the §14.1 storage spike.
- **Single-crate-per-purpose workspace shape (§10.1) deferred:** M0 is `harness-m0`
  (one binary) + the unit crate. M1 splits into `harness-core`/`-scan`/etc.

## 2026-09-17 — Dependencies added (§11.1 justifications)

- `tree-sitter` 0.27 + `tree-sitter-c` 0.24: approved baseline (§11.2) parsing path;
  chosen over libclang for M0 because it needs no `compile_commands.json` and has no
  system-library setup cost. Revisit at M1: libclang gives real type/ABI facts that
  tree-sitter cannot; the scanner frontend trait must not assume either.
- No other non-std dependencies in M0. `clap` deliberately skipped (two subcommands,
  plain `std::env::args` suffices at this size).

## 2026-09-17 — M0 complete; state at session end (§17 handoff)

**M0 is done.** `cargo run -p harness-m0 -- oracle` is GREEN (5/5 checks): 500-case
differential driver byte-identical, whole-program zopfli (mixed C/Rust link)
byte-identical gzip on 3 samples, ASan+UBSan clean. Unit u001-katajainen is
`verified`; zero human-written Rust. Quality gates pass: fmt, clippy -D warnings,
cargo test.

Two real findings the oracle produced (details in targets/zopfli/migration/DECISIONS.md):
1. Implicit contract `maxbits ≤ 15` — C baseline SIGBUSes beyond it (`counts[16]`).
2. C qsort comparator is not a total order for frequencies ≥ 2^22 (UB per C11
   7.22.5p4); Rust port uses the true total order, differential domain constrained.

**Next actions (M1):** state-backend research spike (§14.1) → freeze `facts.db` +
`plan.yaml` schemas → promote m0/ into the §10.1 workspace shape
(harness-core/-scan/-detect/-llm/-oracle/-cli) → multi-unit ordering. Also carried
forward from M0: function-pointer calls invisible to the scanner (M2 detector
material); CI for the §10.2/§11.3 gates not yet set up.

## 2026-09-17 — M1 state-backend research spike (§14.1) — findings and decisions

Five parallel web-research passes (embedded stores, git-native state, content-addressed
storage, runtime conventions, plan-file format), all claims source-verified as of Sept
2026. Full transcripts in the session workflow logs; condensed findings:

- **Binary SQLite in git is an anti-pattern** (no diffs, unmergeable, byte-
  nondeterministic pages). Projects that commit DBs rely on textconv/`.dump` hacks.
  The beads project (Yegge) is the cautionary case study: JSONL-in-git ledger hit
  rebase conflicts from non-idempotent writers and sequential-ID collisions across
  branches, then retreated to a derived-export model.
- **rusqlite 0.40.2** requires system SQLite ≥ 3.45.3; Ubuntu 24.04 LTS ships 3.45.1
  and macOS lags — so when we ship the facts.db export, `bundled` is justified
  (identical behavior everywhere; this answers §11.2's "justify bundling").
  redb 4.3 is credible but KV-only; **sled is still not shippable** (no stable
  release since 2021, README says use SQLite).
- **serde_yaml is dead** (archived 2024-03); its popular successor serde_yml was
  flagged AI-generated/unsound with a security advisory. TOML tooling is at peak
  health (toml 1.x, toml_edit format-preserving edits — cargo-adjacent maintainers).
- **Committed plain files beat git notes/refs** for reviewable tool state
  (git-appraise dormant; Gerrit NoteDb works only behind a daemon; the 2025-26 agent
  tool wave — spec-kit, OpenSpec, Backlog.md — all chose plain files). Proven
  patterns: file-per-record, canonical serialization, content-derived IDs, regenerate-
  don't-hand-merge for derived files, worktrees for parallel builds.
- **CAS shape** (for when artifacts exist): in-repo `cas/<2hex>/<blake3-hex>`, atomic
  temp+rename writes, mark-and-sweep GC from unit-record roots — never mtime-LRU for
  tracked files (git destroys mtimes). blake3 is the incumbent in this niche
  (ccache, Buck2). Design recorded; no code until the LLM loop produces artifacts.
- **Runtime view** (§14.3, for M2): one small generated managed block in AGENTS.md
  (BEGIN/END markers + content hash), CLAUDE.md bridging via `@AGENTS.md`; expose
  state via CLI/MCP *tools*, never MCP resources (~28% client support, Claude Code
  effectively doesn't consume them). Evidence that context files must carry only
  non-derivable info (ETH study: verbose/derivable content costs ~20% and *lowers*
  success) — ledger state qualifies.

**Decisions** (schemas frozen in docs/SCHEMAS.md, reviewed by a 4-lens adversarial
design panel whose two blockers — unbound verdicts, unspecified staleness — are fixed
in the spec):

1. Canonical fact model = deterministic sorted **facts.jsonl, committed**; SQLite
   facts.db becomes a derived gitignored export, **deferred to its first query
   consumer (M2)** with its SQL schema frozen on paper. Deviation from the briefing's
   "facts.db (SQLite)" wording, sanctioned by §14.1's own spike clause; cold-resume
   is strengthened, not weakened (text is diffable/mergeable and regenerable).
2. **plan.toml, not plan.yaml** (§11.2 invited this; the YAML ecosystem rupture
   decides it). Reconciliation semantics, writer model, advisory ordering, and closed
   enums per SCHEMAS.md.
3. **Verdicts are content-bound**: blake3 digests of every oracle input; `harness
   state status` is the staleness detector; `verify` refuses on stale plans and
   demotes status on red. No timestamps in committed canonical files.
4. **Unit crates leave the harness workspace** (`[workspace] exclude` targets/);
   the oracle builds them via `--manifest-path`, so a broken in-progress unit can
   never brick the harness's own tooling on a fresh clone.
5. Traits at M1: `LanguageFrontend`, `OracleStrategy` (now threaded with a
   `TargetContext` so implementations receive config). `PlannerStrategy` demoted to
   a plain function until a second planner exists. `Detector`/`ProviderAdapter`
   defined at M2/M3.
6. Symbol identity is frontend-canonical (`file::name` for C statics) — the fact
   model no longer assumes C's global-uniqueness of external names.

**Dependencies added (§11.1):** `serde`/`serde_json` (canonical JSONL), `toml` +
`toml_edit` (plan read + surgical mutation — the only mainstream format-preserving
editor), `blake3` (committed content hashes need a stable keyed-nowhere hash; std
hashers are per-process-random and release-unstable), `thiserror` (core/scan/oracle
typed errors, §10.2), `clap` + `anyhow` (harness-cli — reverses M0's no-clap note:
the CLI now has four subcommands with flags and §13.4 makes it a stable documented
interface; threshold genuinely crossed). **proptest deferred** (§10.2 pushback,
recorded: M1 ships seeded-PRNG round-trip/idempotence tests + a golden byte fixture;
proptest lands when hand-edited plan files warrant fuzzing). **rusqlite deferred**
to the facts.db export milestone.

**Revisit when:** facts exceed ~10^5 records or cross-unit queries appear (facts.db
export + rusqlite); a second planner strategy appears (re-promote the trait); YAML
interop is ever demanded (serde-saphyr, never serde_yml).

## 2026-09-17 — M1 complete; verification-review findings; state at session end (§17 handoff)

**M1 is done.** Workspace promoted to crates/{harness-core,-scan,-oracle,-cli}; ledger
schemas v1 frozen in docs/SCHEMAS.md and implemented; zopfli plan holds 11 units in
dependency order (one genuine 3-file SCC merged: blocksplitter/deflate/squeeze);
u001-katajainen re-verified GREEN under the content-bound verdict regime. Gates green:
fmt, clippy -D warnings, 18 tests incl. an e2e that runs the full pipeline plus a red
oracle run in a temp copy. CI authored (macOS+Ubuntu + cargo-deny). m0/ deleted.

**Pre-commit adversarial review (4 lenses) found and fixed — all with regression
coverage where testable:**
1. *Split-cycle plan corruption* (blocker): adopt_existing_ids let two clusters claim
   one existing id → plan corrupted on disk. Fixed: claim-once + self-dep drop +
   duplicate-id rejection + `harness plan` validates the reconciled document BEFORE
   writing.
2. *Stem-collision unit loss* (blocker): u-util from src/x and src/y collided and one
   unit silently vanished. Fixed: path-slug fallback ids + hard duplicate check.
3. *Candidate crash ≠ red* (blocker): a crashing/non-building Rust candidate exited 1
   with stale green evidence left standing. Fixed: crate build failures and run
   crashes are now failed checks in a red verdict (exit 10, demotion, last-green kept).
4. *`-lm` before objects* (blocker): whole-program link would fail on Ubuntu
   (--as-needed). Fixed: link args after inputs (cflags/libs split).
5. *CARGO_TARGET_DIR false-green* (blocker): redirected builds left a stale staticlib
   where find_staticlib looks. Fixed: explicit --target-dir.
6. Should-fixes applied: rust_crate digest is now a CLOSED list (Cargo.toml +
   Cargo.lock + src/**, spec amended); atomic writes everywhere (temp+rename);
   structural plan validation on every CLI load; contradiction detection in both
   directions incl. merged; red-on-merged warns without auto-demote; stale-refusal
   recovery advice corrected (scan→plan→verify) and `plan` refuses stale facts;
   status distinguishes missing/unreadable/newer-schema verdicts; EPIPE-safe stdout;
   exit codes 0/1/2/10 all pinned by e2e; SCHEMAS example allowlist includes rustc;
   CI runs the verified unit crate's clippy+tests via --manifest-path (interim until
   the harness drives per-unit gates itself).

**Next actions (M2 — observer):** research spike (translation-literature re-check per
§15); Detector trait + built-in detectors from the §6 gotcha taxonomy (function-
pointer calls are already a known scanner gap); LLM triage pass writing
observations.md; `harness sync-runtime` (§14.3 design already recorded, spike gave the
exact pattern); consider harness-driven per-unit crate gates replacing the CI interim.
Carried forward: facts.db SQLite export deferred to first query consumer; proptest
deferred; [state] profiles config deferred until artifacts exist.

## 2026-09-17 — M2 research spike (§15) — findings and decisions

Three parallel source-verified sweeps (hazard-detection landscape, translation
literature, LLM triage hardening). Condensed:

- **Tree-sitter honestly covers 8/12 taxonomy categories** (Semgrep's GA C support —
  pre-expansion tree-sitter/GLR — validates the substrate). Pointer arithmetic, type
  punning, aliasing, and indirect-call resolution genuinely need type info: recorded
  as libclang-frontend material, carried as standing caveats in observations.md.
  clang-tidy's check list is the M3+ reference oracle; weggli (dormant) is the
  architectural cousin; Coccinelle rule idioms noted.
- **TRACTOR Round-1 report (MIT-LL, Feb 2026)**: six performers, 48–98.7% functional
  correctness; failures dominated by *semantic comparison* not compilation —
  vindicating the oracle-is-the-product principle. Unsafe residue: raw-pointer
  deref/arith > mutable statics > unions. Macros/conditional compilation are the
  cross-performer pain point. Battery difficulty is staged; harder batteries every
  6 months. **No published per-unit risk score exists** — ours is novel; TRACTOR
  per-test failure data is the M4 calibration set.
- **Risk signals (evidence-ranked)**: pointer-role complexity, oracle availability
  (deferred — no coverage data yet), size×coupling (degradation >50 LoC; SCC as
  blast radius), UB/impl-defined flags (most-failed TRACTOR tests), macro density +
  mutable globals. Threading/setjmp/signal = binary blockers, not points.
- **LLM triage**: production systems (Semgrep Assistant >95% agreement, Copilot
  Autofix, Datadog) all ship ADVISORY triage with human review; agentic triage can
  suppress 22% of true positives → asymmetric authority is mandatory. Spotlighting/
  datamarking cuts injection success >50%→<2%; frontier models resist comment-based
  attacks (Feb 2026, 9,366 trials) so slices keep comments. ≤10 findings/call,
  ID-keyed verdicts, rationale-before-verdict field order.

**Design decisions** (schemas in docs/SCHEMAS.md "M2 additions"; 3-lens adversarial
review found 8 blockers, all fixed in the spec):

1. Finding ids are content-keyed (spanned-source hash + occurrence index) — the beads
   id-churn failure mode, avoided a second time. Verdicts keyed by finding id alone;
   shared-header findings triaged once, joined per-unit at render.
2. findings.jsonl is a pure function of (tree, detector suite): freshness-bound to
   facts via header hash + per-file hashes; observe refuses on staleness (M1 refusal
   pattern). Unit attribution + risk computed at render time, never committed.
3. Behavior travels on records (`blocker`, `human_mandatory` flags) — open category
   enum stays fail-safe for unknown categories.
4. Human review surface: reviews.jsonl via `harness review`; dismissals keep full
   risk weight until a human uphold exists. annotations.jsonl ingests oracle/human
   findings (M0's comparator-UB + maxbits-contract are the founding entries).
5. Injection posture: nonce-delimited untrusted regions, `<` escaped, no source-
   derived text in the trusted prompt region, harness-computed content hashes,
   response schema validation. Order: RNG-free blake3 sort (determinism chosen over
   position-bias mitigation; recorded tradeoff).
6. ProviderAdapter trait = name + complete only (Capabilities deferred to first
   consumer, the facts.db precedent). Adapters: anthropic (ureq/rustls, key from env,
   never persisted), trace-based replay/external (one trace format = record = replay
   = external hand-off; external mode is how a driving agent runtime supplies triage
   without an API key — provider-agnostic by construction, and M3's second live
   adapter plugs into the same seam).
7. Risk formula v1: capped weighted sum, fixed absolute caps, blocker pinning ≥90.
   Weights are the M4 calibration hypothesis, recorded not sacred.
8. Model default claude-sonnet-5 (briefing §16 Tier-2 for triage); config-overridable.

**Deps added (§11.1):** `ureq` (approved baseline §11.2, rustls TLS, blocking —
sync-first policy §10.3 upheld; no tokio). No other additions.

**Revisit when:** libclang frontend lands (pointer/punning/aliasing detectors);
M4 TRACTOR calibration (risk weights); per-unit test coverage exists (oracle-
availability signal); canary injection set (recorded as future hardening).

## 2026-09-17 — Risk score v1 normalization caps (normative for scoring stability)

Fixed absolute caps (changing any is a schema-visible scoring change): signature
pointer density 40 `*`s → 25 pts; LoC 2000 → 15; SCC files 5 → 5; fan-in 10 → 5;
dependency depth 5 → 5; UB/impl-defined findings (all annotations count here, plus
bitfield/union/ub-reliance/impl-defined/impl-contract categories) 5 → 20; macro +
global-mutable findings 10 → 15; alloc-ownership findings 10 → 10. Sum = 100; each
signal appears in exactly one term (the M2 review caught an alloc double-count that
summed to 110 — fixed before any score was committed to main). Any `blocker` finding
pins the unit at ≥ 90. Dismissed findings keep full weight until a human
`uphold-dismiss` review exists.

## 2026-09-17 — M2 complete; verification-review findings; state at session end (§17 handoff)

**M2 is done.** Observer stage shipped: `harness-detect` (c-treesitter-v1 suite, 8
taxonomy groups, content-keyed findings freshness-bound to facts), `harness-llm`
(ProviderAdapter trait; `anthropic` live adapter via ureq/rustls, `replay`/`external`
trace adapters — one trace format), the triage pass with its injection posture,
deterministic risk scoring, `observations.md` rendering, the human review loop
(`harness review`), annotations for oracle/human findings, and `harness sync-runtime`
(§14.3). Zopfli run: 31 findings + 2 founding annotations, all 31 triaged (24 confirm,
5 dismiss, 1 uncertain — via the external provider path, since this environment has no
API key), 11 units risk-ranked; the 3-file SCC tops at 51. Gates: fmt, clippy -D
warnings, 50 tests incl. e2e covering detect/observe/review/sync-runtime and all M2
refusal exit codes. Stage-2 done-criteria (every finding triaged; units ranked) are
enforced by the renderer, not by convention.

**Pre-commit review (4 lenses) found and fixed:** 2 blockers — `webpki-roots`'s
CDLA-Permissive-2.0 license missing from deny.toml (CI would have failed) and
`sync-runtime` silently deleting human prose on a corrupted marker (now a hard error);
plus: alloc double-counted in the risk formula (weights summed to 110 — fixed to 100),
annotations not feeding the UB signal, `sync-runtime` swallowing newer-schema refusals,
retry traces recorded under the wrong key (replay would fail), unvalidated finding
fields reaching the trusted prompt region, model rationale able to forge markdown
structure, `api_key_env` exfiltration via hostile harness.toml (now must start with
`ANTHROPIC_`), detector gaps (const-qualifier level on globals, missing
`sigsetjmp`/`_longjmp`/`cnd_`, unnamed fn-pointer params, typedef chains, fn-pointer
struct fields, `__attribute__`-defeated pointer-return heuristic, macro span
off-by-one), and spec/impl drift on trace keys and per-group identity bytes (spec
amended). All with regression tests.

**Live-call status:** the Anthropic adapter is implemented per the current API
(x-api-key + anthropic-version 2023-06-01, no temperature/thinking params, stop_reason
gating, 1 retry on 429/5xx) but has NOT been exercised against the live API in this
environment (no key). First live run should be done deliberately with
`provider = "anthropic"` and its trace pair inspected; it will produce byte-identical
triage.jsonl on replay by construction.

**Next actions (M3 — provider adapter #2):** second live adapter (any OpenAI-compatible
or local endpoint) behind the same trait with zero changes outside harness-llm +
config; prove the same triage run through both; record/replay determinism across
providers. Carry-forwards: canary injection set for the triage prompt (recorded as
future hardening); `state status` does not yet report observer freshness (observe
refuses instead — acceptable, documented); per-unit oracle-availability risk signal
awaits per-unit tests; facts.db export + proptest still deferred; libclang frontend
for the 4 undetectable categories.
