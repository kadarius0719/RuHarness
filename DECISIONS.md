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

## 2026-09-19 — M3 research spike (§15) and design decisions

Three source-verified sweeps (second-provider landscape, translation/repair-loop
evidence, code-emission formats). Condensed:

- **Adapter #2 = OpenAI-compatible Chat Completions.** Still the universal lowest
  common denominator in Sept 2026 (OpenAI, Ollama, llama.cpp, vLLM, LM Studio, Groq,
  Together, OpenRouter, Mistral, Gemini-compat all serve it; the Responses API has not
  displaced it). The one unsafe field is the max-token NAME (`max_tokens` vs
  `max_completion_tokens`); sampling params must be omitted (reasoning models 400 on
  them); errors can arrive inside HTTP 200; ureq 3 has NO default timeouts.
- **Executor evidence:** 1 translate + ≤3 repairs (5th iteration ≈ 0% gain, PtrTrans/
  CRUST-Bench); stateless repair turns with the full current candidate; whole-file
  emission beats diffs at this size (aider: 99.6% vs 71.6% well-formed on a weak
  model); a `<blocked>` escape hatch cut cheating 54%→9% (ImpossibleBench); models
  special-case visible test inputs, so evidence must be bounded; safe-first with a
  mechanical FFI shim (ENCRUST pattern).
- **Environment:** no cloud API keys; Ollama 0.30.10 local with `llama3.2:1b`, which
  serves BOTH the OpenAI and the Anthropic wire formats. A derived 32k-context model
  (`llama3.2-1b-32k`, Modelfile `PARAMETER num_ctx 32768`; no download) avoids silent
  server-side prompt truncation (`ollama ps` confirmed CONTEXT 32768).

**Decisions** (spec: docs/SCHEMAS.md "M3 additions"; a 3-lens design review returned
"flawed" from the security lens — all three blockers fixed in the spec before code):

1. `harness migrate`: translate → parse → deny-scan → harness-owned scaffold → oracle →
   stateless repair, ≤3 repairs; attempts ledger with content-derived ids, per-turn
   atomic journaling, nullable token usage, `prompt_digest` so equal digests prove the
   same migration was posed to different providers.
2. Trust boundaries: target-owned files are hostile (plan path fields validated at
   load; `extra_link_args` is `-l<name>` only; endpoints/credentials live in USER-level
   provider profiles, never the target's harness.toml; LLM spend clamped). Model output
   is untrusted code: harness owns Cargo.toml + lib.rs so the COMPILER confines
   `unsafe` to ffi.rs; symbol-set check (incl. pre-main constructor sections) and the
   sandbox are the security boundaries; the deny-scan is quality feedback only.
3. Sandbox (M0's deferred item; its revisit trigger fired): `sandbox-exec` around
   every build and run, scrubbed env, wall-clock timeouts with process-group kill,
   home reads denied; on `sandbox: none` platforms every code-executing command
   refuses without `--allow-unsandboxed`.
4. Promotion: record-first, stage, two renames, in-place verify, evidence-aware
   recovery.
5. Cut per YAGNI (enum strings reserved so deferral costs no schema bump): held-out
   corpus + driver seeds, escalation tiers, budget enforcement, `harness usage`, thrash
   detection, retry-with-doubled-tokens, Linux unshare/Landlock, syn-based checks.
   Deferred with recorded design: LLM driver/test generation (§3.5 step 1), RESET
   trajectories, deny-by-default sandbox profile, nightly `-Zsanitizer` for the shim.

**What the live run taught (run #1, first sample, discarded pre-commit):** the 1B model
parroted the prompt back, including the contract's own `<blocked>reason</blocked>`
example, which the parser took for a verdict. Fixed: blocked detection is by content
(placeholder/echoed instruction rejected; markdown-wrapped genuine verdicts accepted).
Exactly the class of bug only a weak live model surfaces.

**Pre-commit code review (3 lenses) — 1 blocker + fixes, all with regression tests:**
live re-runs destroyed prior evidence (now immutable; `--retry` records sample
`.rN`, per-sample live traces); pre-main constructor forgery reproduced by the
reviewer (`__mod_init_func` printing forged output before main) — now rejected by the
symbol-set boundary; promotion recovery could roll back a verified promotion (now
evidence-aware); unbounded target-controlled `max_repairs` (clamped); trace dirs
could be redirected through a committed symlink (level-by-level real-dir check);
absolute paths leaked into committed evidence (scrubbed at the oracle source);
the sandbox gate only covered live providers (now every code-executing command);
`observe` skipped the context/truncation checks (shared `checked_complete`); replay
verified nothing against the ledger (now compares every turn, digest, and outcome).

**Recorded runs so far (unit u001-katajainen, identical `prompt_digest`
`blake3:991d2768…` on both):**
- `a-d6b377fb9257` — provider kind `anthropic` (the M2 adapter, base_url → local
  Ollama `/v1/messages`), model `llama3.2-1b-32k`, LIVE: format → format → build
  (model-written Rust compiled in the sandbox and failed) → truncated. Outcome
  `truncated`. ~19.4K input / 5.3K output tokens.
- `a-ef81857896e5` — `external` hand-off answered BLIND: a fresh subagent restricted
  to two tool calls (read the request JSON, write its answer), which never saw the
  existing verified crate. GREEN on the first turn under the full sandboxed oracle.
  The `model` field (`claude-sonnet-5`) is the configured string; the answering model
  was this session's Claude model via the subagent; usage unmeasured (null).

Commit sequencing is deliberate: THIS commit contains the executor, the seam, the
sandbox and run #1 through the pre-existing Anthropic adapter. The next commit adds
ONLY the OpenAI-compatible adapter + its registration + config, then records run #2 —
so `git diff --stat` between them is the literal proof of "zero code changes outside
the adapter and config".

## 2026-09-19 — M3 complete: the agnosticism proof, stated precisely; handoff (§17)

**The literal proof** (`git diff --stat 9a83b34..4cc5724` — commit A → commit B):

```
 crates/harness-llm/src/lib.rs                      |    1 +
 crates/harness-llm/src/openai_compat.rs            | 1057 ++++++++++++++++++++
 crates/harness-llm/src/providers.rs                |   22 +-
 providers.example.toml                             |    9 +
 .../attempts/a-82a651aef9fa/attempt.json           |   34 +
```
One new adapter file, one `mod` line, one `kind_builder` arm (+ the single existing test
that asserted the kind was unsupported), one user-level config profile, and the
evidence of the run. No executor, oracle, core, or CLI code changed between the run
through provider #1 and the run through provider #2.

**What the evidence supports — and what it does not.** M3 shows provider-agnostic
*plumbing*, not provider-independent migration *success*. One harness build drove unit
u001-katajainen with a byte-identical translate prompt (`prompt_digest
blake3:991d2768…`, identical on all three attempts; the translate turn reported 4552
input tokens on BOTH wires) through two independently written wire adapters —
Anthropic Messages (`/v1/messages`) and OpenAI Chat Completions
(`/v1/chat/completions`) — both LIVE against local Ollama 0.30.10 running
`llama3.2-1b-32k` (CONTEXT 32768 confirmed via `ollama ps`). The runs differ only in
the user-level provider profile. Both ended `truncated` (a 1B model cannot hold the
emission contract; run #1 did get one candidate as far as a sandboxed build failure).
That proves two wire formats, not two vendors, and it proves transport + executor +
ledger, not translation quality. The only GREEN came through the `external` hand-off,
answered BLIND by a fresh subagent limited to reading the request and writing its
reply (it never saw the verified crate; green on turn 1 under the full sandboxed
oracle); its token usage is unmeasured (`null`). **No cloud endpoint — including
api.anthropic.com — has been exercised by this harness yet.**

**Cheapest next strengthening (needs the user's consent — a multi-GB download):** pull
one capable local coder model (the spike names Qwen3.6-27B / Devstral Small 2 /
Qwen3-Coder-class, Apache-2.0) and re-run both wire adapters with `--retry`; a green
through BOTH live adapters is what "reproduced" in §8.3 ultimately means. Or set
`ANTHROPIC_API_KEY` and run the built-in `anthropic` profile.

**State:** gates green (fmt, clippy -D warnings, 288 tests incl. the e2e that drives
migrate → external hand-off → green → replay-verify → promotion → wrong translation →
repair request, all under `--allow-unsandboxed`-aware gating). Ledger: u001 verified
(verdict now records `symbol-set` and `sandbox: sandbox-exec`), three attempts
recorded with source-only evidence bundles, 10 units pending.

**Next actions (M4 — TRACTOR benchmark):** spike the corpus layout (Battery 01; P00
Perlin, P01 SPHINCS) → a second target under `targets/` → the missing executor stage
the corpus will force: LLM **driver/test generation** (§3.5 step 1 — today `migrate`
refuses units without a driver) with C-vs-C self-validation → scores as the regression
suite. Carry-forwards: held-out oracle corpus (revisit trigger: first live repair turn
that flips red→green); escalation tiers + budget enforcement + `harness usage` (§16,
enum strings already reserved); deny-by-default sandbox profile; Linux sandbox
(Landlock/unshare); nightly `-Zsanitizer` for ffi shims; canary injection set;
facts.db export; proptest.

## 2026-09-23 — M4 research spike (§15), design, and the adversarial design review

Two source-verified sweeps (corpus + scorer mechanics; LLM test/driver generation) and
direct verification of the corpus and its scorer:

- **Corpus:** `github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus` (MIT, Distribution A),
  tags `v1` (6ec7ae6, 2026-02-20) and `v2` (**37960ee08a7c…**, 2026-09-14, pinned). No
  upstream checksum manifest, no Cargo.lock. 346 case dirs; 150 non-SPHINCS library
  cases; B01 = 80 public library cases + 20 `Hidden-Tests` library cases (released with
  v1 — public since Feb 2026, i.e. NOT post-cutoff for any model used here).
- **Scorer:** the corpus's own `cando2` runners — one vector per invocation, `dlopen`s
  `lib<library>.dylib` from `build-ninja/` (C) or `translated_rust/target/release/`
  (Rust), compares serialized state + stdout/stderr patterns; `has_ub` vectors → Skip.
  42/42 B01 synthetic runners override `library:`/`symbol:`. First Evaluation Report:
  150 B01 tests (incl. 51 executables, hidden tests, Linux containers), best performer
  98.7% — our number is NOT comparable (different set, platform, and scorer harness).
- **Driver generation evidence:** determinism re-runs, sanitizers and mutation adequacy
  are supported; line coverage is a poor gate (advisory only); same-model test+code
  generation risks correlated blind spots → different model per stage.

**Design** (docs/M4-DESIGN.md): three layers that cannot see each other — a generated
driver (the oracle's test, pinned by the ORIGINAL C), the Rust translation, and the
corpus's public vectors, which never gate anything and never reach a prompt: they
MEASURE how good "oracle-green" is. **Adversarial design review (4 lenses):** security
**flawed**, measurement validity **flawed**, architecture and feasibility sound-with-
fixes; twelve resolutions R1–R12 (authoritative §R) — driver forgery gate
(`driver-shape`: object symbol allowlist + source lint) and run confinement (built
binaries read nothing under the target root — this also closed a latent M3 hole: a
candidate could read the previous turn's `drv_c.out`); held-out material never inside a
target root + capability parity for candidates; per-runner library/symbol names;
`-ffp-contract=off` everywhere (measured: Apple clang fuses FMA even at -O0; the -O0
`hsv_to_rgb` baseline failed 1/80 vectors until disabled); mutation gate hardening;
scorer supply chain (vendored, checksum-verified, `--frozen`); per-case strict pass as
the headline; replay-stable contracts.

**Dependencies (§11.1):** no new crates.io dependency in the harness. `harness-oracle`
gains `serde_json` (already a workspace dependency; parses the scorer's report) and a
path dependency on `harness-scan` (mutation sites + driver lint have one tree-sitter
owner). The corpus scorer's ~147 crates are the CORPUS's, locked in
`heldout/Cargo.lock`, vendored into a gitignored dir and built offline — never linked
into the harness. One portability patch to a scorer DEPENDENCY (`process-fun-core`
uses Linux-only `pipe2`; `cando2` cannot build on macOS without it) is hash-locked and
documented in `targets/tractor/heldout/patches/PATCHES.md`.

## 2026-09-23 — M4 run protocol, incidents, and bugs the run found

**Who answered.** No cloud keys; the local 1B model cannot hold the contract (M3). All
model calls went through the `external` hand-off, answered by Claude subagents:
**drivers by Sonnet 5** (`claude-sonnet-5`), **translations by Haiku 4.5**
(`claude-haiku-4-5-20251001`), escalating ONE tier to Sonnet 5 only after a Haiku
attempt exhausted its 1+3-turn budget red (a documented simplification of §16.2's "after
2 failed iterations": the harness cannot close an attempt early without forging a
reply). `--model` is the ACTUAL answering model this time (M3 recorded a placeholder).
Token usage per request is unmeasured (`null`); as logged by the agents' transcripts,
answering cost ≈ 1.63 M Sonnet output tokens (drivers), 0.79 M Haiku + 0.36 M Sonnet
output tokens (translations) — ≈ 22 K Sonnet-class and ≈ 9 K Haiku output tokens per
verified unit, agent overhead included (approximate).

**Blindness, audited mechanically.** Requests are rendered into isolated batch dirs
outside the repo (index kept outside too); each answering agent may only Read/Write
there; its transcript is audited before ANY answer is imported
(`targets/tractor/handoff-tools/`, results in `targets/tractor/handoff-audit.jsonl`):
51 batches, 687 tool calls, **0 breaches**, 4 deviations (an agent listing its own
batch dir, editing its own answer — recorded verbatim, blindness preserved), 296
answers imported. What it does NOT prove: anything about training data (vectors and
organic C are public), or the orchestrating session (which had read some vectors while
building the scorer; it only ever sent the fixed template in the tools README).

**Incident — workflow agents are not blind.** The first driver round used Workflow
agents; those are framed with the user's latest chat message and all ten answered the
chat instead (0 answers, every tool call out of protocol, some listing repo paths). The
audit caught it; nothing reached a ledger. Hand-offs use plain subagents since.

**Bugs the run found (each fixed with a regression test that fails without the fix):**
1. *Mutation gate vs equivalent mutants* (7cd8a52): tiny units (4–6 sites) had mutants
   that compile to byte-identical objects (`char`→`unsigned char` on a memcpy buffer, a
   pointer-cast deletion) — unkillable, so the small-n rule made units unvalidatable.
   **Trivial Compiler Equivalence** (Papadakis et al., ICSE 2015) now discards them:
   117 of 1447 compiled mutants suite-wide.
2. *Replay path leak — latent since M3* (0122106, 627224f): replay verification runs in
   `.replay-<id>/` but compiler evidence quotes the candidate's path; no attempt with
   build-failure evidence could ever reproduce. Alias applied in the scrub stage, BEFORE
   evidence is bounded (the paths differ in length, so truncation moved).
3. *Rust panic thread ids* (644f9c5): Rust 1.94 prints the OS thread id in panic
   messages; the evidence changed every run, so three attempts re-requested turn 2 on
   every resume and never progressed (15 orphaned answers). Scrubbed in the oracle.
4. *Raw stderr cut before any alias* (543660e): length-preserving replay scratch names.

**Findings recorded, not papered over:** `ima_decode_lib`'s turn-2 candidate has UB in
its unsafe FFI shim and dies with SIGABRT or SIGBUS nondeterministically; the signal is
repair evidence, so that finished attempt only replays when the crash repeats (the final
verified crate is deterministic). `043_iso646_and_digraphs_lib` is written with C
digraphs, which tree-sitter-c cannot parse (no unit; stays in the denominator).

## 2026-09-23 — M4 code review (4 lenses) and fix pass

Security: bench scoring had no sandbox floor (fixed: refuses without
`--allow-unsandboxed`); the inline-assembly ban was bypassable by a renaming import
(`use core::arch::asm as z; z!(…)` compiled under the scaffolding — fixed: any
asm-family identifier or `::arch` path fails `capabilities`). Correctness: a slow weak
driver bought mutation kills with the clock (7/22 → 22/22; fixed: a driver run over 3 s
fails determinism — all 91 drivers run < 0.4 s); raw-cut replay divergence (bug 4);
`bench check --replay` false failures (shared hazard helper; stale records skipped).
Measurement integrity: "verified" for scoring now requires a fresh green verdict and a
validated driver (`stale-verified` otherwise); harness failures are `infra-error`, never
a blind spot; the unsound "oracle false negative" class became `vector-pass/oracle-red`
over finished behavior-rejected attempts only; `bench check` judges regressions per case
(changed inputs elsewhere never mask them) and `score --write` re-verifies first.
Contracts: implementation matched SCHEMAS.md exactly; README was stale (updated).
Carry-forwards from review (MINOR): confinement setup failures surface as run failures;
`crash-timeout` label can be triggered by driver stderr text; `evaluate_mutation`'s
all-equivalent n/a is suite-wide, not per file; `interface` lines reach the trusted
`[ABI CONTRACT]` region un-fenced (length-capped, printable) — injection hardening.

## 2026-09-23 — M4 state at session end (§17 handoff) — READ THIS FIRST

**Branch:** `claude/rust-migration-harness-02116f` (worktree), all work committed through
93cd89b. NOT yet merged to `main` / pushed — do that after the final score lands.

**Results so far (preliminary `bench score`, oracle as run, macOS arm64):**
- public split: strict-pass **70/77** scorable cases (90.9%), 70 verified, **0 blind
  spots**, 908/950 non-UB vectors; unscorable 2, C-baseline-invalid 1 (007 errno_pow).
- released-hidden split: strict-pass **14/18** (77.8%), 17 verified, **3 blind spots**,
  81/89 vectors; unscorable 1, C-baseline-invalid 1 (016 switch_arith: glibc `rand`).
- Pipeline: 91/99 drivers validated (8 blocked: UB-only units + 007/017), 89/99 units
  verified; float2half unfinished (Sonnet escalation mid-repair), 043 digraphs no unit.
- **The 3 blind spots, diagnosed** (the key M4 finding): `decorrelate` — the vector
  exercises UNMARKED C UB (writes residuals[5] of a [5] array); `014_pow_subfunction` —
  the unit reports on **stderr and the differential oracle compares only stdout**
  (a real oracle hole; fix next); `read_scalefactors` — verified Rust segfaults on 3
  vectors: its FFI shim sizes slices from struct fields in ways the driver never broke.
- Not comparable to the First TRACTOR Evaluation Report (different set, platform,
  harness); public-vector score, contamination risk disclosed (see M4 protocol entry).

**In flight at handoff:** `harness bench score --suite targets/tractor --write` (old
sequential binary, re-verifies + re-validates every case, ~2 h). If `scores.json` is
absent, re-run it (now parallel):
`cargo run -q -p harness-cli -- bench score --suite targets/tractor --write --jobs 6`
(remove a stale `targets/tractor/.bench/LOCK` first if no run is active; the scorer
needs `targets/tractor/.scorer-vendor` — see targets/tractor/README.md).

**Next actions, in order:**
1. Record `scores.json`, fill the final numbers into the M4 write-up above (per split,
   organic vs synthetic, n's), commit; run `bench check` (expect exit 0) — the
   regression suite demonstrated.
2. Merge to `main`, push (solo-dev rule), update README status numbers.
3. Fix the stderr oracle hole (compare driver stderr C vs Rust in
   `differential-driver`); re-verify; `bench check` should then report
   `014_pow_subfunction` as a lost-verified REGRESSION — the suite working as intended;
   re-migrate it.
4. Carry-forwards: FFI boundary fuzzing (slice sizes / null pointers — the
   read_scalefactors class); UB-vector detection (run vectors on the C under ASan to
   auto-flag unmarked-UB vectors like decorrelate); confinement setup failures as
   harness errors; §16 escalation automation + `harness usage`; interface-line fencing;
   Linux sandbox; post-M4 user direction (TUI cockpit, feature-workflow view, C-vs-Rust
   perf baselines — see memory/roadmap and the TUI design brief summary in the chat
   record: thin CLI wrapper, ledger-derived, `--json` events first).

## 2026-09-23 — M4 complete: final scores (recorded) and the regression suite demonstrated

`targets/tractor/scores.json` recorded by `bench score --write --jobs 6` (re-verifies
and re-validates every case first); `bench check` against it → exit 0 ("no
regression"). Environment as recorded in the file: rustc 1.94.1, Apple clang 21.0.0,
macOS 26.5.2 aarch64, `sandbox-exec`, C baseline `cc -shared -fPIC -O0
-ffp-contract=off`. The final numbers equal the preliminary ones exactly.

**Headline: per-case strict pass over scorable cases** (scorable = cases minus
`unscorable` (every vector UB-marked, or none) minus `c-baseline-invalid` (the C
itself fails its vectors on this platform)). Vectors = non-UB vectors of scorable
cases (C-baseline pass+fail); passed = the Rust side's passes.

| Split / origin | Cases | Scorable | Strict pass | Verified | Blind spots | Vectors passed |
|---|---|---|---|---|---|---|
| public organic | 38 | 37 | 36 (97.3%) | 36 | 0 | 748/772 |
| public synthetic | 42 | 40 | 34 (85.0%) | 34 | 0 | 160/178 |
| **public** | **80** | **77** | **70 (90.9%)** | **70** | **0** | **908/950 (95.6%)** |
| hidden organic | 10 | 9 | 7 (77.8%) | 9 | 2 | 64/69 |
| hidden synthetic | 10 | 9 | 7 (77.8%) | 8 | 1 | 17/20 |
| **released-hidden** | **20** | **18** | **14 (77.8%)** | **17** | **3** | **81/89 (91.0%)** |

Every non-strict-pass case is one of: **blind spot** (verified, fails vectors) —
hidden `decorrelate` (unmarked C UB in the vector), `read_scalefactors` (FFI shim
segfaults on 3/6), synthetic `014_pow_subfunction` (stderr not compared — the oracle
hole); **unverified** (no verified unit; scores 0 vectors) — public `float2half`, `011`,
`012`, `015`, `017`, `018`, `043` (digraphs, no unit), hidden `027`; **unscorable** —
public `update_md5`, `008_long_run`, hidden `md5_transform`; **C-baseline-invalid** —
public `007_errno_pow`, hidden `016_switch_arith`. 0 stale-verified, 0
vector-pass/oracle-red, 0 infra-error.

**What the evidence supports, precisely:** on this platform, of the 87 units the oracle
verified in scorable cases, 84 pass every held-out non-UB vector and 3 do not (all
three diagnosed; one is an oracle defect, one a UB-in-vector measurement artifact, one
a genuine FFI-boundary blind spot). The hidden split is not a contamination-free
held-out measure (released Feb 2026, before the answering models' cutoff); the
public-vector score carries the same disclosed risk. Answering models were Claude
subagents (drivers Sonnet 5, translations Haiku 4.5, four units escalated to Sonnet 5)
behind the audited blind hand-off. NOT comparable to the First TRACTOR Evaluation
Report (150 B01 tests incl. executables, Linux containers, the official harness).

**Budget:** answering ≈ 22 K Sonnet-class + ≈ 9 K Haiku output tokens per verified unit
(approximate, from agent transcripts; per-request usage unmeasured — see §16 carry-
forward `harness usage`).

## 2026-09-23 — Post-M4: the stderr oracle hole, fixed and demonstrated end to end

**Defect.** `differential-driver` (and `whole-program:*`) compared stdout only.
`014_pow_subfunction` reports domain/range errors on stderr; its Rust dropped them and
verified. Two causes, both fixed: the oracle (`ef94105`) and the translate prompt, which
told every printing unit "Output to stderr is not compared" (`c075eee`).

**Oracle semantics (normative in docs/SCHEMAS.md "Observable output").** A clean run's
observable behavior is stdout AND stderr, everywhere a run is judged: the differential
checks, driver validation (determinism, -O0 vs -O2, mutation kills), and repair evidence
(stderr line diffs). Detail strings are unchanged when both stderrs are empty, so
recorded evidence of stdout-only units replays byte-identically. Records gain the
toolchain entry `observable: stdout+stderr` (provenance, as `cflags:` in M4).
Adversarial review (1 reviewer, 4 lenses) found one real measurement defect, fixed: each
confined run has its own fresh `TMPDIR`, so any stderr naming it would be a false diff
or false non-determinism — the run's temp-dir path now reads `$TMPDIR` in captured
output. Minor: validation details now label `on stderr` / `on stdout and stderr`.

**Prompt.** Only a unit whose C names `stderr` (whole-token scan of its closure) gets the
corrected sentence (write it with `eprint!`: unbuffered like C's stderr; the
capabilities gate allows `std::io::stdio`). Every other printing unit keeps the M4
sentence verbatim — a deliberate, recorded inaccuracy that keeps their recorded traces
replayable; a candidate that writes stderr anyway fails the oracle (a repair turn, never
a wrong verdict). Revisit at the next deliberate prompt revision (it will invalidate
every recorded request key — see "interface fencing" below).

**Executor gap found and fixed (`85e77bd`).** `--retry` was silently ignored for the
`external` provider on the premise that a trace-backed trajectory can only reproduce
itself — false as soon as the judge changes. 014's finished attempt stopped
reproducing and could never be re-sampled. Now `--retry` re-verifies the latest sample
and records `<base>.r<N>` only when it does not reproduce (identical requests reuse
their recorded answers); `bench check --replay` replays only the latest `external`
sample of a base. Regression tests fail without each fix.

**Demonstration.** Under the new oracle `bench check` (vs the M4 `scores.json`) exited
10: `PROBLEM: 014_pow_subfunction_lib: verified unit's oracle is RED`, nothing else
(stale-verified 0: every validated driver re-validated with stderr compared). `verify` demoted the
unit; re-migrated through the audited hand-off with Haiku 4.5 (2 batches, 0 breaches,
0 deviations): turn 1 red on stderr (`NaN` vs C's `nan`, shown as a stderr line diff),
turn 2 green; held-out score 7/7 strict pass (was 5/7, a blind spot). The M4 attempt
`a-4adfe6d56ab0` stays as evidence of the stdout-only verification.

**Re-baseline** (`bench score --write`, all cases re-verified under the new oracle):
released-hidden strict pass **15/18** (83.3%; was 14/18), verified 17, blind spots **2**
(decorrelate — unmarked C UB; read_scalefactors — FFI boundary), vectors 83/89; hidden
synthetic 8/9 (vectors 19/20; was 7/9, 17/20), hidden organic unchanged 7/9. Public unchanged:
70/77, 0 blind spots, 908/950. No other case changed class.

## 2026-09-23 — Carry-forward closed: confinement setup failures are harness errors

`Confinement::run` now returns `Result<Result<RunOutput, RunFailure>, Error>`: how the
RUN ended (crash, exit, timeout) stays a failed check (evidence); a confinement that
cannot be SET UP (temp dir, sandbox profile) is a harness `Error` — never text fed to a
model as repair evidence (§16.2: sandbox misconfiguration is a harness bug). Kept as
before, deliberately: a built binary that cannot be SPAWNED stays a run failure (the
M3 decision and its test). Regression test fails without the change.

## 2026-09-23 — Research spikes (§15) for the two remaining blind-spot classes

Both run in cheap subagents (read-only; held-out vectors not read for design).

**A. Unmarked-UB vectors (the `decorrelate` class).** The C writes `residuals_0[5]`
through a pointer decayed from a 5-element struct member; the -O0 baseline "passes" by
luck, correct Rust fails → a false blind spot. Findings: `-fsanitize=bounds` would NOT
catch it (decay erases the static bound — clang UBSan docs); ASan does (stack redzone).
Dlopen'ing an ASan dylib into the scorer's uninstrumented runner aborts ("interceptors
not installed"; rust-lang/rust#79934), and `DYLD_INSERT_LIBRARIES` would be purged by
the SIP-protected `/usr/bin/sandbox-exec`. **Experiment (this session, macOS 26.5
arm64, Apple clang 21):** a Rust host linked with `-C link-arg=<clang>/lib/darwin/
libclang_rt.asan_osx_dynamic.dylib` (+ rpath) dlopens an `-fsanitize=address,undefined`
dylib and reports the decayed-pointer write as `stack-buffer-overflow` (exit 134), also
under `sandbox-exec`; the uninstrumented dylib silently returns. **Direction:** a
sanitized scoring pass — the corpus's own runner built a second time with the ASan
runtime as a link dependency (no vendored-code change) + the C baseline built with the
oracle's `SANITIZER_FLAGS`; a non-`has_ub` vector that trips a sanitizer on the C side
is `unmarked-ub`: excluded like `has_ub`, counted and disclosed separately. Linux: same
build, no loader issue. Rejected: dlopen + `DYLD_INSERT_LIBRARIES` (SIP), trap-only
UBSan (misses this class), per-vector generated C mains (re-implements the scorer's
marshalling). Risk: signed-overflow/alignment reports in intentionally-UB synthetic
cases — report sanitizer kind; revisit if a runner cannot be relinked.

**B. FFI-boundary blind spots (the `read_scalefactors` class).** The verified Rust's
`ffi.rs` eagerly builds `slice::from_raw_parts(buf, len)` with `len` derived from a
bit-limit field, not an allocation size; the C walks the buffer lazily. The LLM driver
never drove consumption past the physical buffer. Options: coverage-guided fuzzing —
rejected today (Apple clang 21 ships no `libclang_rt.fuzzer_osx.a`; cargo-fuzz needs
nightly: rust-fuzz book); LLM-prompted boundary cases — a complement, no guarantee;
**harness-generated boundary battery** (tree-sitter-c finds pointer+length pairs in
signatures/struct fields; values 0, 1, N−1, N, N+1, null+len) mutating the validated
driver's inputs, run differentially — chosen direction. In-contract filter = the C side
is sanitizer-clean on that input (as RustAssure, arXiv:2510.07604, filters on C defined
behavior); any Rust crash on an in-contract input fails. Risks: indirect length
encodings (sentinels, cross-struct) missed; runtime bound. Revisit when a libFuzzer
runtime is an approved dependency and stable Rust gains instrumentation.

Both become written designs + adversarial design review before any code (M4 process).

## 2026-09-23 — Unmarked-UB vectors: the sanitized-C scoring pass (design A, implemented)

Design, reviews and the revision: docs/ORACLE-HARDENING.md §A (§A.R, §A.2, §A.3
authoritative); contract: docs/SCHEMAS.md "scores.json — unmarked-UB additions".

**Rule.** A vector the plain C passed but the verified Rust or scored candidate did not
is re-run against a SANITIZED C; if the C itself is proven memory-unsafe on it — an
allow-listed ASan report, or a `-fbounds-safety` trap — the vector is `unmarked-ub`:
excluded like `has_ub`, counted (`unmarked_ub`, `vectors_unmarked_ub`) and printed with
its paired Rust result. A vector the plain C fails is never excused (no laundering of
c-baseline-invalid cases); UBSan is deliberately not used (it fires on intentional
two's-complement idioms the Rust must reproduce).

**The finding that changed the design.** The ASan-only mechanism (spike + 3-lens review
all agreed) excused NOTHING end to end: cando's uninstrumented runner owns every
vector's state, so `residuals[5]` lands inside memory ASan never poisoned. Apple clang's
`-fbounds-safety` (bounds-carrying local pointers, no runtime) traps it under the
unmodified runner; unannotated pointer-arithmetic code does not compile with it, so the
pass falls back to ASan (read_scalefactors — correctly NOT excused: its failures are a
genuine Rust bug). Lesson recorded: a spike's premise about a third-party harness's
memory ownership must be verified by running it, not by reading the C alone.

**Mechanism.** The corpus's own runner is built a second time with the ASan runtime as
a link dependency (`--target <host>` + `target.<triple>.rustflags`, so build scripts and
proc-macros are untouched); no vendored-code change; no `DYLD_*` (SIP's sandbox-exec
would purge it). `ASAN_OPTIONS` harness-set. Reports are read from cando's report
(`output.stderr`), where cando captures its per-vector child's stderr.

**Reviews.** Design 3 lenses (all sound-with-fixes: R-A1..R-A12); code 2 lenses:
security clean; correctness — a lost excusal on an unverified case read as a Rust
regression (fixed, test), the bounds-safety fallback was silent (now recorded per case
as `sanitized_build` and printed); contract text amended to the real strings.

**Re-baseline** (`bench score --write`; the environment fingerprint gained
`sanitized-pass: asan+bounds-safety`, so the M4 baseline is deliberately incomparable):
released-hidden strict pass **15/17** (88.2%; scorable 18 → 17: decorrelate is now
`unscorable`, both its vectors excused as `bounds-safety-trap`), verified 16, blind
spots **1** (read_scalefactors), vectors 83/87. Public unchanged: 70/77, 0 blind spots,
908/950, 0 excused. The pass ran on exactly the two cases where it could change a class.

## 2026-09-23 — Session end (§17 handoff) — READ THIS FIRST

**State.** All work on `main` (fast-forwarded from branch
`claude/rust-migration-harness-7d1c42`) and pushed at each milestone. M4 closed
(scores.json recorded; regression suite demonstrated twice: exit 0 on the M4 baseline,
exit 10 when the stderr fix exposed 014). Post-M4 oracle hardening done: stderr
compared (+ prompt fix for stderr-writing units), `--retry` for hand-off attempts,
confinement setup failures are harness errors, sanitized-C pass for unmarked-UB
vectors. Current scores: public 70/77 (0 blind spots), released-hidden 15/17 (1 blind
spot: read_scalefactors).

**Next actions, in order.**
1. **Design B** (FFI-boundary blind spots, read_scalefactors): the DRAFT in
   docs/ORACLE-HARDENING.md §B → adversarial design review (3–4 lenses) → implement →
   code review → re-migrate read_scalefactors under the new check → re-baseline.
2. **Decision needed from the user — prompt versioning.** Any change to prompt text
   changes every request key, so every recorded attempt stops replaying (§16.2 makes
   replay mandatory for harness development). Fencing `interface` lines (M4 review
   carry-forward) and a translator hint for B both need it. Proposal: versioned prompt
   templates — `attempt.json` records the template version; replay renders the
   version the attempt was recorded under; new attempts use the latest. Until decided,
   prompts stay byte-identical except via NEW stages or unit-conditional sections
   (the stderr precedent).
3. §16 escalation automation + `harness usage` (per-request tokens are `null` for
   hand-offs: usage must be recorded from the answering runtime or stay unknown).
4. Carry-forward (MINOR): the `crash-timeout` classifier matches substrings of check
   details that can quote child stderr; a structural fix changes recorded turn
   `result`s (replay-compared) — bundle with the prompt-versioning work.
5. After the above (user direction): TUI cockpit (CLI hardening first), feature-workflow
   view, C-vs-Rust performance baselines — each starts with its own spike.

**Environment notes.** Model calls go through the `external` hand-off answered by plain
Agent subagents (never Workflow agents), audited with targets/tractor/handoff-tools;
HANDOFF_ROOT must be outside the repo; HANDOFF_TRANSCRIPTS = the session's tasks dir.
The scorer needs the gitignored `targets/tractor/.scorer-vendor/` (copy-on-write clone
from another worktree with `cp -cR`, or re-vendor per targets/tractor/README.md); the
first sanitized scoring run builds a second scorer (~2 min).

## 2026-09-23 — PROPOSAL (awaiting user decision): prompt edits vs recorded-trace replay

**Problem.** `request_key` = 8 hex of blake3(serialized {model, system, user,
max_tokens}); replay RE-RENDERS each turn from current code and looks the recorded
response up by that key, then re-runs the oracle. One edited sentence therefore
orphans all 203 recorded attempts (328 requests); re-recording would cost ~2.8M output
tokens and produce DIFFERENT evidence. Blocked: fencing `[ABI CONTRACT]` lines
(security), a translator hint for design B, and a now-false sentence ("Output to stderr
is not compared") left in 72 recorded requests' prompts by the unit-conditional
workaround.

**Review** (workflow: 3 research sweeps, 44 sourced findings; 3 competing designs, each
adversarially refuted; synthesis). No design had a fatal flaw. Ranking:
1. **Evidence-first replay + prompt-conformance lock** (7/10) — replay loads each turn by
   the RECORDED key, checks the bytes hash to it, re-runs the CURRENT parser + oracle and
   requires the same results/candidate/outcome; a separate conformance report says
   whether today's renderer would send the same bytes (turn-1 rows enforced in
   `cargo test`/CI as a lockfile; repair rows in `bench check --replay`), naming drifted
   sections. Rules: only the latest prompt may ever SEND a request; a default re-run
   verifies existing evidence (`--new-trial` to start fresh); supersession is an
   explicit `expected-divergence` record, never implied by a newer attempt; scores are
   reported per prompt fingerprint. Stops proving (for drifted turns only): that HEAD
   would pose the same question. Borrowed from Inspect's re-scoring of stored outputs,
   mergewatch's prompt lockfile, inspect_evals task versioning, Restate divergence
   messages; skipped: semantic caches, body-blind cassettes (pydantic-ai#8023),
   hosted registries, new crates.
2. Sealed/pinned prompt versions (the original proposal, strengthened) (6/10) — sound
   and mature practice (Temporal/DBOS-style versioning) but keeps every old renderer
   (incl. evidence formatter) in the binary forever to re-prove bytes already
   committed; old versions can still send requests (reopens the injection hole the ABI
   fence closes unless forbidden); does not address oracle/rustc drift inside
   [EVIDENCE].

**Defect found and verified this session.** `bench check --replay` would fail today on
014's M4 attempt `a-4adfe6d56ab0`: the stderr prompt fix gave 014 a new first request
(a new base id), and `superseding_sample` only covers `.rN` samples of the same base.
Verified: replaying it errors ("recorded for a different translate prompt"); the new
attempt `a-bf33266e0112` replays GREEN. Both designs fix it with an explicit
supersession record. Not patched pending the decision.

## 2026-09-23 — DECIDED (user): evidence-first replay; the prompt edits it unblocked

The user approved the review's recommendation. Implemented per docs/REPLAY-DESIGN.md
(spec → 4-lens design review → implementation → 3-lens code review, 17 confirmed
findings fixed with regression tests, the rule-guarding ones mutation-checked).

**What replay proves now.** Each finished attempt bound to the current inputs is verified
from its RECORDED requests and replies (read-only; nothing is ever sent or filed):
integrity (keys, hashes, prompt digest, model, id re-derivation), then HEAD's parser and
judge re-judge the recorded replies — results, candidate and outcome must reproduce
(strict). A repair turn whose HEAD render differs ONLY in `[EVIDENCE]` is strict too (the
evidence-determinism net that found three M4 bugs; its tests are mutation-checked).
Conformance reports whether HEAD would pose the same requests. Lost, deliberately: for a
drifted turn, the proof that HEAD would have asked the same question.

**Deferred from the approved proposal, with reasons (design review):** default-run reuse
of existing evidence and `--new-trial` — unsafe as specified, since the fallback ignored
prompt inputs other than the C source (a newly confirmed hazard would have been silently
skipped); per-prompt score labels — after the first prompt edit every case reads
"drifted", which carries no information. A changed prompt is simply a new trial when a
stage is run explicitly.

**Also shipped:** typed `Error::Diverged`; `superseded.jsonl` (hand-written, strictly
checked: intact, a tightening unless flagged, green successor for a green record, never
the scored artifact; 014's M4 attempt is the first entry); the promoted attempt is the
one whose candidate IS the crate (fixes 014's two `promoted:true` records); prompt
fixtures for every prompt branch plus guard tests (a prompt edit is a reviewed diff);
driver-diff evidence path-scrubbed plus a guard that refuses to send a machine path.

**Prompt edits landed (each with its fixture diff):** every printing unit is told
stderr is compared (the unit-conditional workaround deleted); the `[ABI CONTRACT]` lines
are fenced as JSON literals in `<abi_NONCE>` blocks (M4 review carry-forward closed). The
u001 render-equality golden was retired as planned; its id/binding half is permanent.

**Gates** (see the entry below for the numbers): `bench check --replay` on the renderer
unchanged (commit A), then on the edited prompts (commit C).

**Gate results** (`bench check --replay`, full TRACTOR ledger, zero tokens):

| Run | Reproduce (strict) | Conformant / drifted | Expected divergence | Problems | `bench check` |
|---|---|---|---|---|---|
| before (old engine) | 198 | n/a | — | 1 (014's M4 attempt) | FAILED |
| commit A (engine, renderer unchanged) | 198 | 198 / 0 | 1 (014, superseded) | 0 | OK |
| commit C (stderr sentence + ABI fence) | 198 | 0 / 198 | 1 (014, superseded) | 0 | OK |

The last row is the point of the change: both prompt edits touched every prompt, and
every recorded attempt still re-judges to its record. Under the old engine the same
edits would have left zero replayable attempts.

## 2026-09-23 — Design B: research re-check, design, adversarial review; DECIDED (user): mechanical

**Baseline at session start (zero tokens):** tree clean and green (424 tests); `bench check
--replay --jobs 6` → `198 reproduce (0 conformant, 198 drifted), 1 expected divergence(s), 0
skipped, 0 problem(s)`, `bench check: OK`, exactly as the previous handoff predicted.

**Research re-check (§15, three subagents, verified end to end on this machine).** No published
C→Rust system measures what the source touches and holds the translation to it (RustAssure fills
100-element symbolic buffers; Syzygy records allocation bounds, not accesses; SACTOR/ENCRUST
adopt "length from a sibling field or a conservative fixed bound" — our blind spot, published as
the method; TRACTOR's official harness has no footprint oracle; &inator "does not determine the
sizes of arrays"). Toolchain facts: `-fsanitize-coverage=edge,trace-loads,trace-stores` traces
every scalar access with no runtime library (`edge` required, else it silently instruments
nothing); aggregate copies, `mem*`/`str*` (fortified to `__*_chk` even at -O0), RMW atomics and
>16-byte vectors are untraced; a `PROT_NONE` access raises SIGBUS on this platform with an exact
`si_addr`; mmap/mprotect/sigaction/sigaltstack all work under the run profile; stable rustc can
trace loads but misses `memcpy` and relies on LLVM-internal flags (not used); no Miri/ASan/libFuzzer
on stable. `__asan_locate_address` gives exact object extents for stack, heap and globals and
reports uninstrumented memory as unknown, also under `sandbox-exec`.

**Prototype (the premise, run before designing).** The validated `read_scalefactors` driver with
every unit argument allocated through a guard API: the harness measured the C's per-allocation
windows, re-ran with each allocation shrunk to its window — the C stayed byte-identical in both
layouts (window flush against the following page, then the preceding one), the VERIFIED Rust
faulted (`scfcod`, call 1: the C touches none of it), a fully lazy Rust passed. Per-call windows
(not whole-run) are required: the driver's own read-back of `bs->pos` masks the eager `&mut *bs`.
A unit with an untraced struct copy was learned and widened correctly (phase L converged in one
round).

**Design.** docs/ORACLE-HARDENING.md §B.0–B.13 (measured-footprint boundary check: sancov
measurement → guard-page enforcement, per call, two layouts, learn/confirm ground truth, a wrapper
generated from the plan's interface lines, a model-written boundary driver as a new stage).

**Adversarial design review** (workflow: 4 lenses → 46 findings → 20 blocker/serious ones each
handed to an independent verifier; the verdicts are in §B.R, authoritative). Two findings change
the shape of the work:
1. **Fail closed, or it is not an oracle (SEC-1/2, reproduced under the real sandbox profile).** A
   candidate can intercept the guard fault before it becomes a signal or replace the handler
   through an unlisted installer; the `capabilities` denylist is "a policy, not an enforcement
   boundary". The runtime must verify its own handler, the exception ports and a canary page after
   every call (§B.R-1).
2. **Mechanical, not model-written (S1, rebuilt independently by two lenses).** The check can run
   the already-validated, mutation-gated `driver.c` unmodified through the generated wrapper, with
   ASan locating each argument's object and per-call copy-in shadows giving exact per-call windows
   — zero tokens, no new stage, record, bench or replay surface, and it subsumes seven other
   findings. Cost: memory reached only through pointer fields stays unchecked (disclosed; a
   partial detector is specified). §B.R-2 recommends it; it reverses the written design's
   direction, so it is the user's call.
Other confirmed serious findings and their resolutions: measure at -O1 (widening was the norm at
-O0); C-side failures are red checks, never harness `Err`s (an `Err` would abort the whole suite);
`mem` and `signal` capability classes (no recorded verdict changes — all 189 recorded crates were
rebuilt to prove it); gates that can actually observe the byte-identity claims (step 0: re-record
`scores.json` at HEAD, which already drifts by six `pipeline.migrate_outcome` values from the R-5
rule); positive controls (the four `read_scalefactors` variants) and a stratified calibration set.
Refuted: binding attempts to the boundary driver, a tightness gate, an env-var table channel, an
overflow finding.

**Landed this session (committed, green): ** `harness_scan::parse_interface` (the plan's
interface lines → wrapper shape; hostile-line refusals; every corpus line parses except zopfli's
unnamed-parameter unit and the two `driver(...)` symbols the plan names differently); the
`capabilities` `mem` class with its regression test (a candidate declaring `mprotect` is red).
Kept in the worktree, uncommitted (the runtime is being reworked to §B.R-2's copy-in shadows and
§B.R-1's integrity checks; the confinement extras are dead code until the orchestrator uses them):
`crates/harness-oracle/src/boundary.rs` + `boundary/` (rh_in runtime v2, parsers, wrapper
renderer, 6 tests), `confine.rs` `Extras` (RUHARNESS_* env + strict read-back of one temp-dir
file, 1 test). Prototypes and every review experiment are under the session scratchpad.

**Next, in order (after the user's B.R-2 decision):** step-0 `bench score --write` gate; revise
§B's body to the decided mechanism; implement the runtime (integrity checks first, mutation-
checked), the wrapper + compiler classifier probe, verify integration (check last, red on C-side
failure, omitted when not configured), the four fixtures; adversarial code review; calibration
run over the stratified set (zero tokens under B.R-2); re-migrate `read_scalefactors` on the
unhinted prompt through the audited hand-off; supersede its old green attempt; re-baseline.

**DECIDED (user, 2026-09-23): "Mechanical now, model-written later if calibration shows it's
needed."** §B.R-2's mechanism is the always-on baseline; a model-written additive boundary driver
(the original §B.7/§B.8, kept in git history at 67a8cea) becomes a §B.13 revisit trigger: adopt
it only if calibration shows the nested-pointer-field gap (§B.R-11) matters in practice. §B's body
is being revised to the decided mechanism; step 0 of the gates (`bench score --write` at HEAD)
runs first.

**Runtime v3 validated before the Rust port (2026-09-23).** The mechanical runtime, run through
a scratch orchestrator implementing §B.4 on three real cases under a run-like sandbox profile:
`read_scalefactors` — verified Rust RED at `bs` in call 1 ("the C does not touch it in that
call"), scfcod-lazy RED at `bs`, fully lazy GREEN, the `bs->buf` over-read GREEN with the
driver-stack detector firing (the disclosed pointer-field limit, correctly surfaced);
`hex2bin` (pointer out-param, 31 string-literal pass-throughs) — C byte-identical in both
layouts, verified Rust GREEN; `006_static_alias` (pointer return, two symbols) — GREEN. Tamper
experiments: macOS maintains `ru_nsignals` (3 handled faults → 0→3), and a candidate that
installs its own handler, survives its over-read and restores the handler before returning is
GREEN under the after-call checks alone and `RH-TAMPER signal` with signal accounting — added
to §B.R-1 as check (1). The new `signal`-class names occur in none of the 191 recorded crates.

## 2026-09-23 — Design B implemented (mechanical, §B.R-2); gates green; calibration next

**What landed** (every workspace gate green: fmt, clippy `-D warnings`, 443 tests):
- `harness-oracle/src/boundary/`: the runtime (`ruharness_guard.c`, its header, the
  probe) — per-call copy-in shadows located by ASan in the measure build, tail/head
  layouts, learn mode, pointer relocation on exit, no address reuse, and the §B.R-1
  integrity checks (signal accounting, handler, exception ports, canary).
- `harness-oracle/src/boundary.rs`: the pure half — strict parsers of the run records,
  windows and learning, the window table, the generated call wrapper and the compiler
  classification probes (baseline / is-pointer / pointee-complete), the runtime digest.
- `harness-oracle/src/boundary_run.rs`: phases 0/M/L/C/R and the `boundary` check; every
  C-side outcome a failed check with the shared lead-in
  (`harness_core::verdict::BOUNDARY_C_SIDE_LEAD_IN`), never an `Err`.
- `verify`: the check runs last, only for `[unit.oracle] boundary = true`, only when every
  earlier check passed; the toolchain entry `boundary: sancov+guard-pages rt=<8 hex>`.
  `CAbiDifferential::boundary_only` is the calibration entry point; `harness bench
  boundary` loops a suite with it, writing nothing.
- `capabilities`: `mem` and `signal` classes; an opted-in unit's candidate never gets
  either; `ruharness_*`/`__sanitizer_cov_*` references are always red (tests).
- `harness-scan`: `parse_interface` exposes the declarator span, pointer returns and
  function-pointer parameters. `confine.rs`: `Extras` (harness-set `RUHARNESS_*` variables,
  strict read-back of one temp-dir file; test).
- The migrate judge: a C-side `boundary` failure is a harness error like a red
  `driver-shape`; a candidate-caused red gets `BOUNDARY_EXPLANATION` (class `oracle`) with
  its fixture `migrate-repair-boundary.txt` and the guard entry — a prompt edit confined to
  a new failure branch; no existing fixture changed.
- Tests: `harness-oracle/tests/boundary.rs` — a synthetic unit shaped like the blind spot
  (a struct read only on some path, a buffer read up to `n`, a pointer return the driver
  dereferences): no entry when not opted in (byte-identity), green for a faithful
  candidate (16 calls, 40 objects, 17 untouched, 16 partial, relocation proven by the
  dereferenced return), red for an object the C never touches and for a read past the
  window with the harness's wording, a candidate that alters fault delivery caught by
  `capabilities` first and by the runtime (`RH-TAMPER handler`) through the calibration
  entry point, the C-side lead-in when an interface line names a type the headers do not
  declare, and the gate (a red `symbol-set` → no `boundary` entry).
- Contracts: docs/SCHEMAS.md "The boundary check"; README.

**Verified through the CLI on the corpus before the tests were written:**
`read_scalefactors` red ("in call 1 of read_scalefactors, the Rust touched the object
passed as `bs` (1 x 16 bytes) the C does not touch it in that call"); `hex2bin` green (57
calls, 114 objects, 31 literals unshadowed); `collided` not applicable — its helper
symbols' types live only in `lib.c` (the driver redeclares them), which the harness cannot
know: the baseline probe reports it honestly. That probe exists because the first version
took ANY compile failure of the is-pointer probe as "pointer" and produced a nonsense
wrapper for a by-value struct.

**Next:** the calibration run (`bench boundary` over the suite, zero tokens) →
adversarial code review → fix pass → opt in `read_scalefactors`, re-migrate it on the
unhinted prompt through the audited hand-off, supersede its old green attempt, re-baseline.

## 2026-09-23 — Design B calibration (§B.R-10): `bench boundary` over the whole suite, zero tokens

**Totals (100 cases):** 30 green, 5 red, 54 not applicable, 11 skipped (not verified / no
unit), 0 harness errors. No green is vacuous (every green has at least one object the C
touches partially or fully). One green carries the pointer-field note (the C reads the
driver's stack through a pointer field: unchecked, disclosed).

**Every red diagnosed by hand — 5 of 5 are real under the rule, 0 false reds:**
- `read_scalefactors` (hidden organic): `bs` dereferenced at entry; the C never touches it
  in call 1. The known blind spot, caught.
- `dequantize_granule` (public organic): `let bs_ref = &mut *bs; … bs_ref.limit` at entry;
  the C never touches `bs` in call 2. Same class, verified green since M4.
- `002_echo` (hidden synthetic): the shim loops `for i in 0..argc` and reads `argv[0]`;
  the C loops from 1 and never touches `argv` when `argc == 1`.
- `wcscat` (public organic): the shim scans `src` to the NUL before knowing the room in
  `dst`; the C reads `src` only while `ptr < dst + numElem` and stops at 5 elements in call
  9 — the classic eager `strlen` over-read of a source that need not be terminated within
  the room.
- `hdr_compare` (public organic): the shim builds a 3-byte slice and the logic reads
  `h1[2]`; the C's `&&` chain short-circuits after `h1[1]` in call 3. **Stance recorded
  (B.R-10 asked for it):** the per-call footprint stands, with no "size fixed by contract"
  exception — that contract is exactly what nobody wrote down, `read_scalefactors` is the
  same shape, and the fix in `ffi.rs` is one lazy read. Practical severity is low
  (every real MP3 header is 4 bytes); the check reports it, the re-migration decides.

**Not applicable (54), classified:** 33 "no data-pointer parameter" (honest: nothing to
guard, mostly synthetic units); 15 "the generated call wrapper does not compile" — a
harness bug, one cause in 14 (a helper symbol defined in `lib.c` but declared in no header:
the wrapper must declare each symbol from its interface line, which is what the line is
for) and one more in `crc16` (a PARAMETER named `crc16` shadows the function inside the
wrapper: bind the real symbol at file scope before parameters come into scope); 4 "the
prototype of `X` does not compile against the unit's headers" (honest: types that live
only in `lib.c`, e.g. `collided`'s helpers — the driver redeclares them, the harness
cannot know); 2 "tracing inactive" — a harness bug: `045_strtok` and `029_strcspn` hand
their pointer straight to libc and make no load of their own, so their objects have no
coverage callback to reference; the probe is the canary, the per-object `nm` check is
wrong (replace it with a refusal of `no_sanitize`/`disable_sanitizer_instrumentation`
attributes and `#pragma clang attribute` in the unit's sources, M12's actual concern).

The three harness bugs go into the fix pass with the code review's findings, each with a
regression test; the calibration is re-run afterwards.

## 2026-09-23 — Design B code review (4 lenses, 20 verified findings) and the fix pass

**Review** (workflow: runtime C soundness, Rust orchestration, contracts/replay, tests &
measurement; every blocker/serious finding handed to an independent verifier). Verdicts:
all four lenses sound-with-fixes; 19 of 20 verified findings CONFIRMED serious, 1 refuted
to minor. Every lens independently rediscovered the three calibration bugs. What the review
found beyond them, all fixed with regression tests:
- **Runtime.** Relocation skipped every object whose shadow was not 8-aligned — all
  byte-element parameters — leaving pointers the C stored dangling (RT-2): now at every
  8-byte offset from the OBJECT start, via `memcpy`, and only where the call changed the
  bytes (which also removes the false relocation of untouched data, RT-8). A learn-mode
  fault in a reservation's gap re-faulted until the 120 s timeout writing an unparseable
  record (RT-3): terminal now, and a second fault on an already-opened object too. A
  process ending inside a unit call (a candidate calling `exit`) produced `RH-ERROR`, which
  phase R turned into a harness `Err` — aborting `bench score/check` for every case and
  leaving the old green on disk (RT-4/R2/C-1): now `RH-EXITED` and a `candidate run failed:
  …` red; no `TightEnd` maps to `Err` any more. A run making fewer calls than the table ended
  with a clean `end` (RT-9): now `RH-DIVERGED`. Gap sizing follows B.R-12 (≥ max(1 MiB,
  size)). RT-1 (re-verify the guard pages) was refuted to minor — the capabilities gate
  denies every page-reprotecting route before phase R — and implemented anyway as cheap
  defense in depth (`mach_vm_region` after each call: `RH-TAMPER protection`; the
  restore-before-return variant is the disclosed residual, like the exception-port one).
- **Wrapper.** Prototypes from the interface lines (14 corpus units), file-scope binding of
  the real symbol (`crc16`'s parameter named `crc16`), locals named by parameter index (R6),
  header paths with `"` or `\` refused (R8), the per-object `nm` rule replaced by a refusal
  of instrumentation-disabling source (`no_sanitize`, `#pragma clang attribute`, …).
- **Harness side.** `parse_tight` range-checks every call/object and accepts `exited`
  (R7); the stale detail names the earlier call's symbol and parameter, never a byte (C-6);
  compiler stderr is capped on a line/word boundary so no partial path escapes the
  scrubber (C-10); the migrate judge's `BOUNDARY_EXPLANATION` applies only to the fault and
  retained-pointer shapes (C-8); a C-side `boundary` failure is a harness error like a red
  `driver-shape`, with the regression test that would pass without it (C-4/TM-4); the `rt=`
  digest covers the coverage flags, the measurement policy and the wrapper as rendered for a
  fixed signature, pinned by a golden (C-9/TM-11); `bench boundary` reports VACUOUS (a green
  that guarded nothing, kept out of the tally), per-parameter figures with a "no power"
  flag, and names every widened object (R4/TM-6); a missing case dir is skipped (R9);
  `verify` and the calibration entry point share one input derivation (R11).
- **Tests (TM-1/2/3/7, TM-9, RT-5).** The synthetic unit now has a below-window read that
  only the head layout can catch, a retained pointer, a process exit inside the last call, a
  run with fewer calls, out-param pointer relocation (the driver dereferences what the C
  stored), a libc-only object (widened, named), a late red (`differential-driver`) proving
  the gate where it matters, and one candidate per fail-closed check — signal accounting
  (a handler installed, used and RESTORED before returning), a Mach exception port, an
  opened page. 13 integration tests; 8 boundary unit tests; 3 migrate tests.

**Calibration bugs closed** (wrapper prototypes, symbol shadowing, the tracing rule): the
re-run after the fix pass is recorded in the next entry.

## 2026-09-23 — Design B calibration, re-run after the fix pass (zero tokens)

**Totals (100 cases):** 37 green, 5 red, 10 vacuous, 37 not applicable, 11 skipped, 0 harness
errors. The 17 not-applicable outcomes that were harness bugs are gone: 7 became green and 10
VACUOUS — a passing check that guarded nothing because every object the unit receives is read
only through libc (`strtok`, `strchr`, `strcspn`, `printLine`-style units: their windows are
learned and widened to the whole object), reported as such and kept out of the green tally
(030_integer_underflow_char_min_multiply_lib, 045_strtok_lib, 009_stack_buffer_overflow_lib, 013_poor_quality_addition_lib, 014_dead_code_lib, 016_divide_by_zero_float_lib, 019_integer_overflow_char_max_multiply_lib, 025_struct_and_errno_and_static_lib, 028_strchr_lib, 029_strcspn_lib). The 37 remaining not-applicable are 33 units with no data-pointer parameter and 4 whose
interface types live only in `lib.c`. The 5 reds are the same five, each diagnosed real
(previous entry). 50 (symbol, parameter) pairs are flagged "no power" — the C never leaves
their object untouched or partly touched in any driver call, so the check can only catch an
over-run past the object's extent there; disclosed per case by `bench boundary`. This is the
calibration the design gates opt-in on: 0 false reds in 42 non-vacuous outcomes.

## 2026-09-23 — `read_scalefactors` re-migrated on the boundary-aware oracle (the blind spot closed)

Opted in (`[unit.oracle] boundary = true`) and re-run through the audited hand-off (plain
Sonnet 5 subagents, `targets/tractor/handoff-tools`, `HANDOFF_ROOT` outside the repo). HEAD's
translate prompt differs from the one the old attempt was recorded under, so the harness
opened a NEW trial `a-13c941dfff95` (R-1) rather than a `.rN` sample; the old green
`a-28d8ddc411f9` is superseded by an explicit `superseded.jsonl` tightening entry naming it.
Three rounds, 0 breaches, 0 deviations (17 tool calls, all inside the batch dirs):
1. **translate → red at `bs`** — the fresh Sonnet translation derived every slice length
   from `limit`/`bands` and dereferenced `bs` at entry (its own report says so), exactly the
   pattern of the M4 verified crate; every other check passed (driver output byte-identical).
2. **repair → red at `scfcod`** — `bs` made lazy (an accessor), `scfcod[i]` still read for
   every band; the check moved to the next object.
3. **repair → GREEN** — `scfcod` read only under `ba != 0`, as the C does: "Rust stays inside
   the C's footprint: 31 call(s), 124 guarded object(s) (6 untouched by the C, 86 partially
   touched, 0 widened) … tail and head layouts clean; note: in call 3 the C reads the driver's
   stack through a pointer field (unchecked)" — the disclosed limit, surfaced.
Promoted with `--promote` (re-verified from the recorded evidence, `prompt: conformant`,
nothing sent); `oracle-latest.json` green with `boundary: sancov+guard-pages rt=fe651697`.
The re-migration ran on the UNHINTED prompt (B.R-13): the repair explanation alone carried
the fix in two turns. Gates and the re-baseline follow in the next entry.

## 2026-09-24 — read_scalefactors re-baselined: the blind spot is closed (hidden 16/17, blind spots 0)

`bench check --suite targets/tractor --replay --jobs 6`: `198 reproduce (1 conformant, 197
drifted), 2 expected divergence(s), 0 skipped, 0 problem(s)` — the new `a-13c941dfff95` is the
one conformant attempt (recorded under HEAD's prompt), `a-28d8ddc411f9` is reported as an
expected divergence through its `superseded.jsonl` entry, no regression on unchanged cases.
`bench score --write`: hidden strict-pass 15/17 → **16/17 (94.1%)**, blind spots 1 → **0**,
non-UB vectors 83/87 → 86/87; public unchanged (70/77, 908/950). `read_scalefactors_lib`
moves from `blind-spot` (Rust segfaulted on 3/6 held-out vectors while verified green) to
`strict-pass` 6/6, `migrate_turns` 1 → 3. The M4 briefing's last known blind spot is gone;
nothing else in the suite moved.


## 2026-09-24 — TUI track: §15 research spike (four time-boxed spikes, Sonnet subagents, verified against crates.io/RustSec/repos and this workspace)

Scope per the kickoff: CLI hardening first (writer lock, `migrate --no-promote` + `harness promote`,
cancellation killing sandboxed process groups, `--json` events), then a thin read-mostly
`harness-tui` review cockpit driven from the ledger, with chat in Claude Code through a small
`harness-mcp`. Numbers below are measured (`cargo tree -e normal --prefix none | sort -u | wc -l`
in scratch projects; RustSec queried per crate, the query validated against a crate known to have
advisories). Workspace today: edition 2021, `rust-version = "1.85"`, toolchain 1.94.1, no tokio,
no crossterm/ratatui/diff/highlight crate; `tree-sitter 0.27` already in the graph.

**Spike 1 — TUI stack.** Options: ratatui 0.30.2 (2026-06-19, active; split into ratatui-core/
-widgets/-macros at 0.30) on crossterm 0.29.0 (crates.io 2025-04, repo commits 2026-09; the
crates.io release just hasn't been cut) — 61 unique crates for the base; termion (unix-only, no
event-stream parity), termwiz (wezterm's weight), cursive (stale since 2024-08), iocraft 0.9
(new, React-like, credible but immature), tui-realm (framework overhead). Helpers: similar 3.2
(+1 crate) over imara-diff/diffy; tree-sitter-highlight 0.27 (+9, fewer here since the core is
already resolved) over syntect 5.3 (+31 via onig/fancy-regex) and synoptic (stale); editor widgets
(tui-textarea stale, edtui +56) rejected — the cockpit is read-mostly and every write goes through
the CLI; tui-scrollview (+7) only if `Paragraph` scroll offsets prove insufficient. No advisories on
any candidate. Full recommended combo: 78 unique crates, `cargo audit` clean over 109. **Chosen
default:** ratatui 0.30 (`default-features = false`, `crossterm`) + crossterm 0.29 + similar +
tree-sitter-highlight, in a separate `harness-tui` crate so the `harness` CLI's own tree does not
grow. **MSRV:** ratatui 0.30 declares 1.88 and tree-sitter-highlight 0.27 declares 1.90; the
workspace's `rust-version = "1.85"` is bumped to 1.90 when `harness-tui` lands (ratatui 0.29 at
MSRV 1.74 is the fallback if 1.85 must hold). **Revisit when** ratatui ships another breaking
release, crossterm cuts a new release, or the cockpit grows in-place editing.

**Spike 2 — side-by-side review presentation.** Surveyed delta, difftastic, diffnav, gitui,
lazygit, tig, jj's `scm-diff-editor`, git-split-diffs, vim diff mode, Claude Code, aider, Codex CLI,
opencode/crush (accept/modify specifics UNVERIFIED), and GitHub/Gerrit/Phabricator/Reviewable as
interaction references. Findings that transfer: difftastic aligns by syntax node, i.e. "align by
function" is a known-good idea; unequal-height sides are padded with filler lines (vim, GitHub);
lazygit's `E` (edit this hunk in `$EDITOR`) is the precedent for a labelled hand edit; Codex keeps
"propose/apply" and "review" as separate modes; none of the terminal tools attach a note to an
accept/reject, and crush buries diffs in the chat stream — both are what the cockpit exists to fix.
**Chosen default:** a vertically stacked list of function PAIRS (C left, Rust right, filler-padded
to equal height) with a thin persistent rail of functions carrying per-function verdict dots; the
function boundary is the unit of navigation (`]f`/`[f`), no synced scrollbars; below a measured
column threshold (explicit override flag; difftastic's width auto-detection regressed three times)
collapse to unified-stacked C-then-Rust per pair. Keys: `j/k`, `]f/[f`, `a` accept (only when
green), `m` steer note, `e` labelled hand edit, `v` verdict detail. **Revisit when** the
function-to-symbol mapping stops being 1:1.

**Spike 3 — CLI hardening mechanisms.** (1) Writer lock: fd-lock 4.0.4 (flock, RAII, +3 crates:
rustix/bitflags/errno; released by the kernel on crash) over fs4 1.1 (same backend, non-RAII API —
the fallback), fslock/file-lock (fcntl record locks are process-scoped: closing any fd drops the
lock), and the existing hand-rolled `create_new` `BenchLock` (harness-oracle/src/bench.rs:612 —
no staleness detection, tells the user to delete it). Lock file `migration/.lock`; pid/host written
for diagnostics only. (2) Process groups: already done — exec.rs:355 `process_group(0)`, group kill
via `/bin/kill -KILL -- -<pgid>` (exec.rs:365), tested, and `sandbox-exec` execs in place so the
pgid survives. The MISSING piece is CLI-level SIGINT/SIGTERM handling: because children lead their
own groups, a terminal Ctrl-C reaches only the CLI (cargo-nextest has the same shape and
re-broadcasts). signal-hook 0.4.4 (+2) over ctrlc 3.5 (+5 on macOS via nix + the objc2/dispatch2
bridge, single handler) and tokio (not in the tree; harness-llm is synchronous). (3) `--json`
events: NDJSON on stdout, zero new deps (serde_json present), header line with the SCHEMAS.md
envelope, `k`-discriminated events, human logs on stderr — cargo's `reason` NDJSON and Claude Code's
`stream-json` as the models; `gh --json` is a snapshot shape, right for status queries only.
(4) Promotion today (harness-cli/src/main.rs:1090-1160) stages `.promote-<id>/`, two-rename swaps,
re-verifies IN PLACE and rolls back on non-green, with evidence-based crash recovery
(`recover_interrupted_promotion`); a standalone `harness promote <unit> <attempt>` relocates that
block unchanged. Gotcha: drivers have a second, simpler promotion path (gen_driver.rs) without crash
recovery. **Revisit when** fd-lock goes 2 years without a release, ledgers move to network storage
(flock/NFS), tokio enters the tree, or attempts get GC'd before promotion.

**Spike 4 — MCP server.** rmcp 3.4.1 (official SDK, 2026-09-23, MSRV 1.88) needs tokio and adds
56 crates for a stdio-only server (its RUSTSEC-2026-0189 is HTTP-transport only); rust-mcp-sdk 2.0
(active, unofficial) as fallback; mcpr/mcp_rust_sdk/mcp-server abandoned. **Chosen default:**
hand-rolled JSON-RPC 2.0 over stdio, zero new crates (~150–300 lines): protocol version 2025-06-18,
newline-delimited, nothing but protocol on stdout; `initialize`, `notifications/initialized`,
`tools/list`, `tools/call`, `ping`, `-32601` otherwise — all Claude Code requires (`claude mcp add`
/ project `.mcp.json`). **Revisit when** a second transport or OAuth is needed, or tokio enters the
workspace for another reason.

**Direction (no change):** the spikes confirm the brief; the only new decision is the MSRV bump.
Total new crates for the whole track: fd-lock, signal-hook (CLI, +5 unique); ratatui+crossterm,
similar, tree-sitter-highlight (harness-tui only). Sources: crates.io API and docs.rs pages for
every crate named; RustSec advisory-db (GitHub API + local clone HEAD 2026-09-14); ratatui v0.30
release notes; github.com repos of difftastic (issues #693/#1064), lazygit keybindings, gitui,
diffnav, git-split-diffs, crush; jj docs; neovim diff.txt; developers.openai.com Codex review docs;
code.claude.com docs (mcp, headless); modelcontextprotocol.io spec 2025-06-18 transports;
nexte.st signal-handling design; doc.rust-lang.org cargo external-tools; rust-lang/rust#93857.

## 2026-09-24 — CLI hardening: design and adversarial design review (15 confirmed, 0 refuted)

`docs/CLI-HARDENING.md` designs the milestone the review cockpit needs from the CLI: a writer
lock, `migrate --no-promote` + `harness promote`, cancellation that kills sandboxed process
groups, and a `--json` events mode. Four review lenses (concurrency, crash consistency &
signals, contract & consumers, dependencies & security), every finding attacked by an
independent verifier against the code: 15 confirmed, 0 refuted; §R of the design holds each
resolution. The ones that changed the design's direction:
- **fd-lock dropped** — std `File::try_lock` (Rust 1.89) is the same `flock(2)` call with zero
  crates; the workspace `rust-version` moves 1.85 → 1.89. `signal-hook` stands (std has no
  signal API). The spike had missed std's lock because it considered only `O_EXCL` as the
  std option — recorded here so the next spike checks std first.
- **A try-lock "holder" probe is a bug** (it takes the exclusive lock for microseconds and can
  fail a real writer with a dead pid in the message): readers only ever READ the holder line.
- **The promotion protocol was not crash-proof** even before this design: recovery was keyed on
  `.<crate>.prev`, which a FIRST promotion never creates and which is removed before the
  attempt's `promoted` flag is written. One marker now spans the protocol, the green tail is
  idempotent in a fixed order, and `recover_promotion` resolves every marker by evidence.
- **Cancellation must be a state the spawn choke point observes**, not just a group kill: a
  SIGKILLed child reaped by the 50 ms poll would be journaled as a `crash-timeout` turn and
  close the attempt red — evidence manufactured by Ctrl-C. `Error::Interrupted` is never a
  `ChildEnd`; `built_with_env` gains the outer/inner result shape `tool_outcome` has.
- **The harness must die BY the signal** (`emulate_default_handler`), not exit 130: measured on
  this machine, interactive bash 3.2 and zsh 5.9 continue a `for u in …` loop after a child
  that exits 130 and abort it after a child that dies by SIGINT.
- **`harness promote` binds the record to the current inputs** (`unit_source` + `driver`, the
  R-5 provenance rule) and re-establishes migrate's preconditions (plan staleness, R6 driver
  freshness); `[llm.migrate] promote_on_green = false` makes "Accept = explicit act" sticky
  across the several invocations one `external` attempt takes.
- **Events carry ledger values verbatim** (the Turn's closed `result` set, a `UnitReport` struct
  shared by the human line, the event, the cockpit and the MCP bridge) and machine `kind`s come
  from typed errors (`Locked`, `Stale`, `Awaiting { attempt }`, `Interrupted`), not prose
  matching; the lock's in-place write follows the symlink discipline of every other write.
Follow-ups (not this milestone): `verify` lacks the R6 gate; driver-attempt Accept; queueing on
contention; an async client for cooperative cancellation.

## 2026-09-24 — CLI hardening IMPLEMENTED (writer lock, `promote`, cancellation, `--json` events)

Built against the reviewed design (`docs/CLI-HARDENING.md`, now "IMPLEMENTED"), then a
4-lens adversarial code review (11 confirmed, 0 refuted; §R) and a fix pass with regression
tests. What landed:
- **Writer lock** — `harness_core::ledger::WriterLock` on `migration/.lock` via std
  `File::try_lock` (zero crates; MSRV 1.85 → 1.89): symlink/hard-link-proof open protocol,
  holder line written under the lock and truncated on clean release, readers only READ it,
  `Error::Locked` fail-fast with the holder named. Every writing command takes it; `bench
  score|check` lock every selected case up front and hold across both passes; `bench
  boundary|init` per case. `BenchLock` migrated to the same mechanism (no more "remove it
  yourself"). `state status` is two-phase and reports `write in flight` only for a LIVE holder
  (`kill -0`) — every signal death leaves a dead holder's line, and the review caught that it
  would have masked real contradictions.
- **Promotion** — `promote_attempt` with one marker for the whole protocol, an idempotent
  green tail (verdicts → status → `promoted` → `.prev` → marker last), `recover_promotion` by
  evidence (finish or roll back; a FIRST promotion is now covered), `migrate --no-promote`,
  `[llm.migrate] promote_on_green`, and `harness promote <UNIT> <ATTEMPT> [--replace]` with the
  full refusal list (clean id, record id/unit, migrate record, green, digest, binding to the
  current `unit_source`+`driver`, plan staleness, R6 driver freshness). A rolled-back
  promotion reports its checks but never a `verdict` line (nothing was stored).
- **Cancellation** — `exec.rs`: `CANCELLED` + a registry of live process groups; spawns happen
  under the registry lock; a child that ends after the cancellation is `Error::Interrupted`,
  never a `ChildEnd` (`built_with_env` gained the outer/inner result shape); the handler kills
  every live group, writes its courtesy output from a helper thread with a 250 ms budget (a
  stalled `--json` consumer or a closed stderr must not stop it), then dies BY the signal
  (`emulate_default_handler`). SIGHUP is registered only when the output is a terminal, so
  `nohup harness … &` keeps its inherited `SIG_IGN` — both pinned by e2e tests (stalled pipe;
  `sh -c 'trap "" HUP; exec harness …'`), and the cancellation e2e now watches a spinning C
  driver named `drv_c` die.
- **`--json` events** — `report.rs` (process-global mode, whole-line writes under a mutex),
  the `ruharness-events` v1 stream (header/result, message, facts/unit via
  `harness_core::status::UnitReport` shared by the human line and the event, turn-start/turn-end
  from a `harness_llm::progress` sink the trajectory reports to, attempt with the promotion
  reason, check/verdict — including migrate's final judged turn at `attempt-verdict.json` —,
  promote, awaiting with the exact resume command, error with typed kinds from
  `Error::{Locked, Stale, Awaiting, Interrupted}`). `run_triage` carries the typed `Awaiting`
  through its batch fold (the review caught it re-stringifying it).
- One new crate: `signal-hook` 0.4.4 (+`signal-hook-registry`, `errno`). Deviation from the
  design recorded: the progress sink is process-global (installed once under `--json`) rather
  than a `MigrateParams` field — 16 construction sites, same semantics.
Gates: fmt/clippy clean, `cargo test --workspace` green (465 tests incl. the three hardening
e2e tests), `bench check --suite targets/tractor --replay --jobs 6`: `198 reproduce (1
conformant, 197 drifted), 2 expected divergence(s), 0 problem(s)`, OK — no regression.
Follow-ups (from both reviews): `verify` lacks the R6 gate; driver-attempt Accept; queueing on
contention; an async client for cooperative cancellation. Next: the `harness-tui` design.

## 2026-09-24 — harness-tui: design and adversarial design review (20 confirmed, 0 refuted)

`docs/TUI-DESIGN.md` designs the review cockpit and the two CLI additions its acts need
(`migrate --steer --from`, `harness override`). Four review lenses (ledger & replay, read model
& pairing, process & acts, scope), every finding verified against the code: 20 confirmed, 0
refuted (9 verifiers hit a session limit and were resumed from the journal). What changed:
- **A steer attempt's evidence must come from committed evidence only.** The existing repair
  rendering appends a driver-output excerpt read from gitignored `migration/build/<unit>/`,
  which belongs to whatever ran last; a steer turn rendered from it would show the model
  someone else's output and give the attempt an id that depends on scratch. The steer turn
  renders from the seed's stored `attempt-verdict.json` without the excerpt — deterministic,
  because stored verdicts carry no machine paths (checked: 0 of 103 committed).
- **The first turn is a job property** (`FirstTurn::{Translate, Steer}`), the record carries
  `seeded_from` + `steer_note`, and every verification path loads the record first — the old
  engine hard-codes turn 1 as `translate` and renders HEAD's translate request.
- **`--from` is required**: the ledger defines no order over a unit's attempts (content ids, no
  timestamps), so "the latest attempt" does not exist.
- **A hand edit must never score as the pipeline's.** scores.json carries no provider, so a
  human attempt that satisfied R-5 would count as strict-pass; R-5 moves into harness-core as
  `attempts::provenance` with a `Human` outcome that `bench` reports as a PROBLEM. `override`
  judges exactly `logic.rs` + `ffi.rs` through the one migrate judge (a verbatim crate copy
  would bypass the harness-owned manifest and lint structure) and refuses a byte-identical
  edit (which would make provenance ambiguous).
- **No automatic spawn**: the resume watcher only marks "response present"; `R` asks and
  re-spawns the TUI's own argv (the event's `resume` string is a human hint without `--json`).
- **The cockpit's child runs in its own process group** (a terminal hangup would otherwise
  kill the harness by the default action — its SIGHUP handler is off under pipes — orphaning
  sandboxed groups; reproduced), the TUI has its own signal path, and it reloads only after
  reaping (on a signal the CLI emits `result` before dying with the lock line; `kill -0` reads
  a zombie as alive).
- **Pairing**: shims found in every source file (the M0 crate has an inline `mod ffi`), callees
  resolved through `use` imports and aliases; C spans sliced only when the file still matches
  the scan; tabs expanded at render (both corpora are tab-free, so a synthetic fixture pins it).
- **Shape**: `harness-tui` is a library (the read model, for harness-mcp) plus a binary behind
  the default `tui` feature; `default-members` keeps a plain root `cargo build` lean.

## 2026-09-24 — review acts implemented (steer, override, provenance in core); harness-tui read model; session split

**Step 1–2 of docs/TUI-DESIGN.md §8, implemented and tested:**
- harness-core: `attempts::{current_binding, unit_crate_digest, provenance}` — the ONE
  implementation of R-5 (`Provenance::{None, Pipeline, Ambiguous, Human}`; a steer attempt that
  reproduced its seed collapses into the seed); `AttemptRecord.{seeded_from, steer_note, note}`
  (additive, omitted when absent); `UnitReport.promotion_interrupted` (reported instead of a
  contradiction while a promotion marker waits for recovery and no live writer holds the lock).
- harness-llm: `FirstTurn::{Translate, Steer}` on the job; the steer turn rendered from the
  seed's committed candidate + stored verdict only (never the build-dir excerpt), with
  `[GUIDANCE]` after `[HISTORY]` on EVERY turn of a steer attempt (a stateless repair must not
  lose what the reviewer asked for — a small extension of the design, which only said "then
  ordinary repair turns"); verification builds the first turn from the RECORD; the
  evidence-only drift rule covers a steer first turn; `record_human_attempt` through the
  migrate stage's one judge. New prompt fixtures `migrate-steer-{red,green,repair}.txt`,
  reviewed; every translate/repair fixture unchanged.
- harness-cli: `migrate --steer --from` (required together; the refusal lists the seeds),
  shell-quoted `resume` + additive `awaiting.args`, `harness override` (exactly logic.rs +
  ffi.rs; shape, symlink, size and identical-source refusals), bench on the core provenance
  rule with a PROBLEM for a human-promoted crate and `skipped (human)` in `--replay`.
- Tests: core provenance outcomes; steer rendering (no excerpt, byte-identical across a
  build-dir rewrite, same id), seed refusals, conformant replay of a steer attempt from its
  record, human attempts red/green/check/harness-error; e2e `steer_override.rs` (a note with
  quotes, `&` and `$x` resumed through `sh -c "$resume"` finishes the SAME attempt; override
  refusals; human label; promote).

**Step 3 (harness-tui library) implemented:** `crates/harness-tui` with `model` (Snapshot,
UnitView with ordered attempts and `ProvenanceView`, facts freshness), `pairs` (facts-guarded C
spans; tree-sitter-rust shim/callee location in any file, through `use` aliases), `display`
(tab stops, control and bidi characters to `?`, char-boundary cut); tested on the committed
tractor and zopfli ledgers. The `tui` feature's dependencies are declared (ratatui 0.30 with
`crossterm_0_29`, similar, tree-sitter-highlight, tree-sitter-c, signal-hook) but the front end
is not written: the binary prints that and exits 2. Workspace `rust-version` 1.90;
`default-members` leaves harness-tui out of a plain root `cargo build`.

**Session split (user, 2026-09-24):** the terminal front end (§8 step 4) moves to a fresh
session; `docs/NEXT-SESSION.md` is its kickoff. The adversarial code review of step 1–2 ran at
the end of this session: 16 findings confirmed, 0 refuted, 9 distinct — the worst: a steer
seeded from a human attempt would launder a hand edit into `Pipeline` provenance, and a steered
crate is indistinguishable from unassisted pipeline output in scores.json. They are recorded,
OPEN, in docs/TUI-DESIGN.md §R2 (full text: docs/reviews/2026-09-24-steer-override-code-review.md);
the next session's first task is their fix pass.

**Privacy prompt (user report):** the oracle test `sandbox_denies_network_home_reads_and_stray_writes`
ran `/bin/ls $HOME` UNSANDBOXED as its control; `ls` stats every entry, so each `cargo test
--workspace` touched `~/Music` and network shares mounted in the home folder, and macOS asked
for Apple Music and network-volume access. The control now lists `~/.cargo` (else `~/.rustup`,
else `ls -d ~`), which proves the same sandbox denial without touching privacy-protected folders.


## 2026-09-24 — §R2 fix pass (steer/override/provenance), and its own review

The nine findings of the step 1–2 code review (docs/TUI-DESIGN.md §R2), each fixed with a
regression test that fails without it (27 rule-guarding mutations run, every one killed after
two tests were strengthened; one survivor showed a check that another check already made —
the duplicate was removed so each rule lives in one place). What changed:
- **Provenance is authorship, not provider kind.** `attempts::authorship` follows `seeded_from`
  through every record of the unit (bounded, so a hand-edited cycle cannot loop): an unseeded
  model attempt is pipeline output; a steer attempt whose chain reaches a `human` attempt IS
  that hand edit (one hop or several, red or green seed); any other steer attempt is `Steered`.
  **Decision (finding 2, the one design question):** a steered crate is a bench PROBLEM, like a
  hand edit — the note is free human text that can carry the fix, or the TRACTOR vectors the
  model must never see (M4-DESIGN §2), so scores.json holds unassisted pipeline output only. An
  unassisted model attempt with the same candidate still outranks both. Every other pipeline
  figure (`migrate_outcome`, the scored unverified candidate) counts unassisted attempts only.
- **Steer fields are integrity-bound.** New additive `seed_verdict` (blake3 of the seed's
  `attempt-verdict.json` bytes); `replay_divergences` — the one function every verification
  path passes through — requires the record's `(seeded_from, steer_note, seed_verdict)` to equal
  the job's first turn, and `recorded_pairs` requires the recorded turn 1 to be of kind `steer`
  and to pose the note as `[GUIDANCE]` before `[TASK]` with `[HISTORY]` naming the seed (and a
  non-steer record's to pose no `[GUIDANCE]`). Adding, removing or editing the fields, or the
  seed's verdict, is an integrity error, never drift or a divergence.
- **Human attempts.** An override killed mid-judge leaves an `in-progress` record under the
  edit's content-derived id: the same edit now reclaims it (`reset_unfinished`); a finished one
  stays refused; an unfinished MODEL record under a human id is refused as inconsistent. A
  deny-scan red prints its violations and keeps the two files in `attempts/<id>/edit/src/`, so
  every human attempt holds what its `response_hash` hashes.
- **Hyphen-leading notes.** Every `awaiting` hint (migrate, observe, gen-driver — the last one
  also lost its run flags before) attaches its values (`--steer='- keep it'`); clients pass
  `--steer=<note>`/`--note=<text>` as one argv element. Round-trip tests through `sh` and clap.
- bench's provenance, pipeline figures and replay skip are pure helpers with unit tests.

**The fix pass's own review** (three lenses, 8 findings, 7 confirmed, all low, 1 refuted):
doc comments stranded on the wrong functions, the scores.json contract docs and two design
sentences still describing the old rule, the §4 hand-edit `--note=` that the fix had dropped,
a pinned replay blaming the seed's verdict for an edited `seeded_from` (the recorded evidence
is now checked first, and the message names both possibilities), the observe/gen-driver hints,
and a reclaim test that passed without `reset_unfinished` — all fixed.

**Gates.** fmt, clippy `-D warnings`, `cargo test --workspace` green. `bench check --suite
targets/tractor --replay --jobs 6`: replay `198 reproduce (1 conformant, 197 drifted), 2
expected divergence(s), 0 skipped, 0 problem(s)` — identical to the baseline run at the start
of this session. One run under a load average of 33 (mutation runs, review agents and a pty
test in parallel) reported `011_static_dag_lib: driver no longer validates`; the same
`validate_driver` call is green 3/3 in isolation (4.9 s each) and on the quiet re-run below —
driver validation has timing-sensitive checks, so the gate is only meaningful on a quiet
machine.

## 2026-09-24 — harness-tui: the terminal front end (TUI-DESIGN §8 step 4)

Built against the reviewed design, then a four-lens adversarial code review (27 confirmed, 0
refuted — §R3), a fix pass, a verification of that fix pass (4 partial fixes, 18 new issues
confirmed — also in §R3) and a second fix pass. Every fix has a regression test; the
rule-guarding ones were mutation-checked (27 mutations across the two passes, all killed
after two tests were tightened).
- **Library, for every client** (no terminal crates, reused by harness-mcp): `events` (the
  typed `ruharness-events` reader; unknown `k` kept as `Other`, a non-JSON line as `NotJson`;
  lines bounded at 1 MiB) and `spawn` (the child in its own process group, stdin null, two
  reader threads, over only after BOTH pipes hit EOF AND it was reaped, `/bin/kill -INT` only
  while unreaped, the signal path's interrupt-and-wait). Event fixtures were recorded from the
  real CLI on a zopfli copy (migrate awaiting/green, promote rolled back, scan locked).
- **Front end** (`tui` feature): `highlight` (tree-sitter-highlight over both grammars' own
  queries; plain on failure), `app` (state; keys return commands; every act shows its exact
  argv and needs a plain `y` to a prompt drawn whole with no input pending; notes checked
  against the CLI's rules), `view` (grapheme-exact widths, only visible rows built, windowed
  rail, failures-first verdict strip, overlays scrolling by wrapped rows), `handedit`, `main`.
- **A hand edit is never lost** — the rule the two review rounds converged on: the temp dir
  goes only when the override recorded it (its `attempt` event), it changed nothing and left
  nothing, or on an explicit armed `D`; everything else keeps it, `E` offers it again, exits
  print where it is, TERM/HUP while the editor runs are forwarded to it and the cockpit dies
  only once it is gone.
- **What the tests found that reviews did not:** the pty end-to-end (`script`, keys, a
  spinning C driver, SIGHUP to the cockpit's group) failed on its first run —
  `Terminal::clear` asks the terminal for the cursor position, a terminal that never answers
  failed the hand edit after the editor exited, and the error path then deleted the edit.
  The resume now repaints through `resize`, and staging precedes any terminal call.
- **No new crates.** serde_json became a plain dependency of the library (already in the
  tree via harness-core). ratatui's crossterm has bracketed paste on by default.
- **Recorded, not done:** the editor's own crash (not a signal) while the cockpit is killed
  -9 is out of reach (nothing runs); a hand-edit override the user quits away from may still
  record the kept edit (said on exit).

## 2026-09-24 — harness-mcp: design and adversarial design review (20 confirmed, 1 refuted)

`docs/MCP-DESIGN.md` designs the stdio MCP server (hand-rolled JSON-RPC 2.0, MCP 2025-06-18,
zero new crates, reusing `harness_tui::{model, events, spawn}`). Three lenses (protocol,
trust, contract), every finding verified; §R holds the resolutions. The one that reshaped it
(high, found by all three lenses): an `external` hand-off answered by the chat agent — which
can read the repo, the held-out vectors and the conversation — would be recorded as blind,
unassisted pipeline output and scored, bypassing the audited blind protocol (M4-DESIGN §R R2).
So v1 poses **steer attempts only** (authorship `Steered`, a bench PROBLEM), retries only in
the record's own run shape (never an unseeded `external` one), promotes existing attempts,
and has no verify / fresh-migrate / hand-edit tools. The rest: provider list and targets are
server policy (`--provider`, `--target-root`), progress notifications against the client's
30-minute idle abort, a shutdown path that interrupts the child, one act in flight with `busy`
refusals, untrusted values wrapped in `structuredContent`, size caps. Not implemented yet —
the next session's task (docs/NEXT-SESSION.md).

## 2026-09-24 — Direction change (user): the cockpit becomes a user-friendly wrapper with chat inside

After trying the cockpit, the user set its direction (recorded in full in docs/TUI-DESIGN.md
§9; nothing built yet):
- **Audience:** people who do not navigate with vim-style keys — "if we wanted that, we could
  just stay in the CLI". The TUI is a discoverable wrapper: arrow keys and the mouse, `Enter`
  / `Esc` / `Tab`, menus and on-screen hints; vim keys at most optional aliases.
- **Navigation by file:** a file tree of the target as the main navigator.
- **Deterministic actions on `Enter`** (scan/parse, units, detectors, plan, verify, promote,
  status) run behind the scenes and report in plain language; the exact command stays one
  keypress away.
- **Chat inside the cockpit, like Copilot**, for model-driven work: "migrate this" is asked in
  chat, which kicks it off through harness-mcp and a project skill that knows the workflow.
- **This reverses** the §15 spike's "chat lives in Claude Code through harness-mcp, not in the
  TUI": the chat pane will embed an existing agent runtime headless (Claude Code's streaming
  mode first — the user's existing access, no API key; a spike must confirm its current
  flags, auth and permission prompts), with harness-mcp as its tools. Everything else holds:
  the ledger is the truth, every write is a spawned CLI command, nothing runs without the
  user's confirmation, provenance stays honest (a chat-requested migration is labelled and
  never scored as unassisted), no second agent runtime, no unvetted crates.
- **Order proposed:** (1) the wrapper UX design → review → build on today's engine;
  (2) harness-mcp, adding the requester label for chat-requested migrations (MCP-DESIGN §7);
  (3) the chat pane after its spike, plus the "migrate this" skill.

## 2026-09-24 — harness-mcp: the stdio MCP server (MCP-DESIGN, built, reviewed three times)

`crates/harness-mcp` (one binary; harness-core, harness-tui without its terminal front end,
serde_json, signal-hook — **zero new crates**, 51 packages in its tree, all already locked; a
default workspace member), built against the reviewed design; then a four-lens adversarial code
review (protocol & process, trust, contract, tests: 33 confirmed, 0 refuted — docs/MCP-DESIGN.md
§R2), a fix pass, a verification of that fix pass by three checkers (20 more confirmed: 3
partial fixes, 5 test gaps, 12 new — §R3), a second fix pass, a check of the second pass (8
more, 2 medium — §R4) and a third, short pass. Every rule-guarding fix of the three passes was
mutation-checked. The findings shrank each round (33 → 20 → 8), and none after the first
touched the central rule.
- **Protocol**: hand-rolled newline-delimited JSON-RPC 2.0, MCP 2025-06-18 always; 1 MiB lines
  read without buffering past the cap; batches `-32600`; unknown tool / bad arguments `-32602`
  from ONE schema definition that is both `inputSchema` and validator; loose `outputSchema`s (a
  client validates `structuredContent`, refusals included). One act in flight, `busy` refusals,
  reads answered meanwhile, `notifications/cancelled` → the act's process group interrupted and
  no response, progress only with a token (closed values only; a 30 s heartbeat), shutdown on
  EOF / stdout failure / SIGINT-TERM-HUP (dies by the signal) / a panic (non-blocking hook, exit).
- **Tools**: `harness_status`, `harness_unit` (+ `symbol`, every attempt id), `harness_steer`,
  `harness_answer` (new in the fix pass), `harness_retry` (steer attempts only; an `external` one
  names its model), `harness_promote`; `harness_status` pages its units with `after`.
- **Decision (the fix pass's one design change): `harness_answer`.** The review showed the
  design's file-based answering needed `traces/` writable, so a chat agent could answer a pending
  BLIND hand-off by file and the CLI would record it as unassisted pipeline output (TRUST-1,
  CONTRACT-1) — and a retry refusal even invited it. Now the server answers only the hand-offs
  its own acts posed (remembered per target and attempt: response file, argv, answering model),
  only when the caller names the attempt's model (a name check, stated as such), writing the
  response atomically (temp + hard link, never over one) with token counts 0 (a guessed count
  below the prompt size voids the turn). The recommended deny list closes all of `migration/**`
  to the runtime's file tools; pending blind hand-offs (migrate AND driver) are counted in the
  status head, flagged on their unit and refused by every tool; `harness_retry` refuses every
  unseeded attempt, of any provider (TRUST-7: a live retry would record fresh pipeline output
  the chat selected). A shell can still write any file — the remaining, stated edge; the README
  also says not to serve the benchmark's case trees to chat (one holds a pending blind hand-off).
- **Other decisions**: a JSON value is plain only in its closed set or the harness's exact
  attempt-id shape (unit ids, profile names, check names wrapped — TRUST-4), except the values
  `answer_with` hands back (a model name, an attempt id), plain in their shape (VC-2); a preflight
  before every read AND act of every file the read model reads or hashes (no symlinked ledger
  files — a TOML parse error quoted a linked `~/.env` line —, paths that stay inside the target,
  regular files — a FIFO facts path hung the server —, caps: records and verdicts 1 MiB,
  facts/plan/drivers/sources 64 MiB (a deliberate deviation from the reviewed 1 MiB: they grow
  with the project), crate trees 32 MiB, 256 MiB held, 4 GiB hashed per read); a 48 KiB result
  budget written outcome first (Claude Code passes 100 000 characters / 25k tokens of text to the
  model and ignores `structuredContent`), oversized items skipped, failed checks before passes,
  attempt ids last; read WORK bounded as well as bytes (≤ 50 000 facts files, units × facts ≤ 50
  million, plan bytes × units ≤ 4 GiB, 4 GiB hashed — checked after the facts pass too), with
  `Facts::include_closure` indexed in harness-core (linear, the same result) and `pairs` reading
  each C file once per call in harness-tui;
  `spawn::interrupt` signals the child's process GROUP (harness-tui's library, so the cockpit
  too), reader threads via `Builder` (never a panic under the slot lock); the note rule moved into
  `harness_core::attempts::note_problem` (harness-llm's `validate_note` delegates, byte-identical)
  so the server checks notes with the CLI's own function.
- **Relation to the direction change recorded in parallel this session** (the entry above):
  this session ran the kickoff it was given — harness-mcp before the wrapper UX. The server is
  the chat pane's future tool surface as built; its rule (what chat contributes is labelled
  Steered, never scored) is the direction's "provenance stays honest". Still to add for "migrate
  this" from chat: the requester label (MCP-DESIGN §7) — this server poses no fresh translation
  until a ledger label records who asked.
- **Revisit when**: a real target's read needs more than 4 GiB of hashing or blocks the loop
  noticeably → reads on a worker thread; a client sends no progress token for long acts → an
  async run-id shape (§7); a ledger label records the requester → a fresh-translate tool for
  live providers (§7).
- **What the tests found**: the e2e's first spinning-driver plan failed on the oracle's
  driver-shape gate (a driver may not call `access`), so the spin moved to zopfli's own `main`,
  which the whole-program check runs (`whole_c`/`whole_mixed`) outside u001's binding; the first
  e2e promote was refused because zopfli's u001 is already verified (`replace`); a shutdown test
  raced the fake's `trap` under parallel load (it now waits for the fake's first event). A failed
  mutation run left a fake spinner running — found by a checker, killed, and the in-process tests
  now kill their fakes' groups (and remove their temp dirs) on drop; a test the second pass
  added waited for "any progress" while a 100 ms heartbeat could come first, so under the
  replay's load its fake was interrupted before its `trap` existed — it now waits for the fake's
  own event.
- **Mutation checks**: first pass 38 reverts — 34 killed, the 4 survivors showed tests that
  could not see their rule (a colliding file name, a fake silent after SIGINT, spoils the loaders
  reject anyway), strengthened → 38/38. Second pass 18/18, third pass 7/7. Not mutation-covered
  (stated): the atomicity of the response write, `Builder`'s error path and the non-blocking
  panic hook (no way to induce them in a test), and the retry's answering model taken from the
  caller (equal to the record's by the check before it).
- **Gates**: fmt, clippy `-D warnings` (workspace, all targets), `cargo test --workspace` 611
  passed (harness-mcp: 61 unit, 4 protocol, 1 end to end). `bench check --suite targets/tractor
  --replay --jobs 6` at the start of the session and again on the final code: replay `198
  reproduce (1 conformant, 197 drifted), 2 expected divergence(s), 0 skipped, 0 problem(s)`,
  `bench check: OK — no regression` — identical.

## 2026-09-24 — Reconciliation of the parallel sessions (harness-tui front end ∥ harness-mcp)

Two sessions worked on 2026-09-24 in parallel: one built the cockpit's terminal front end and
recorded the user's direction change, the other built harness-mcp on top of it. Checked
afterwards, from `main` at 4c140b7:
- **Nothing lost.** `cargo test --workspace` 611 passed, 0 failed; the cockpit's pty tests
  (`a_hangup_cancels_the_running_harness_and_its_sandboxed_group`,
  `a_terminated_edit_is_never_lost`) pass.
- **The MCP session's changes to harness-tui** (`git diff a02e79d 4c140b7 -- crates/harness-tui`):
  `spawn::interrupt` now INTs the child's process GROUP (`/bin/kill -INT -- -<pgid>`), reader
  threads start through `Builder` (a failure kills and reaps the child instead of panicking),
  a non-blocking `try_interrupt` for panic hooks; `pairs` reads each C file once per call (same
  results: a stale or unreadable file is still "stale"). The cockpit's `x`, `Q` and its own
  signal path all go through `spawn::interrupt`, so they now reach the whole group — which
  TUI-DESIGN §4 already said. The group holds only the CLI: every process the oracle spawns
  leads its own group (`harness-oracle/src/exec.rs`, `process_group(0)`), so the CLI still
  cancels its sandboxed groups itself.
- **The cockpit opened once** in a 120×40 pty on the read_scalefactors case: split layout, the
  pair, the green verdict; `q` exits 0 and restores the terminal (paste off, cursor, alternate
  screen). Seen in passing, for the redesign: the bottom key-hint line is cut at the right edge.
- **Docs reconciled**: TUI-DESIGN status, §0, §1 (the library now holds `events` and `spawn`,
  and harness-mcp depends on it) and §9's order (harness-mcp is built; its requester label
  remains); README: the cockpit and harness-mcp sections now say how they work together today
  (a separate agent session, `g` to re-read after an act from chat, the writer lock between
  them) and where the cockpit is going; CLI-HARDENING's "next milestone". Stale code comments
  (the `Cancel` doc comment in app.rs, the `tui` feature comment in Cargo.toml) are left for the
  wrapper build.

## 2026-09-24 — Cockpit wrapper: §15 spike (approachable TUIs; mouse and trees in ratatui)

Time-boxed, three subagents (UX patterns from live sources; ratatui/crossterm mechanics from
the locked crates' sources; a map of today's engine), premises re-checked by hand. Findings
that shaped docs/COCKPIT-WRAPPER-DESIGN.md:
- **Arrow/Enter/Esc/`?`** is chess-tui's scheme (github.com/thomas-mauran/chess-tui); a
  persistent, focus-dependent hint bar is zellij's answer to "nothing to memorise"
  (github.com/zellij-org/zellij/discussions/1270); gitui keeps quit unambiguous (`Esc` closes
  a popup, `q` quits only without one — gitui-org/gitui#771).
- **The command behind an action**: lazygit's command log shows git commands as they run
  (toggle `@`), but not before — a long-open request asks for exactly that
  (jesseduffield/lazygit#2572). The cockpit shows the argv in the confirm dialog (before) and
  in the activity details (during/after).
- **Menus**: lazygit's `?` menu lists actions with keys (arrow + Enter) but shows no disabled
  state (lazygit.dev/keybindings). Decision: inapplicable items hidden, blocked items greyed
  with the reason.
- **Long operations**: lazygit's push/fetch spinner locks the UI with no cancel
  (jesseduffield/lazygit#3324) — the anti-pattern; the cockpit keeps browsing free and
  offers Cancel.
- **Mouse**: on by default in zellij (Shift for native selection) and btop; opt-in in k9s;
  "not a goal" in gitui. Users of lazygit asked for a way to turn it off (#602) → on by
  default, `--no-mouse`. Selection under capture: Option-drag (Terminal.app, iTerm2),
  Shift-drag elsewhere; tmux best effort.
- **crossterm 0.29 `EnableMouseCapture`** writes `?1000h ?1002h ?1003h ?1015h ?1006h`
  (verified in `crossterm-0.29.0/src/event.rs`): 1003 reports every mouse movement, which
  would flood the loop and keep the confirm prompt's "no input pending" arming from ever
  holding. Decision: our own `Command` writing `?1000h ?1006h` (press/release/wheel, SGR),
  crossterm's `DisableMouseCapture` (resets all five) on every exit path. crossterm reports
  no double-click (the app times it); `Rect::contains(Position)` and `ListState::offset()`
  suffice for hit testing (a sketch `cargo check`ed against the locked versions); bare `Esc`
  is returned without a fixed delay.
- **Tree widget**: ratatui has none. `tui-tree-widget` 0.24.1 (crates.io API: MIT, released
  2026-08-09, 860k recent downloads; deps ratatui-core/ratatui-widgets/unicode-width — all
  already locked, zero new crates; source read: `click_at`, `rendered_at`, identifiers unique
  among siblings, no `unsafe`) is vetted and viable, but not taken: a flattened row list
  (~200 lines) is how the cockpit already renders its lists, and the widget's 24 breaking
  releases would pace our ratatui upgrades. Revisit if the hand-rolled tree passes ~400 lines
  or needs drag/multi-select.
- **Engine map**: no per-file state or file → unit index exists (`Snapshot` aggregates
  freshness only); units own files (`unit.files`); the scanner's file rule is private to
  harness-scan (`collect_source_files`) — to be shared through harness-core; findings live in
  `observer/findings.jsonl` and the cockpit never reads them; `scan`, `plan`, `detect`,
  `verify` are deterministic writers under the lock (`plan`/`detect` refuse stale facts);
  `observe`, `migrate`, `gen-driver` call a model; there is no plan-approval command.
- **Seen when opening the cockpit** (reconciliation step): the bottom hint line is cut
  mid-entry at 120 columns; a target without `harness.toml` is refused with a raw io error.

## 2026-09-24 — Cockpit wrapper: design, adversarial design review (46 → 13 → 1), design only

`docs/COCKPIT-WRAPPER-DESIGN.md` designs TUI-DESIGN §9 "Suggested order" 1: a file tree of the
target as the one navigator, the View showing the selection, `Enter` for a menu of what fits the
node's state, armed confirmations worded for a person, an always-visible activity panel that
narrates in plain language (the command one key away), the mouse in a second build. Not built —
the user asked for the design only this session.

**Review.** Four lenses (usability for non-vim users, safety & provenance, engine reuse, scope),
then an independent verifier per lens: 46 findings, 43 confirmed, 3 partly, 0 refuted (13 high).
A checker of the revision found 13 more (3 high: Modify let the target's `harness.toml` pick a
paid provider; ⚠ on states nothing could clear, stranding zopfli's u001; "record the crate as it
is" hit dead ends and relabelled unknown authorship as human). Both rounds' resolutions are in
the design (§R, §R2); a second, scoped check confirmed all 13 against the code (e.g. u001 passes
the "known code" test through the verdict's `rust_crate`, the same comparison `promote` makes)
and found one wording slip, fixed.

**What the review changed** (decisions):
- **Two builds, each reviewed**: Build A (keyboard-complete navigator, starting with the bugs
  below), Build B (mouse). One layout down to 80 columns; Activity visible at every width.
- **Confirmation**: arming is a latch — drawn whole, 300 ms since the last input READ, nothing
  pending (today's `poll(0)` kept). Keys before arming are dropped with visible feedback. Focus
  starts on the safe button; after arming a letter, or a move plus `Enter`, confirms — so a held
  `Enter` (auto-repeat arrives as Press) can never run an act. Quit and cancel are armed dialogs.
- **Provenance**: Re-check (`verify`) only on code the harness knows — a recorded attempt's
  candidate, or what the oracle last judged (the verdict's `rust_crate`) — else `⚠ changed outside
  the harness` (restore, or `harness override`). Verified code with no attributable attempt is
  `✓? origin not recorded`, not counted as migrated. MCP-DESIGN cut `verify` from chat for the
  same reason (an unlabelled edit).
- **Providers**: a cockpit `--provider` list (default `external`) as in harness-mcp; Modify
  passes it; Retry only for listed providers; never Retry an unseeded `external` attempt (a blind
  hand-off) or a half-seeded record.
- **Engine**: `Selection` replaces the rail cursor; `files::build(&Snapshot, &Walk)` pure, with
  two additive model fields (`stale_paths`, `crate_digest`); a language-neutral
  `walk::confined` in harness-core returning errors/skips/truncation as data (the scanner stays
  fail-fast, skips FIFOs, facts proven byte-identical); `status::live_holder` public (busy at
  project level only); harness-mcp's read preflight moves into the harness-tui library and the
  cockpit runs it before every load, on a worker thread; a stateful narrator matched to the real
  events; a non-blocking terminal guard.
- **Cut or deferred**: the idle watcher (with the chat pane), a findings view, find, right-click,
  duration promises, a guided "record an outside edit" flow; §10 reserves the chat's screen space
  and decides nothing else (the chat spike's questions).

**Bugs found in the SHIPPED cockpit** (Build A step 0, each with a failing test first): Retry of
an unseeded `external` attempt poses a blind hand-off whose hand-written answer would score as
pipeline output (SAFE-3); Modify and Retry let the target's config/record choose the provider
(CHK-1, SAFE-12); the cockpit's reads are unbounded — a facts path to `/dev/zero` hangs it
(SAFE-6); a signal right after the editor races `resume`, leaving a raw terminal and an
unannounced kept edit (SAFE-10); `Q` and the quit prompt act unarmed (SAFE-11).

**Found outside the cockpit, decided separately** (suggested as their own tasks): the crate
content hash covers only `Cargo.toml`, `Cargo.lock`, `src/**` while Cargo also builds a crate-root
`build.rs` (SAFE-5 — an integrity gap in provenance and verdict freshness); harness-detect's own
walk follows symlinks out of `source_dir` and has no cycle guard, while claiming to be the
scanner's (SCOPE-4). Also decided separately: `verify`'s R6 gate.

**Revisit when**: the hand-rolled tree passes ~400 lines or needs drag → `tui-tree-widget`
(vetted, zero new transitive crates); the chat pane lands → the idle watcher (off the UI thread,
every file the snapshot reads) and how chat's acts reach the activity panel.

## 2026-09-25 — Cockpit wrapper, Build A: built, reviewed, fix pass verified twice

docs/COCKPIT-WRAPPER-DESIGN.md Build A is built: the keyboard-complete navigator. Commits on
`main`: 50a3213 (step 0: the shipped cockpit's bugs), c39c9b6 (steps 1–4), c089575 (the code
review's fix pass), 2d6a662 (the fix pass verified, second fix pass), and this handoff (the
second pass checked, third fix pass).

**Baseline before the work** (quiet machine, zero tokens, 32 min): `bench check --replay
--jobs 6` → 198 reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems;
no regression. Tests 611 → 695.

**What was built** (the design's order):
- Step 0, each fix after a failing test: `Q` is `q` and the quit prompt is armed (SAFE-11); the
  terminal guard `harness_tui::termguard` (enables under a mutex, nothing once dying; the
  signal path restores, waits ≤ 200 ms, restores again; the panic hook never takes the mutex)
  and the kept edit named before `resume` (SAFE-10, CHK-5); `--provider` (default `external`),
  Modify passes the first, Retry refuses blind, half-seeded and unlisted (SAFE-3, SAFE-12,
  CHK-1, CHK-13); harness-mcp's read preflight moved to `harness_tui::preflight` (harness-mcp
  re-exports it) and the cockpit reads on a loader thread (`harness_tui::load`), the
  preflight first (SAFE-6, CHK-10).
- Step 1: `dialog` (the arming latch with an injected clock: drawn whole, 300 ms quiet from
  the last input AND the first full draw, nothing pending; focus on the safe button; a
  letter, or a move plus Enter, once armed); `menu` (items per node and state; greyed with
  reasons; model items under a separator; accelerators are the menu's shortcuts; one argv
  builder, `App::act_argv`, gains scan/plan/detect/verify).
- Step 2: `harness_core::walk::confined` (errors, skips, truncation as data); the scanner uses
  it — facts byte-identical on zopfli, read_scalefactors and a copy with inside links (a
  committed test compares with the committed facts), and a FIFO named `a.c` is now skipped and
  reported instead of hanging `harness scan`; `status::live_holder` public; the two model
  fields; `files` (the state rules, pure).
- Step 3: `tree` (Selection keyed by path/id, expansion, back stack); `app` rebuilt around the
  selection; `view` (Files, the View per selection, two activity rows with `narrate`, notices,
  a mode-aware hint bar, help with the legend, empty states, the hit record for Build B);
  goldens; `signals.rs` rewritten with a small VT interpreter (ratatui redraws only changed
  cells: a byte-stream search cannot see "ready"); a keyboard end-to-end (Enter → Scan → ready
  → → Enter → Done, facts rewritten) and a TERM right after the editor (restores last, cooked
  mode checked with `stty -a`, the edit named).
- Step 4: README's cockpit section, TUI-DESIGN §1/§3/§4, MCP-DESIGN §4.

**Review** (docs/COCKPIT-WRAPPER-DESIGN.md §R3, §R4): four lenses → 51 findings, 49 distinct,
two verifiers: 33 confirmed, 13 partly, 3 real-but-as-designed, 0 refuted. Fix pass; then
three checkers of the fix pass (two re-checked each resolution, one hunted regressions: 12
new, 1 medium found by all three — a dialog drawn once too small lost its words for good);
second fix pass; a scoped check of it (14 of 19 rows complete; 7 findings, 3 medium: the
crate's menu hashed a crate outside the preflight, so a FIFO froze the cockpit; Accept skipped
its check when the last read saw no crate; the terminal's drop was an unbounded write at exit);
third fix pass (§R5). Every fix has a regression test; the final run kills all 68 mutations
(the design's named rules and every fix of the three passes). Real end-to-end runs through a
pty (first run: Scan → Refresh the plan; a Re-check) found one wart (the CLI's own "run
`harness scan`" note repeated under the Next step), fixed.

**Decisions taken in the review, changing the reviewed design** (recorded in §R3/§R4):
- **Accept from a unit, crate or file opens the attempt first** (ENG-5; the design's §4.2 put
  a direct Accept there, but the unit's View shows the unit crate — TUI-DESIGN §R STATE-2,
  "Accept never promotes unseen code"). Accept also confirms on the crate as it is now:
  unknown code is never replaced; a hand edit of unknown code waits too.
- **No blank chat strip** from 150 columns (USE-15; §1/§10 reserved it, drawn empty): the View
  takes the width until the chat exists. Revisit with the chat pane.
- **Modify pins `--model=`** to the target's migrate model as last read: the model a dialog
  names is the one that runs (the provider stays the cockpit's list).
- Focus never wraps in a dialog; `Open` on every node; `E` works from anywhere; the confirm
  preflight only where confirm reads (Re-check, Accept, Retry, Resume); a Re-check with
  nothing shown opens no dialog; a new cause "its verdict is missing" (not "changed outside").
- The read model binds nothing, instead of failing, when a source or driver was deleted
  (NotFound only): the cockpit can show the file missing; harness-mcp now reports such a
  target instead of refusing it.

**Found in passing, not the cockpit's**: `ledger::tests::a_reader_never_makes_a_writer_fail`
fails when another test in the same binary forks (a fork holds a duplicate of the lock fd,
so `flock` stays held a moment); the new walk test was changed to use a socket instead of
`mkfifo`. Other tests in harness-core must not spawn processes, or that test should retry.

**Next** (unchanged order): Build B (the mouse: modes through the guard with a pty restore test
first; gestures; review) — the hit record exists and is tested. Then the chat pane (spike
first) with harness-mcp's requester label and a "migrate this" skill; the feature-workflow
view; C-vs-Rust performance baselines; the briefing's M5. Still separate tasks: the crate
content hash skipping files outside `src/` (SAFE-5 of the design review); harness-detect's
walk following symlinks out of `source_dir` (now easy: `walk::confined`); `verify`'s R6 gate.
Carry-forwards as before (§16 escalation + `harness usage`; `crash-timeout`; a Linux sandbox;
the two deferred replay items; driver-attempt Accept; queueing on lock contention; an async
client for cooperative cancellation; per-function verdict dots; bounded reads in harness-core;
harness-mcp's revisit triggers).

**Revisit when**: the chat pane lands → reserve its side then; a real target makes the
confirm-time preflight slow on the UI thread → move the confirm to the loader; the hand-rolled
tree passes ~400 lines → `tui-tree-widget` (vetted, zero new crates).

## 2026-09-27 — Cockpit wrapper, Build B (the mouse): built, reviewed, three fix passes

docs/COCKPIT-WRAPPER-DESIGN.md Build B is built: the mouse. Commits on `main`: 156b033
(step 1: the mouse modes through the terminal guard), 9e54cf2 (step 2: the gestures),
3fc3bec + ae4d904 (the review's fix pass), 2a05443 (§R6), 0500157 + 7fe0440 (the fix pass
checked, second fix pass, §R7), e7b9e78 + 554363a (the second pass checked, third fix
pass, §R8) and this handoff.

**Baseline**: 695 tests green before the work; the CLI and the scanner are untouched (no
bench check needed — only the workspace `Cargo.toml` gained a dev profile line for
crossterm). Tests 695 → 731.

**What was built** (the design's order):
- Step 1, starting with its pty restore test: `termguard::EnableMouse` (our own crossterm
  `Command`: `?1003l ?1002l ?1015l` then `?1000h ?1006h` — button tracking with SGR
  coordinates, never every movement); enabled under the guard (the loop's first pass, and
  `resume` after the editor; never once dying); crossterm's `DisableMouseCapture` on every
  way out (`restore_terminal`: quit, error, the panic hook, the signal path; `suspend`
  before the editor; the signal path also before it waits for the command). `--no-mouse`.
  pty tests: on from the start, off in every test editor (a mark the editors print), on again
  after `resume`, off LAST after a HUP, a TERM right after or during the editor, a USR1, a
  plain quit; `--no-mouse` never on.
- Step 2: gestures from the hit record through `App::on_event` (the loop's step: every
  input restarts a dialog's quiet time) and `App::on_mouse`. Click selects and focuses; ▸
  folds; a double click (rows, links; 400 ms, the first press timed with how long it may
  have waited in the queue — `QueueClock`) is `Enter`; the wheel scrolls the open menu,
  dialog or overlay, else the pane under the pointer (the tree and a list keep their
  selection); hint-bar keys and the activity row's buttons press their key (the activity
  row only in the panes); keys, buttons and menu items act on the release on the same spot,
  in the same screen and dialog; a drag of two cells says how to select text; Help's
  "Mouse on/off" (`m`).

**Review** (§R6): four lenses → 48 findings, two verifiers: 0 refuted (a few "partly" or
"as designed"). One high: the activity row's `[Cancel x]` was pressed as the key `x` under
an ARMED quit dialog — a click outside a dialog stopped the command and quit. A medium:
the second press of a slow double click on a menu item found the dialog it had opened
armed (300 ms quiet < 400 ms window < the OS double-click time) and pressed the button
drawn under it — "Find hazards" → Run; the verifiers found "Cancel the running command" →
`[Stop it x]` at every size. A real pty drive before the review found the triple click
(E2E-1). Fix pass; then three checkers of it (two re-checked the §R6 rows, one hunted
regressions): one medium found by all three — a press held while a KEY opened a dialog
was released onto it — and ~15 lows; second fix pass (§R7); a scoped check of it (no high or medium: one low-medium regression — "Stop it and quit" left the
mouse's reports of its 1.5 s wait to the shell — and lows); third fix pass (§R8), not
checked again (lows, each mutation-checked).
Mutation list: 75 mutations of the Build B rules ("the mouse is off on every exit path",
"a click never runs an unarmed dialog") and of every fix of the three passes — all killed.
The mutation runner lives in the session scratchpad only (a string replacement per mutant,
the guarding test, restore); it is not committed.

**Decisions taken in the review, changing §7** (recorded in §R6/§R7, which govern):
- A dialog button answers a press AND release on it, never within `CLICK_SETTLE` = 1 s of
  the dialog opening (allowing for queue delay), and — all but the safe one — only once a
  frame showed it armed. The keyboard's rule is unchanged (Build A's latch).
- A click on Quit is asked first even with nothing running (`Kind::QuitIdle`); the key `q`
  still quits at once when idle.
- Swallows: a press or release that opens or closes something swallows presses anywhere for
  the 400 ms window (from the gesture, never extended); a double click and ▸ swallow their
  own region. Repeated clicks on a key that opens nothing all count.
- A click on ▸/▾ folds (§7 had no fold gesture; functions were keyboard-only).
- Below 80 columns the details leave the activity rows and the hint bar (§1 said "the
  whole screen"), so a click can close them.
- Apple's Terminal has no key that lets a drag through (its docs name only ⌘R, "Allow
  Mouse Reporting"); the words say Shift (most terminals), Option (iTerm2), ⌘R in
  Terminal, or `m` in Help.
- `[profile.dev.package.crossterm] overflow-checks = false`: crossterm 0.29 computes a
  report's coordinate as `n - 1`; a zero coordinate panicked debug builds.

**Accepted / later**: hits are the last frame's (a reload that shifts rows within the
user's reaction time can put another row under the pointer — selection and menus only);
the overlays' border prompts, the notice row and the checks' rows are not clickable; keys on
an armed dialog need no armed frame (Build A); TSTP is not handled (raw mode makes Ctrl-Z a
key); `bounded`'s inline fallback can wait on the stdout lock when no thread can start AND
the main thread is stuck in a stalled write.

**Process notes**: the check of the fix pass again found a medium all checkers agreed on —
keep the scoped re-check after every pass. Mutation checks surfaced equivalent mutants
where a fix doubled a guard (e.g. the mouse turned off both in `suspend` and in its drain):
mutate both copies together. A test on a synthetic clock can pass for the wrong reason (a
key clears notices older than its `now`) — drive such checks with `Instant::now()`.

**Next** (unchanged order): the chat pane (spike first; it takes the right side from 150
columns — reserve it then) with harness-mcp's requester label and a "migrate this" skill;
the feature-workflow view; C-vs-Rust performance baselines; the briefing's M5 (external
detector plugin + `EXTENDING.md`). Still separate tasks: the crate content hash skipping
files outside `src/` (SAFE-5 of the design review); harness-detect's walk following symlinks
out of `source_dir` (one line: `walk::confined`); `verify`'s R6 gate; the harness-core
ledger test that a fork in the same test binary can fail. Carry-forwards as before (§16
escalation + `harness usage`; `crash-timeout`; a Linux sandbox; the two deferred replay
items; driver-attempt Accept; queueing on lock contention; an async client for cooperative
cancellation; per-function verdict dots; bounded reads in harness-core; harness-mcp's
revisit triggers).

**Revisit when**: the chat pane lands → clickable chat, and whether the notice row and the
border prompts should answer clicks; a user reports the 1 s settle as slow → measure the
OS double-click interval instead of a constant; a terminal without SGR reports shows up →
the X10 parser's coordinate limits (223) and `--no-mouse`.

## 2026-09-27 — Chat pane: §15 spike (an embedded agent runtime; chat in ratatui)

Time-boxed: three Sonnet subagents (Claude Code's headless mode from its docs, the Python
Agent SDK's source and the installed binary; other runtimes and ACP; chat rendering in
ratatui TUIs and the candidate crates), and the premise run end to end by the main session
(a Python driver over pipes, Claude Code 2.1.274 on haiku, harness-mcp from `target/debug`
attached to a zopfli copy, ~$0.30).

**Verified by running it** (the driver's logs are the evidence):
- `claude -p --input-format stream-json --output-format stream-json --verbose` stays up for
  many turns, one JSON line per user message, until stdin EOF. Each turn opens with
  `system/init` (the MCP servers and their status, the tools, the model, `apiKeySource`) and
  ends with `result` (`success` / `error_during_execution`, `terminal_reason`,
  `total_cost_usd` — cumulative for the process —, `permission_denials`). Lines seen up to
  ~24 KB. No API key: the binary's own subscription login (`apiKeySource: none` under the
  desktop host). `CLAUDECODE=1` (a cockpit started in a Claude Code terminal) changes nothing.
- **Permission prompts reach the parent before the tool runs.** Without a handler, every tool
  use not pre-allowed is denied. `--permission-prompt-tool stdio` sends `control_request`
  `can_use_tool` (tool name, full input, `tool_use_id`) on stdout; the parent writes
  `control_response` `allow` (+ `updatedInput`) or `deny` + message on stdin. **Held open for
  150 s, it was answered normally** (no timeout). A deny whose message said "the cockpit ran
  this after the user confirmed it; result: GREEN …" was read by the model as that outcome.
- `control_request` `interrupt` ends the turn (`aborted_streaming`, or `aborted_tools` with a
  `control_cancel_request` for a pending permission) and the process takes the next message.
  A user message written mid-turn is folded into that turn. SIGINT to the group: the pending
  permission cancelled, exit in ~1 s, harness-mcp gone with it. stdin EOF with a permission
  pending: the request fails, the model may retry (fails too), the turn ends, exit 0 — EOF
  alone lets a turn finish.
- `--session-id <uuid>` then `--resume <uuid>` in a new process continues the conversation;
  transcripts go to the runtime's `~/.claude/projects/<cwd slug>/` (outside the repo — §12.1:
  they may hold the target's code); `--no-session-persistence` writes none.
- Isolation: `--tools "Read,Grep,Glob" --restricted --setting-sources "" --strict-mcp-config
  --mcp-config <json>` leaves exactly those three (confined to the working dirs) plus the
  harness tools — no Bash/Edit/Write/Web, no deferred-tool search, no auto memory. The user's
  own skills stay listed (usable only through the `Skill` tool); `--disable-slash-commands`
  removes every skill. A harness-shipped skill loads with `--plugin-dir <dir>`
  (`ruharness:migrate-this`) and was invoked and followed on "Please migrate the katajainen
  unit."
- **With every tool call auto-allowed, the model "migrated" a verified unit by calling
  `harness_promote`, then again with `replace: true`, on its own initiative** (scratch copy).
  Every chat act must be the person's armed confirmation, never the model's call.

**From the sources** (docs agent; code.claude.com/docs/en/{headless,cli-reference,sessions,mcp,
env-vars,legal-and-compliance}.md, agent-sdk/typescript.md; github.com/anthropics/
claude-agent-sdk-python `types.py`, `_internal/query.py`):
- The `stdio` value is **SDK-internal**: the Python SDK passes it for a `can_use_tool`
  callback; the public docs name only the MCP-tool form. The protocol can change in any
  release.
- `--bare` (recommended for scripted runs) never reads OAuth or the keychain: it needs an
  API key, so it is out. SIGTERM kills the process with no result and answers no pending
  prompt; SIGINT ends only the turn. `result` subtypes also include `error_max_budget_usd`
  and `error_max_structured_output_retries`; failed `--mcp-config` entries are reported in
  `init.mcp_server_errors`. MCP `notifications/progress` do not reach the stream (only the
  runtime's own `tool_progress` heartbeat). MCP output above 25k tokens is saved to a file.
- **Policy** (legal-and-compliance.md, "Usage policy"): running the UNMODIFIED Claude Code
  binary, each user signed in through its own flow with their own subscription or key, is
  permitted, including inside other products (commercial terms apply to products);
  developers may not offer claude.ai login in their own apps, nor collect, store or
  intermediate credentials, nor pay for or resell usage for their users. The cockpit spawns
  the user's installed `claude` and never touches a credential.

**Other runtimes** (github.com/agentclientprotocol; openai.com/index/unlocking-the-codex-harness;
geminicli.com/docs/cli/acp-mode; cursor.com/docs/cli/acp): the Agent Client Protocol (v1;
a v2 schema in progress) has the same shape — prompt, streamed updates,
`session/request_permission`, cancel, MCP servers passed in `session/new`. Gemini CLI and
Cursor's CLI speak it natively; Claude Code only through `@agentclientprotocol/claude-agent-acp`
(Node ≥ 22, wraps the Agent SDK, renamed twice in a year); Codex through its own app-server
JSON-RPC (server-initiated approvals) or an adapter. The Rust `agent-client-protocol` 2.2
crate is async (futures).

**Rendering** (codex-rs/tui, the closest reference — Rust and ratatui): history cells, one
mutable while streaming; markdown hand-rendered from pulldown-cmark; finished history
written into the terminal's own scrollback; the approval as a list overlay whose Esc
cancels; the composer hand-rolled; bracketed paste plus a burst heuristic; the model's
control characters filtered before any escape of its own. Crates checked on crates.io and
RustSec (none with advisories): `ratatui-textarea` 0.9.2 (the ratatui org's fork; the
original `tui-textarea` is stalled before ratatui 0.30), `tui-markdown` 0.3.10
(pulldown-cmark, itertools, tracing; optional syntect), `pulldown-cmark` 0.13.4, `textwrap`
0.16.4, `tui-scrollview` 0.6.8.

**Chosen defaults** (for the design to specify and its review to attack):
1. **Claude Code headless, spoken directly** by a small sync module in harness-tui: the child
   in its own process group, stdin a pipe (user messages, control responses), stdout NDJSON
   read by a thread with a line bound, stderr drained — the pattern of `spawn`, zero new
   crates. The adapter turns the stream into a narrow internal event set (text, tool call,
   permission request, turn end, error), so an ACP adapter can be added later without the
   pane knowing.
2. **The cockpit is the only executor and the only approver.** The chat runs with
   `--tools "Read,Grep,Glob" --restricted --setting-sources "" --strict-mcp-config` and
   harness-mcp attached in a new `--cockpit` mode with no harness binary at all (corrected
   by the design review, SAFE-9/ENG-19: omitting `--harness` is NOT read-only — harness-mcp
   then finds `harness` on PATH or next to itself; the spike's runs passed `--harness`). Every tool call arrives as `can_use_tool`: the reads are
   allowed; a harness act is mapped onto the cockpit's OWN act (its argv builder and §4.3
   gates) and shown in the same armed dialog, as the chat's request; the permission stays
   open while the dialog and the command run, and is answered `deny` with the outcome — the
   cockpit ran it. The activity panel, one command at a time, Cancel, the signal path and the
   hand-edit rules apply unchanged.
3. **Zero new crates for the pane**: plain text (no markdown), wrapped by display width with
   the view's filter; an in-app transcript that follows new output unless scrolled up (the
   cockpit owns the alternate screen — native scrollback does not fit); a hand-rolled input
   box like the note input.

**Rejected**: ACP via claude-agent-acp (Node and an adapter with churn, an async crate, for
no capability Claude Code lacks); an Agent SDK (a Python or Node runtime inside a Rust
tool); the raw Messages API (no API key here, and an agent loop of our own — briefing §7);
`--bare` (no subscription login); the documented MCP-tool prompt form (the prompt would land
in harness-mcp, a grandchild with no channel to the cockpit); letting harness-mcp run the
acts (a second executor: no events in the panel, the one-at-a-time rule broken, a signal
path three processes deep, the argv shown not provably the argv run); `tui-markdown`,
`ratatui-textarea`, `tui-scrollview` (weight for what the cockpit already does).

**Open for the design** (found by the spike): who writes the response when the chat answers
an `external` hand-off (today harness-mcp writes only hand-offs it posed; the cockpit never
writes the ledger); the requester label — attempt ids are content-derived (unit, source,
driver, provider kind, model, request), so a chat-requested fresh attempt would share its id
and directory with a blind one of the same inputs unless the requester is part of the id;
the model named in a chat act's `--model` must be the model that answers (the chat's, from
`init`), not the target's migrate model; session persistence (off by default: transcripts
would leave the repo); where the skill lives (a `--plugin-dir` plugin, which also exposes the
user's own skills through `Skill`, or the same text as `--append-system-prompt-file`).

**Found in passing** (its own task): a `claude-code` provider for harness-llm — `claude -p`
with no tools, no settings, an empty working dir and the system prompt replaced — would give
blind translations through the user's subscription without an API key or a hand-off. Needs
its own spike: what still leaks into the context (CLAUDE.md, memory, skills), token counts,
and the policy above.

**Revisit when**: a Claude Code release changes the stdio control protocol (the adapter's
recorded-stream tests and a live smoke test catch it) → the documented MCP-tool form with a
channel to the cockpit; a runtime cannot hold a permission open → harness-mcp in a
request-only mode, the outcome sent later as a message; Claude Code speaks ACP natively, or a
second runtime is wanted → an ACP adapter; `--bare` accepts the subscription login → use it;
the transcript needs markdown → pulldown-cmark (vetted above).

## 2026-09-27 — Chat pane: the design (three reviews) and Build C (the requester label)

docs/CHAT-PANE-DESIGN.md is written, reviewed three times, and its Build C — the requester
label, the harness side of chat — is built, reviewed, and fixed in three passes, each checked
or mutation-checked. Commits: 3aa8270 (the design), e14ecca (revised after its review),
150fca2 (the second revision), fa2697e (Build C), 8e16be1 (its fix pass), 0ef2f73 + 100dcc4
(fix pass 2), ddfe3ba (fix pass 3) and this handoff. Build D (the pane itself) is next.

**The design** (§0–§11): the chat asks only for model work — Migrate, Modify, Retry and a new
Continue (an answer to a hand-off) — each shown in the cockpit's own armed dialog with its
argv, run by the cockpit (the only executor), its permission held and answered `deny` with the
outcome; Scan, Plan, Re-check and Accept stay the person's. Claude Code headless, spoken
directly (the §15 spike); harness-mcp attached in a `--cockpit` mode with no harness binary;
`Read`/`Grep`/`Glob` only; the brief by `--append-system-prompt`; three columns from 156,
the chat only while focused below that. Reviews: four lenses → 87 findings, 0 refuted (§R);
the check of the revision → 71 (§R2); a scoped check of the second revision → 18, no high
(§R3). The shared-traces hole — a chat hand-off and a blind one of the same inputs keyed by the
request alone — was found by every reviewer and by the main session: chat attempts get their
own `traces/chat/` and a label in the id.

**Build C, as built** (§4):
- `AttemptRecord.requester: Option<String>`, the closed set {"chat"}; written
  `schema_version` 2 only when labelled (unlabelled records byte-identical, v1); ids mix
  `\0requester:chat` in, so a chat attempt never shares a blind one's id; its hand-offs live
  in the unit's `traces/chat/`. `blind()` = unseeded + `external` + no label.
  `Authorship::Chat`, `Provenance::Chat` (buckets pipeline, steered, chat, human); the bench
  reports a chat-provenance crate as a PROBLEM and replays chat attempts, tagged `(chat)`.
- CLI: `migrate --requester=chat`; `--answer=FILE|- --answer-key=KEY` (stdin framed by
  `--answer-bytes=N`) files an answer as the response to one pending request — refused up
  front (`answer-refused`) unless the attempt the run resumes waits on it; `answer-unused` when the run never asked for it (after its own
  events). The `awaiting` event gains `request_key`; its args and resume drop the answer flags.
- harness-mcp: labelled steer and retry; `harness_answer` answers through the CLI (the text on
  its stdin — harness-mcp writes no file at all); a new `harness_request` read pages a
  labelled attempt's pending request.
- The cockpit: "asked in chat" beside steered; Retry keeps a record's label, and a chat
  `external` record's retry is the chat's.

**Bench**: `bench check --replay` on fa2697e: no regression, identical totals (hidden
16/17, public 70/77 strict-pass), 198 replays reproduce, 0 problems (29:52); again on fix pass
2 (100dcc4), which changed how every recorded response is read: identical (28:32). Fix pass 3
touched no replay path.

**Review of Build C** (§R4): three reviewers (robustness, engine & contracts, provenance &
safety) → 38 findings, many found twice or three times. Two high: the cockpit's Retry dropped
the label — a chat record retried as a blind base or an unlabelled steer (all three found it);
harness-mcp adopted an existing answer dir on a shared `/tmp` and the CLI followed symlinks.
Mediums: every up-front `--answer` refusal typed `answer-unused` (its test passed for the
wrong reason); `answer-unused` swallowing a finished run's events; `check_answer` validating
another attempt than the run resumes; the sweep deleting live dirs; predictable temp names
followed through committed symlinks (the same pattern was in `write_atomic` since M1); the
hand-off dropped before its answer was spent; quadratic paging. Fix pass 8e16be1.

**Check of the fix pass** (§R5): three checkers — every §R4 row fixed or partial, no high,
one low-medium: the key binding keyed on "a record with no turn", which a resume produces
until its first turn is re-judged, so an interrupted one refused its real repair. The safety
checker found the reason to drop answer files altogether: the oracle's build sandbox may
write all of the temp dirs, so a hostile build step could rewrite an answer file between
harness-mcp's write and the CLI's read. Fix pass 2 (0ef2f73): **answers travel on the CLI's
stdin** (`Running::spawn_with_input`, which the cockpit will use too) — the answer dir, the
sweep and six findings about them are gone; the key is bound by the run's own first request;
`ledger::read_regular` (non-blocking open, the handle checked, bounded) for every trace and
answer file read; a hard-link fallback; the rest of §R5. A scoped check of fix pass 2 (§R6):
no high, no medium; three lows — the hard-link fallback could leave a torn response, a stdin
answer cut short (harness-mcp killed mid-write) would be filed as a prefix, the cockpit read a
symlinked response the CLI refuses — and four nits. Fix pass 3: rename-into-place, stdin framed
by `--answer-bytes`, the cockpit through `read_regular`, `libc`'s open flags (already in the
graph via signal-hook, errno and getrandom; the Rust project's own crate, no advisories — no new
code compiled), a terminal on stdin refused; not checked again (lows, each mutation-checked).

**Mutation checks**: 24 mutations of Build C's named rules (the label on Retry, the answer's
target and binding, the model and kind checks, `blind`, the store's closed set, the
`traces/chat/` routing, the answer flags stripped, the hand-off lifecycle, paging, the sweep),
13 of fix pass 2's and 7 of fix pass 3's — 44, all killed. The runner lives in the session
scratchpad only.

**Tests**: 731 → 752, clippy clean.

**Decisions taken in the reviews** (the design's §R4/§R5 govern):
- An answer goes on the CLI's stdin, never in a file — harness-mcp's and the cockpit's —
  framed by `--answer-bytes`. `--answer=FILE` stays for a person.
- `--answer`'s key is bound to the attempt only while the attempt's first request is
  unanswered; after it, any pending request of the model passes the up-front check and the
  run files only the key it asks for (`answer-unused` otherwise).
- The ledger lock and promotion recovery come before `--answer`'s checks, as for every
  migrate; nothing else is written before a refusal, and no directory is created.
- The cockpit refuses to retry a chat-labelled `external` attempt: the chat answers its turns.

**Accepted / later**: `harness_request` re-reads up to 16 MiB per call (linear now); the
hard-link fallback is untested (no such filesystem here); the `(chat)` bench fixture, a chat
`.r2` replay from `traces/chat/` and `request_key` in observe/gen-driver events are untested;
the flaky harness-oracle process-group test under load (spawned as its own task).

**Process notes**: the check of the fix pass found a low-medium again, as in Builds A and B —
keep the scoped re-check after every pass. A reviewer's residual ("only an unsandboxed
process could race this") must be checked against the sandbox profile: the build profile's
temp-dir allowance made it reachable. Tests that fork (`mkfifo`) belong outside harness-core's
lib test binary, where a fork can hold the lock tests' lock.

**Next**: Build D, the pane (docs/CHAT-PANE-DESIGN.md §10): harness-mcp `--cockpit` and new
recordings with the exact argv; the `chat` module (runtime child, stream, end routine; a
shell-script fake `claude` replaying recordings; pty tests); `app` (focus, input, requests,
mapping, outcomes, the hand-off table, the continuation permission, the typing guard, the
chat-dialog rules); `view` (layout, tab strip, transcript, Help), the brief, README; the live
test by hand from a plain terminal and from inside Claude Code. Then review, fix, verify,
mutation checks, DECISIONS, push. The chat's stale-dir sweep inherits §R5's rule (own 0700
dirs; "gone" only from `kill -0` under `LC_ALL=C`). Still separate: the crate hash skipping
files outside `src/`; harness-detect's walk following symlinks; `verify`'s R6 gate; the
harness-core lock test a fork can fail; the build sandbox's temp-dir allowance (a per-build
temp dir, as the run profile has).

**Revisit when**: Claude Code changes its stdio control protocol (§15 spike); a filesystem
without hard links shows up (test the fallback there); `harness_request` is slow on a large
unit (cache pages per file identity).

## 2026-09-28 — Chat pane, Build D (the pane): built, live-tested, reviewed, four fix passes (three checked)

The chat pane is built: the cockpit runs Claude Code headless as a child, speaks its stream-JSON
protocol, and maps every tool call of the chat onto the cockpit's own acts. docs/CHAT-PANE-DESIGN.md
§10's five steps, each committed when green: c765883 (harness-mcp `--cockpit`; the fence and
`valid_model` into harness-tui's library), 7a38247 (recordings of Claude Code 2.1.274 with the
exact argv; the brief), a78ed2e (the `chat` module), 3a91742 + 77e843b + b7c0c7b (app, view,
main; their tests; the pty tests), 7c3e192 (the live test from both environments; README);
the fix pass c1e6362 (§R7), tests 8eff10f + d67b441 + 4e37f89 (the mutation run's survivors),
fix pass 2 3ef8b5c (§R8), fix pass 3 57eaa11 + 87d3563 (§R9), fix pass 4 45ff177 (§R10), and
this handoff.

**As built** (the design's §1–§9 hold; where the build decided):
- harness-mcp `--cockpit`: no harness binary found or run at all (`--harness`, `--provider`,
  `--target-root`, `--allow-unsandboxed` refused); the reads without a target parameter; the
  act tools ask-only (`harness_migrate`, `harness_steer`, `harness_retry`, `harness_answer`),
  answered by the cockpit's `deny` with the outcome; the banner says it runs no act.
- `chat::runtime`: binaries resolved absolute; §1.1's environment (inside a Claude Code
  session an allowlist plus the person's sign-in, the endpoint only with a credential; outside,
  all but the session's variables; `TERM=dumb`); its own process group; a 0700 directory
  (a `/tmp` fallback when the socket path would pass 103 bytes); the stale-dir sweep off the
  loop (own 0700 dirs, not links; "gone" only from `kill -0` under `LC_ALL=C`); a registry
  shared with the signal path and the panic hook; lines bounded at 2 MiB (a longer one is
  reported, never truncated); the end: stdin closed, TERM at 2 s, KILL at 3 s, reaped only
  after the group's KILL; the leader looked at every 2 s (a zombie ends the chat, its pipes read
  for 500 ms more).
- `chat` (stream, transcript, input): init checked on every init (permission mode, the one
  server, the tools); a result ends a Stop; Stop markers counted per Stop; echoes matched by
  the cockpit's uuids; a message the cockpit did not send withdraws the permissions; the quiet
  and start-up watchdogs; a transcript bounded at 2 MiB with a wrap cache.
- `app/asks.rs`: requests queue and settle a second before a key answers them; Review opens
  the cockpit's own armed dialog with the argv rebuilt fresh and the gates re-checked at
  confirm; the chat-dialog rules (no letters, bursts and Tab dropped); outcomes sent after the
  first read at or after the reap, queued, per generation; the chat hand-off table; Continue
  with the answer on the CLI's stdin (`--answer=- --answer-bytes --answer-key`), shown whole
  behind a gutter only the cockpit writes; the continuation permission (granted by an act's
  `awaiting` only under the same epoch, generation, model and held request; never under
  `--allow-unsandboxed`; ended by a Stop, a withdrawal, a foreign message, New chat, the end,
  a hold, a decline, a Cancel); the typing guard.
- `view/chat_pane.rs`: three columns from 156 columns (the chat ≤ 64, the View ≥ 80), below
  that the chat in the right column only while focused, a tab strip on every single pane;
  the bottom block by priority; Help's chat section says why the chat is unavailable.

**Live**: the whole round works with the real `claude` on haiku — Migrate asked in chat,
confirmed, `awaiting`, `harness_request`, `harness_answer`, Continue under the permission,
GREEN — from inside Claude Code and from a plain environment; the unknown-model,
failed-server and New-chat-then-TERM tests pass in both, again after fix passes 3 and 4 (GREEN
in both). After fix pass 2 the plain round ended RED (haiku's translation, 5 of 8 checks) with
the whole chat flow working — the test had waited for GREEN; it now takes either verdict.
Found by the live runs, not the review: the model once wrote its answer into the chat (the brief now says it goes only to
`harness_answer`); a message typed mid-turn is queued as the next turn, not folded
(`fold.jsonl`); the pty tests must read a rendered screen (ratatui skips cells already showing
the letter — the one unexplained quit failure).

**Review** (§R7): three lenses — safety & provenance, process & protocol, usability — 45
findings, two verifiers, none refuted. One high: the continuation permission granted by an
act's `awaiting` after a Stop, a withdrawal or a foreign message had ended permissions while
the act ran (the epoch). Mediums: an answer line cut at 4 KiB on screen while the CLI got
every byte; the details over the chat passing letters to the panes; a dialog's hint clicks
reaching the chat beneath; requests refused rather than queued while a command ran; the quiet
notice after every long act; a Stop outliving an aborted result. Fix pass c1e6362.

**Checks of the fix pass** (§R8): a row-by-row check with 64 mutations (every fix present,
every high and medium killed; ~20 rows without a killing test) and a regression hunt (no path
to a chat act without an armed dialog or a live permission; the answer could forge the `↩`
split mark; Cancel dead under the details; a late Stop marker taken for a foreign message; the
epoch voiding an unrelated grant, and not voided by the grant act's own Cancel). Fix pass 2
3ef8b5c answered each, with a test. Its scoped check (§R9): every fix present and killed, three
partial — the Stop-marker bookkeeping keyed on `queued_turn_count`, which 2.1.274 reports as 0
with a turn queued (so the fix was inert on the real runtime, its test built on a shape the
runtime never sends); a Cancel after the grant act's `awaiting` kept the permission; the cause
words from a field never cleared. Fix pass 3 57eaa11: a turn is queued while a message of the
cockpit's is not yet echoed; each permission records the request that gave it; causes per
attempt. Its scoped check (§R10): every row in place and killed; one low it made reachable
— a message cancelled between its start and its echo stayed "not yet echoed" all generation,
so the markers' idle reset stayed off and a marker-shaped message the cockpit did not send could
be swallowed later — plus test gaps. Fix pass 4 45ff177: a message also leaves on its terminal
lifecycle (the runtime's `command_uuid` is the cockpit's uuid); the markers matched exactly;
the gaps tested. Lows and tests, each mutation-checked; not checked again (as Build C's pass 3).

**Mutation checks**: 87 of Build D's named rules and the fix pass (after strengthening ten
tests the first run passed, and one invalid mutant fixed), 38 of fix pass 2, 14 of fix pass 3,
9 of fix pass 4 — 148, all killed. (Until fix pass 3's run the runner took a syntax error
for a failing test; it now counts "could not compile" without a test result as invalid — no
earlier log was one.)

**Tests**: 752 → 843 (+4 live, ignored unless `RUHARNESS_LIVE_CHAT=1`), clippy clean. No new
crate. The bench is untouched (neither the CLI nor the scanner changed).

**Decisions taken in the build and the reviews**:
- The answer a Continue files is shown whole in its dialog: every line hard-wrapped with its
  indentation, behind a numbered gutter only the cockpit writes — never cut, never re-flowed.
- The permission epoch is bumped by generation-wide causes and by the person's Cancel of a
  grant act; one attempt's hold or decline does not void another's grant.
- Requests queue while a command runs; Review and confirm keep one command at a time.
- A turn is queued while a message of the cockpit's is neither echoed nor at its terminal
  lifecycle — never from `queued_turn_count` alone (2.1.274 says 0 with a turn queued). A Stop
  sent as its turn ended stops the turn queued behind it; "stopped" is said only for a turn
  that ended aborted, brought the marker, or ended in error while stopping.
- Burst: keys within 5 ms, and the key after two reads in a row with input pending (100 ms).
- In the chat, a quit is always asked.

**Accepted / later**: untested by design — SAF-9's two `dying` checks in main.rs (a signal
inside a waiting Continue's quiet second), a timed-out `kill_now` kept (PRO-9), a KILL that
cannot be sent (PRO-10: `/bin/kill` cannot be made to fail here; no `libc::killpg`, the crate
forbids unsafe code); `ps` runs on the loop every 2 s while a chat lives (~1.6 ms); the
protocol verified with Claude Code 2.1.274 only (`TESTED_VERSION`; a newer one is said, not
refused); the §11 list stands.

**Process notes**: the check of every fix pass found something again (Build D: a forged split
mark and a dead Cancel the first fix pass made reachable) — keep it. The mutation run finds
tests that pass for the wrong reason (a test whose fixture already made the rule true, a check
of absence racing a writer thread, a guard doubled by the fix): run it before the check of the
fix pass, not after. Live runs with the real model find protocol facts the recordings miss —
and a fix of protocol handling must be tested on the recorded shape: fix pass 2's Stop rule
was inert on the real runtime, its test built on a `queued_turn_count` it never sends. A
subagent's report of a probe that flips against the parent's rule is the strongest evidence a
check gives: ask for it.

**Next**: the feature-workflow view (docs/NEXT-SESSION.md), then the C-vs-Rust performance
baselines, then the briefing's M5. Still separate: confine the oracle build sandbox's temp
dirs; deflake the oracle's process-group timeout test; the crate hash skipping files outside
`src/`; harness-detect's walk following symlinks; `verify`'s R6 gate; the harness-core ledger
test a fork can fail.

**Revisit when**: Claude Code changes its stream-JSON control protocol or its init shape
(re-record with `tests/fixtures/chat/record.py`; the recordings name the version); a peer
inbox for Claude Code sessions becomes discoverable (the foreign-message rule); another
runtime (ACP) is wanted.

## 2026-09-29 — Feature-workflow view: §15 spike (what "a feature" is here); DECIDED (user): scenarios + verify

Three time-boxed Sonnet subagents (the landscape, an inventory of this workspace, the premise
run end to end on `targets/zopfli` and `targets/tractor`), then the probe idea prototyped by
the main session on a copy of zopfli.

**The premise failed as stated.** "A feature = an entry point plus the call paths under it" does
not separate behaviours on the real targets:
- zopfli: after two graph repairs (23 calls to `static inline` functions in `symbols.h` are
  unresolved — the scanner resolves statics only within the calling file; 4 functions reached
  only as function-pointer values have no edge), `main` reaches 110 of 111 functions and all 11
  units. The five README API functions are not roots (the CLI calls them). gzip, zlib and
  deflate differ by 2 functions of ~100 (Jaccard 0.96–0.98). CLI options set values inside
  `main`; the dispatch happens deep in `ZopfliCompress`. Static reachability cannot split them
  without dataflow.
- TRACTOR: 99 of 100 cases are one unit, 95 have one root; the only descriptive scenario names
  sit in `heldout/`, which never enters a target or a prompt. Not a useful target for this view.
- **The oracle gap the spike exposed:** zopfli's whole-program check runs `zopfli -c <sample>`
  only — the gzip path. No program-level check ever runs the zlib or deflate code, so a unit on
  those paths is judged at program level by a run that never calls it.

**What does separate them:** running each behaviour. Per-scenario coverage (clang source-based
coverage, `xcrun llvm-profdata`/`llvm-cov`) gives `--zlib` = gzip − {ZopfliGzipCompress, CRC} +
{ZopfliZlibCompress, adler32}, `--deflate` = gzip − {…}, `-v` + PrintBlockSplitPoints, `--i1`
− the four RandomizeStatFreqs functions. The main session then prototyped the harness-owned
alternative: a copy of the sources with `__probe(N);` inserted right after each scanned
function's opening brace (same line — `__LINE__` unchanged), a harness-owned C runtime that
writes each id once to a file in the run's temp dir, `-include` for its declaration, only `cc`.
Identical results to llvm-cov, exact facts ids (statics, header inlines as
`src/zopfli/symbols.h::…`), the program's output byte-identical to the uninstrumented build.
Chosen: the probe — no new tools on the allowlist (llvm-cov is not on PATH on macOS; `xcrun` is
a general launcher), gcc and clang alike, ids that are the scanner's own, data written at first
hit (a crash keeps it), and the oracle already has a confined run that reads back one file
from its temp dir (design B's `Extras::collect`).

**Landscape** (sources in the spike report; checked, not recalled): feature location splits
into static, dynamic (software reconnaissance, Wilde & Scully 1995; scenario traces with
concept analysis, Eisenbarth et al. 2003), textual and hybrid; dynamic results depend on
scenario design and are confounded by input (a fixed input per scenario); "omnipresent" code
is usually filtered by fan-in, which for a migration hides exactly the shared code a swap can
break — annotate it (touched by k of N features), never drop it. C→Rust work (TRACTOR's vectors:
argv/stdin/env → stdout/stderr/exit; CRUST-Bench; Syzygy; RustPrint's LLM rubric) has no
code-mapped feature: behaviour is always a test vector. Names come from usage text, README,
test names — or a person; an LLM may later propose, never override, a person's label.

**Decided (user, 2026-09-29), of three scopes offered — scenarios + verify / scenarios, view
only / static only:** a person names features in a ledger file, each defined by scenarios
(a run of the program: flags plus an input); the harness observes each scenario's footprint
on a probed copy of the original C (functions → files → units); each scenario also becomes a
whole-program C-vs-mixed check in `verify`, so a unit's verdict says which of the person's
behaviours still match; the cockpit shows a Features group and, on each unit, the features
that run through it. The design is docs/FEATURES-DESIGN.md.

**Rejected:** static reachability as the map (above); program slicing (needs dataflow the
facts lack); textual/IR location (terse C names, fuzzy); an LLM rubric as the map
(non-deterministic, a proxy); pure reconnaissance (keeps only feature-unique code); llvm-cov
(new allowlisted tools, clang-only, per-TU names needing a mapping rule); `-finstrument-
functions` (addresses: statics of the same name in two files are ambiguous without a link map).

**Found in passing, separate tasks:** the scanner leaves calls to a header's `static inline`
functions unresolved when the caller is in another file, and records no edge for a function
passed as a value (known gap, SCHEMAS.md); `043_iso646_and_digraphs_lib` scans to zero symbols
(digraphs).

**Revisit when:** a target is a library with no `main` (features then need a scenario program
of their own); a behaviour's output is a file, not a stream; stdin input is needed; two
features share their scenarios' footprints entirely (then only line-level evidence separates
them).

## 2026-09-29 — Feature-workflow view: built, reviewed, four fix passes (paused before the last check)

**What exists.** A person's features in `migration/features/features.toml` — features, each one
or more scenarios (fixed arguments, at most one of three in-memory samples as input). Every
Re-check and every judged migration turn runs each scenario on the all-C program and on the
program with the unit's Rust swapped in (C, mixed, C): a check `feature:<f>/<s>` passes iff the
exit status and both streams match byte for byte (streams rewritten one-to-one: `$$`,
`$TMPDIR`, `$PROGDIR`); a C side that is not usable is a recorded skip with a closed reason,
never evidence. `harness features map` builds a probed scratch copy (a one-byte-guarded note
after each function's `{`, a harness-owned runtime) and records which functions each scenario
runs in the committed `map.json`. The cockpit has a Features group (eleven states, per-unit
results, the Edit flow through a private draft and a confirmed `features save` with the text on
stdin), markers on verdicts and lines on units and functions; harness-mcp reports a closed
`features` coverage field. Nothing about features blocks work: an invalid file, unusable
scenarios or a unit outside the program each record a marker and a skip. zopfli is the
dogfood: 7 features, 8 scenarios, mapped in ~9 s (gzip/zlib/deflate/`-v`/`--i1` separate as the
spike found); u001 verifies green with 8 feature checks.

**How it was checked.** Design: four revisions (190 findings, §R–§R4). Build: eight steps, each
green. Code review from four lenses (trust, oracle, probe and map, cockpit and docs): 35
findings, each verified by an independent agent — 33 confirmed (two high: a kept features
draft deleted on an unchanged editor return; `source_dir = "."` read as "outside the program"
everywhere), 2 refuted as design-accepted. Four fix passes, each checked by independent agents;
each check found real issues in the pass before it — the pattern worth keeping:
- pass 1's include check could never fire (the scanner never records an include that leaves
  `source_dir`); the C-binary guard noted the bytes after the whole-program check's candidate
  runs (an in-crate test then showed the false green a self-copying candidate had without the
  sandbox, whole-program check included — pre-existing);
- pass 2's probe rule unwatched 31 ordinary functions in a 294-file corpus (sqlite,
  oniguruma, tree-sitter), and its hand-read `#include` lines refused includes under `#if 0`;
- pass 3's dependency comparison (`cc -MM`, the program's build against its copy's) turned off
  silently under a folder with a space (make's `\ ` escape).
87 mutants of the §12 named rules and every fix: 86 killed, 1 equivalent. 952 tests.

**Decided along the way (the design text holds each):** byte counts in details as a person
reads them (the dogfood found 206 for zopfli's 205 — the `$$` escape); `SOURCE_DATE_EPOCH=0`
for every tool child (two compiles of the same C print the same `__DATE__` — this also removes
a flaky false failure in the whole-program check); the program digest covers the facts' closure
plus every header on the include path, or is `facts-stale` exactly where a scan would change
the facts; a scenario's cwd is `run/` inside its temp dir; human CLI lines show control
characters as `?`.

**Open when paused (docs/FEATURES-PROGRESS.md "Open"):** two low items the check of pass 3
found (a top-level `.c` linked out of `source_dir`; a FIFO named `x.c`), a check of pass 4,
`bench check --replay` (the CLI changed), the live chat tests (the `claude` sign-in had
expired; every failure was that, said in words by the chat).

**Next**: finish the open items, then the C-vs-Rust performance baselines, then the briefing's
M5. Still separate: the ledger test that fails under load
(`a_reader_never_makes_a_writer_fail`); a whole-program check without the sandbox is only as
strong as the C binary guard (C reads elsewhere are not covered — SCHEMAS.md).

**Revisit when:** a target is a library (features then need a scenario program); a behaviour's
output is a file, not a stream; stdin input is wanted; the scanner learns macro-made
definitions (the map could watch them).

## 2026-09-30 — C-vs-Rust performance baselines: §15 spike

Three time-boxed Sonnet subagents (measurement on this machine; the migration and CI
landscape; where timing hooks into the harness), then the premise run by the main session on
`targets/zopfli` (u001-katajainen, the all-C and mixed programs `verify` builds). Notes:
scratchpad `spike-perf/{measurement,landscape,codebase,premise}.md`.

**The premise holds, with two corrections.** On a 200 KB input, 1 warm-up and 5 interleaved
runs each, under load from other agents: the all-C program 3.94e9 instructions (range 0.022%),
1.007e9 cycles (1.7%), 0.26 s wall (3.8%); the mixed one **+3.24% instructions, +0.79% cycles,
+3.8% wall**, the same 12.39 MB peak footprint. Corrections: (1) **the oracle's samples are
useless for timing** — `sample_text.txt` compresses 30 KB to 205 bytes in ~1 ms, where fixed
costs dominate (+85% instructions, cycles range 46%); workloads must be the person's and big
enough (≥ ~50 ms or ~1e8 instructions a run); (2) **instructions and cycles disagree** (the
Rust katajainen executes 3% more instructions at a better IPC) — instructions are the stable
signal, cycles the time-like one; report both, never judge on instructions alone.

**Measurement (verified on this M3, macOS 26.5.2, no root).** `proc_pid_rusage(pid,
RUSAGE_INFO_V4)` works for a child, under `sandbox-exec` (it execs in place, same pid), but
only before the child is reaped: `waitid(P_PID, WEXITED|WNOWAIT)`, then the rusage, then
`wait4` (user/sys time, maxrss). `libc` (already a dependency) exposes all of it; every call is
`unsafe`, so it needs an isolated FFI module — the second `unsafe` exception, to be justified in
the design. Traps: `ri_user_time` is in mach ticks, not ns; `ri_phys_footprint` is 0 after exit
(use `ri_lifetime_max_phys_footprint`, byte-identical across 30 runs); `ru_maxrss` is bytes on
macOS and KiB on Linux. Stability over 30 runs of a 0.3 s loop: instructions CV 0.004%,
cycles 0.05%, wall 0.53% (a 6 ms run: 0.076%, 1.1%, 4.1%); A vs A+0.5% work was detected at
n=6. `/usr/bin/time -l` prints the same counters, but the run sandbox forbids exec of anything
but the program. Linux: `perf_event_open` user-space instructions at `perf_event_paranoid` 2;
GitHub-hosted runners expose no PMU; valgrind `Ir` is the deterministic fallback.

**Landscape** (checked, with sources in the notes): TRACTOR's benchmark ranks correctness >
safety > performance > idiomaticity and measures runtime with `perf stat` and memory with
massif, randomized order with warm-up; most LLM translators measure no runtime; Syzygy's Zopfli
(our dogfood) ran up to 3.67× slower optimized (`Vec` allocation, bounds checks in
`ZopfliUpdateHash`), and 9–14× under default (debug) settings; c2rust-derived rav1d is within
6% after work (dynamic dispatch, locks, zeroing, bounds checks). Regression practice:
rustc-perf and the LLVM tracker judge instructions against per-benchmark noise; benchstat
reports medians with a 95% interval and prints `~` when not significant; interleaving A/B runs
cuts variance (layout and environment bias). Cross-language LTO needs matching LLVM majors and
does not work with Apple clang — out of scope.

**Chosen defaults (for the design):** a `harness perf` step of its own, never inside `verify`
(verdicts are content-bound and replay-compared; `migrate` and `bench check` run `verify`); the
two programs `verify` already builds (`whole_c`, `whole_mixed`) on workloads the person names
(args + an input file inside the target), plus the unit's differential driver (`drv_c` vs
`drv_rs`) as a per-unit row flagged when too short; instructions, cycles, wall, CPU time and
peak footprint per run; 1 warm-up + n interleaved runs a side (default 5); medians, ranges,
the ratio, and words from a non-inferiority rule ("slower by X%" only when the ranges are
apart and the gap exceeds the noise floor; else "within noise" or "inconclusive — add runs");
statistics in `std`; results with a machine and toolchain fingerprint; the cockpit shows a line
per unit. Tier 0 throughout.

**Rejected:** wall time alone (50–100× noisier); kperf/kpc (private, root); `xctrace` (a 16 MB
sampled trace for a 10 ms run); valgrind on macOS arm64 (an experimental fork); hyperfine,
criterion, divan as dependencies (an external binary without a decision rule; in-process
harnesses); Gungraun (in-process Rust benches, Linux-only); a history server or dashboard;
gating migrations or CI on perf; comparing across machines; cross-language LTO.

**Revisit when:** TRACTOR's round reports publish metrics and thresholds; Apple ships a public
per-thread counter API or `libc` gains `rusage_info_v6` (P/E-core split); instruction and cycle
deltas disagree in sign on real units (then cycles become the headline); a CI target has no
PMU (add a valgrind lane); multithreaded targets arrive.

## 2026-09-30 — Features map: PROPOSAL for review — let the compiler decide which functions the probe can watch

**Why now.** Seven fix passes of the features track, each checked by independent agents, and
every check found new ways the probe's syntax rules go wrong: pass 4's check 4 problems, pass
5's 2 medium, pass 6's 3 medium, pass 7's (b31f6ef, 45 confirmed by two verifiers each —
scratchpad `check7/findings.md`) one silent wrong map, three regressions of pass 7's own
split exception, and a precision cost the rules cannot avoid (a function whose head starts
with a macro word is never watched: about half of sqlite, much of Cython). The rules guess, from
a tree-sitter parse of unpreprocessed C, whether a statement at a body's start compiles; macros
make that undecidable, so each rule trades one wrong guess for another. Briefing §17: when an
implementation keeps hitting what the spec cannot answer, stop and hold a design session.

**Proposal (for the person's review).** The compiler is the oracle, as everywhere else in the
harness:
1. Watch every function the facts record, except where a note cannot even be placed (no real
   `{`, a directive between the declarator and the body, a definition under a parse error, a
   `naked` head) — the "hard" rules stay; the guessing rules (pragma macros, bare names,
   splits, lone macros, leading edges) go.
2. Before the probed build, compile each probed file with `-fsyntax-only` and the copy's own
   flags. If it fails where the original compiles: read each error's `file:line`, unwatch the
   watched function whose body holds that line (its note is on the body's first line; a
   misplaced pragma's error sits a line or two below), record the compiler's first message as
   the reason, re-probe that file and try again — bounded (at most the file's watched functions,
   and 8 rounds); an error that no watched function holds puts the file back unprobed, every
   function in it unwatched with that reason. A probe miss then costs one function, named
   with the compiler's words — never the whole map.
3. Refuse, by name, a probed file the copy reads as data rather than as code (listed by `-M`,
   never entered by `-H`: `#embed`, `.incbin`) — the one silent wrong map the check found.
4. Costs: one `-fsyntax-only` compile per probed file (sqlite3.c ~1 s), more only for files
   that fail. The probe unit tests of the guessing rules become map-level tests of the retry.

**DECIDED (user, 2026-09-30): redesign.** Build it as every track: premise run, design, an
adversarial design review from 3–4 lenses with findings verified, revision and its check,
steps, code review, fix passes each checked, mutation checks. The rest of the check's findings (wording, flaky-test causes, the
runner's poll ramp, doc gaps) are independent of this choice.

## 2026-09-30 — Features map, compiler-guided probe: the design, reviewed and checked twice; built from revision 3

**Premise** (scratchpad `proto/`): a prototype with the guessing rules off and a compile-and-retry
driver built every adversarial repro file (24/24, 25/25; today's rules 17/24, 18/25) in ≤ 2
rounds, and on real code cut unwatched functions from 3 044 to 6 (corpus) and 1 162 to 129
(extensions) — sqlite3.c ~1 500 → 0 — with no note rejected. A shared-memory runtime checked
under the sandbox (scratchpad `spike-mmap/`).

**Review**: the draft (777cd27) — 4 angles, 37 findings, each reproduced by two verifiers
(a skipped-branch note read "not run", a note in a stringized macro argument, backend and link
errors missed by syntax-only checks, the runner's 8 KiB error excerpt, `#line` and same-line
placement, gcc). Revision 1 (a14c807) checked: 4 fully resolved, 33 in part, 20 new. Revision 2
(f345a49, the same-program text comparison) checked: 17 of 57 fully, 40 in part, 14 new (two
high: a pre-zeroed notes file reading "nothing ran", and the comparison refusing every program).
Every finding, all rounds, confirmed by two independent verifiers; none refuted.

**Decided (process):** revision 3 (docs/FEATURES-PROBE-REDESIGN.md) takes every concrete change
those checks asked for and is built without a further design round. The mechanisms settled in
three rounds; what the last check found were details of the mechanisms (line-marker forms,
linker message forms, byte limits) that the build's tests, its code review and the checks of its
fix passes find better than another reading — each design round also added mechanisms that drew
new edge cases. The build still follows the process: steps each committed green, an adversarial
code review verified against the code, every fix pass checked, mutation checks of the named
rules, the premise re-run through `harness features map` itself (§8).

**Revisit when:** the build's premise re-run (§8) disagrees with §2's table — stop and hold a
design session before building further.

## 2026-09-30 — Compiler-guided probe: built (steps a–f), paused for its code review

Built from docs/FEATURES-PROBE-REDESIGN.md rev 3: 9a8ed37, f18e161, 2955dfc, b943926 (978 tests).
Checked while building: every adversarial repro and matrix shape of the last three checks maps;
200 rejected notes (over 8 KiB of errors) read in ≤ 2 rounds; byte placement on shared lines,
`#line`, errors through include chains; `#embed`/`__has_embed`/`__has_include`/stringized notes;
the runtime's merge across images and its attach byte; zopfli re-mapped with the same functions
per scenario and none unwatched. Found while building: the one-byte note shows no inlining window
on clang 21 (the C99-inline link case could not be provoked); the map loader would have refused
the new "setup did not run" reason (fixed, tested). Open: the §8 premise re-run and the code
review (both running at the pause), fix passes, mutation checks — docs/FEATURES-PROGRESS.md.

## 2026-10-01 — Compiler-guided probe: code review closed after seven fix passes; performance design closed

**The review.** The probe's code review was re-run at 665495b: five reviewers looking from
different angles (the runner and sandbox, what the person sees, the copy check, and two hunting
for untested rules), 72 findings, each reproduced by two independent verifiers. Seven fix passes
followed. Each was checked by a fresh review of its own changes, every finding reproduced twice
before it counted, and each pass had a mutation check (every new rule switched off in turn; a
test had to fail). Findings per check: 26, 26, 21, 13, 4, 4.

**What was wrong, in plain words, and is fixed** (design §10–§10.6 has the detail):
- The map could say a function ran when it did not, or "not run" when it did, where two
  functions are written so the parser reads them as one (a macro that supplies a whole body,
  written straight before another definition), where a function body's `#if` branches each open
  a brace, and where a hidden `#if` variant was "explained" by a same-named function that the
  compiler actually built under another name.
- The link step could refuse a program that can be mapped, search past its compile bounds, or
  unprobe files that never mattered.
- The copy check's tokenizer missed rare spellings (Unicode spaces, `#line` after comments,
  unclosed quotes); the object reader missed rare ELF layouts.
- Scratch folders could be left behind on a signal; older map files were refused instead of
  read; Linux-only test paths were untested.

**Checked on real code after every pass:** the 101 benchmark targets' facts byte-identical,
sqlite3.c unchanged (4 621 functions, 457 unwatched inside its two misread bodies), the premise
re-run identical (79 of 79). At the close: 194 789 real definitions read identically before and
after the last two passes; the full test suite 1 077 tests, all passing (two chat-pane timing
tests failed once under load and passed alone); `bench check --replay`: 198
reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems — no regression.

**Decided (process): the review stops after fix pass 7.** The last two checks found only rarer
spellings of one situation — a body-supplying macro written straight before another definition,
read without a preprocessor — with no instance in about 195 000 real definitions. Chasing
spellings has no end; design §6 names the class and what it costs (the first function can read
as run when the second ran, or the second is lost from the facts and an `#if` twin of it can read
"not run"), to revisit with a preprocessing frontend (libclang). Fix pass 7 got
a check that its fixes hold and regress nothing, not another open search. That check found
one of its rules (ranking an empty-parentheses head by where its line starts) broke shapes read
right before, so it was reverted and its target shape named in §6 too.

**Decided (process): the performance design is closed** after revision 5's check found no
mechanism that fails (no high finding); its 30 build notes (docs/PERF-DESIGN.md §10) answer the
rest and become the build's tests. It is built next in §5's order with the full process.

**Revisit when:** a real program shows the two-heads class (then: a preprocessing frontend), or
the premise re-run disagrees with §2's table.

## 2026-10-03 — C-vs-Rust performance baselines: built, reviewed, fixed; merged to main

**Built** from docs/PERF-DESIGN.md revision 5 in seven steps, each committed green (b, c, a, d,
e, f1–f4, g): the statistics and words (harness-core `perf`), the launcher and trampoline
(perfrun, perfgo; a private per-user cache; the perf sandbox profile), the measurement
(harness-oracle `perf`), `harness perf init|save|run|show`, the cockpit's Speed group, View and
acts, the behaves-differently fact and Compare the outputs, harness-mcp's Speed fact, and the docs
(SCHEMAS, the tutorial's Speed part, the testing guide's Part 11). Real result on zopfli:
u001-katajainen about as fast as the C (within 2 %).

**Reviewed** at d8d6d11 by six reviewers (statistics and words; files and currency; launcher
and sandbox; the measurement; CLI, cockpit and MCP; tests against the design): 84 findings, 82
confirmed by two verifiers each, 2 split (one wording no person can reach), 0 refuted. **Fix
pass 1** handled all 82, in six worktrees, one area each, every fix with a test; merged with one
docs conflict. **The scoped fix check** re-verified the 9 high findings on the merged code, two
skeptical verifiers each: all hold, each guarded by a test that fails when the fix is undone, no
regression. **Fix pass 2** so far: verify's own sandbox profiles closed (below). Then: every test
of the five perf crates green (about 950), clippy clean, `bench check --replay` 198 reproduce, 2
expected divergences, 0 problems, no regression.

**What was wrong, in plain words, and is fixed:** the launcher kept a core busy while the
program ran (it skewed the load it then reported as "busy"); a run under only one leg of the
floor was called short, so common workloads could never read "about as fast"; a missing input
after a too-short row stopped the whole run with an error; `perf show` ran the project's
compilers outside the sandbox; the hand-edit advice told the person to measure again when perf
would have measured the unchanged crate; the perf sandbox let a program have macOS start another
program for it (LaunchServices, Apple events, launchd jobs), which would outlive the run;
several rounding and wording edge cases (an interval printed backwards, a close call inside
±2 %, a program kept off the fast cores told "too few runs"); and many tests the design lists did
not exist.

**Decided: the same system-start rule now ends verify's own profiles** (tool, run, scenario) —
the perf review found they shared the gap; a pre-existing hole in the part of the harness
already in use, closed first in fix pass 2 (goldens updated, a live test under the run profile).

**Decided: perf runs on macOS only for now.** `perf run` refuses by name elsewhere ("perf runs
on macOS only for now — the Linux launcher is not built yet"). Reason: no Linux machine to build
or test the launcher's Linux half on (perf_event_open, epoll, PR_SET_PDEATHSIG), and CI's Linux
job has been red since 2026-09-17. The design's Linux mechanisms (§3.3) are written and marked
unchecked; building them is its own item when a Linux machine is available.

**Decided: stored rows read out of date after this close** — `PERF_RECIPE` is `perf-recipe-2`
and `PERF_LAUNCHER` `perf-launcher-2` (design note 31) — because rows measured by the spinning
launcher, or judged short by either leg, must not read current. Measuring again brings a row
back.

**Deferred to the week of 2026-10-07** (docs/NEXT-WEEK-PLAN.md): the full re-check of the other
73 findings, fix pass 2's remaining small items, the mutation checks, and the liblzg run of the
testing guide's Part 11 (not run here: liblzg is not on this machine). The person chose to stop
at 75 % of the weekly limit and merge once the security fix and the high findings were checked.

**Recorded, not decided:** the project map — real project layouts, several programs, a
deterministic walk with link closures, a model binning only the ambiguous part, the harness
checking its answer, and an interactive picture of the map (docs/PROJECT-MAP-ROADMAP.md,
-INVESTIGATION.md, -DESIGN.md draft 0). To design next, weighed against M5.

## 2026-10-07 — Project map: the spike on real downloads, and the person's answers to its open questions

**Spike** (docs/PROJECT-MAP-INVESTIGATION.md "The spike on real downloads"): the link-closure script
rewritten and run on lz4 (48 files, 33 entry points) and liblzg (8 files, 3 tools), both downloaded
with the person's OK into `~/code/ruharness-test-downloads/` (test data only, nothing installed).
The closures were exact and cheap (lz4 in 2.2 s; liblzg's `lzg` closure is the testing guide's
hand-picked file list). What the design underrated: **the build's flags decide the program** (lz4's
tool gains a file and pthreads with `-DLZ4IO_MULTITHREAD`), the project's own build systems
disagree (Makefile and Meson thread, CMake does not), and **a flag cannot be checked by linking**
(both configurations link). Also: a `main` that is a driver for many fuzzers, and duplicates
between programs that never meet in one closure (harmless).

**Decided (the person, 2026-10-07)** — recorded in docs/PROJECT-MAP-DESIGN.md §7:
- without a `compile_commands.json`, **the person states the build and its flags**; a model may
  suggest them from the build files, the person confirms — the baseline is one named configuration;
- `harness map` **always stops and shows**; the person accepts which tools become targets;
- a mapped tool's target and ledger live **inside the project** (`<project>/migration/…`);
- a **library with no tool stays a target** on its own (the benchmark's shape);
- the per-file compiles run by default (2.2 s for lz4: decided on the evidence).
Still open (§7 5–7): the picture, model advice, an improvements mode — last, as agreed 2026-10-03.

**Revisit when:** a project's flags differ per file in a way one stated configuration cannot
express (then: several configurations from `compile_commands.json`).

## 2026-10-07 — Speed work, fix pass 2: the twelve leftovers fixed; the liblzg walkthrough run

**Fix pass 2** (docs/NEXT-WEEK-PLAN.md §2, items 2–13; item 1 was done 2026-10-03): five agents, one
area each, in their own worktrees (workflow `wf_c913c6d1-af7`), every fix with a test each agent
showed fails when the fix is undone; merged as five merge commits (7abd11d … c92f6ec), full
workspace 1 251 tests green, fmt and clippy clean. In plain words: `perf show` without facts no
longer says "the C changed" on every row (it says once that the C is not checked); with nothing
stored it runs no compiler or computer check; `--as-it-stands-only` refuses before building
anything when fewer than two units are measurable; the results reader caps every list a row holds
(a forged file's long list is refused by name); `perf run` never reads through or deletes a linked
`migration/perf/units` folder (refused by name, before anything is built); perf run and perf show
read the compilers with one function; tests now pin the stored short flag, the second-run case and
the two-unit run's row counts; the launcher tests can no longer pick another worktree's processes
when two test runs share the machine; SCHEMAS and PERF-DESIGN wording fixed.

**Decided: harness-tui may depend on harness-oracle**, optionally and only for the `tui` feature
(the terminal front end), to ask `perf_launcher_cached()` — whether perf's launcher is built — so
the Measure dialogs' estimate counts the 25 s launcher build when it is not. No outside crate is
added (the cockpit's tree gains harness-oracle and harness-scan, both workspace crates); the read
model harness-mcp reuses stays harness-core only. The question is asked on its own thread, waited
for at most 0.25 s. Rejected: a new `harness` subcommand for it (CLI surface is a stable contract).

**The liblzg walkthrough** (plan step 4): Parts 0–10 of docs/TESTING-GUIDE.md run end to end on a
real download, both translations through the cockpit's chat on the person's subscription (Opus,
GREEN on the first turn each, $0.38 and $0.22), the cockpit driven headless with
devtools/cockpit-drive. Every step behaved as written; the guide now prints the real values
(808 bytes, the header and checksum bytes, the 13 findings, the mutation line) and five small
corrections. Part 11 (speed) is half done: it needs a quiet machine.

**Checked** (workflow `wf_6812de33-51b`, 17 Opus agents, report docs/reviews/2026-10-07-perf-fix2-check.md):
every area's fixes hold; the checkers found 0 high, 6 medium and 22 low problems, left for the next
session to triage (the main one: without facts, `perf show` and the cockpit judge a program-as-it-
stands row in opposite wrong ways). **Mutation checks** of the plan's rule list and fix pass 2's own
rules: 253 mutants — 185 killed, 52 survived and each got the test that kills it (merged), 14
equivalent (reasons recorded), 2 open (recorded). The weakest guards were currency's reasons and the
cockpit's change advice: several reasons had no test at all.

**Not done here:** `bench check --replay` was not run for this merge (fix pass 2 touched perf, the
cockpit's estimate and tests only — nothing verify, migrate or bench read); run it at the next merge.

## 2026-10-07 — Speed work, fix passes 3 and 4: the check round's findings fixed, checked, fixed again

**Fix pass 3** (docs/reviews/2026-10-07-perf-fix3-plan.md: the triage of fix pass 2's check round —
28 findings kept in five areas, 7 dropped with reasons; five Opus fixers in their own worktrees,
merged at ec78455, 1 275 tests green). In plain words: without facts, `perf show`, the cockpit and
harness-mcp no longer judge a program-as-it-stands row in opposite wrong ways — all three say the C
and the units it holds are not checked; the early `--as-it-stands-only` refusal names the units left
out and why; `perf show` stops at the first compiler that fails and no longer hides every row behind
one bad unit file; the cockpit's Measure dialog answers with certainty only for the `harness` beside
its own binary, gives a three-way launcher answer (current / will build / cannot use, with perf's own
refusal words) and never blocks the UI thread (the open mutation survivor closed); `perf run` refuses
a plan of more than 999 units by name before anything is built; the results reader reads the version
without building the whole file (peak 57 → 12 MB on a forged 4 MiB file), caps each rows list at 16
and checks a last try's units like the row's own; the "3.7× as slow" low end a float hair past 2 %
reads 1.03×, not 1.0× (the other open survivor); the launcher tests use bounded waits and a decoy with
a living parent; looping test children are killed when a test ends either way (eight week-old loops
had been found running).

**Its check** (docs/reviews/2026-10-07-perf-fix3-check.md: ten Opus checkers — holds + mutation
checks and a regression hunter per area, a docs checker, a whole-pass hunter; 86 mutants — 63 killed,
17 given a test by the checkers, 4 equivalent, 2 by design): every area holds; 0 high, 10 medium,
~25 low findings, triaged into **fix pass 4** (four areas, merged at c2fcf02, 1 301 tests green):
one selection with the plan cap first so `--as-it-stands-only` on a huge plan gets the size refusal
(it had printed a 39 KB "needs two" line), the left-out list capped at 10 then "and N more", the
post-build refusal in the early one's words; `perf show` reads what it can past a junk `program.json`
or a broken units folder and names orphan files without reading them; harness-mcp says
`program_checked: false` and `current: null` for rows nobody judged, names unreadable results files
(`unreadable`, ≤ 20) and the plan refusal (`note`); the cockpit shows unit rows without facts and
greys Measure for a plan over 999 units in perf's words; the cache answer refuses a `.lock` that is a
link as the run does; test loops bound themselves to their parent (`kill -0 $PPID`); a results-file
error kind ("results file: <path>: …", no longer "invalid plan: …"); the version is read only from an
object. The checkers' own killing tests (17) were adopted.

**Fix pass 4's check** (docs/reviews/2026-10-07-perf-fix4-check.md: three Opus checkers; 48 mutants —
35 killed, 7 given a test, 1 a finding, 4 by design, 1 equivalent): every item holds, 0 high, 3
narrow mediums, 21 low → **fix pass 5** (two fixers, the docs by the main session): `perf show` judges
currency without the plan cap (it had called a 1 000-unit plan's held units "not checked" while the
cockpit judged them); the capped list reads "; and N more left out"; the full run's progress line and
the post-build list follow the refusal's rule and plan order; a results file must be a JSON object;
harness-mcp lists the units with rows without facts (each with its rows, since `harness_unit` needs
the facts), names orphan results files (`orphans`, ≤ 20) and gives unreadable reasons without the
path; the cache answer gives the run's own error for a `.lock` that is a folder. **Decided: no check
round after fix pass 5** beyond the full gates — the pass is small and every item adopted a checker's
test or added one that fails without the fix; the next review of this area is the next track's.
**Decided: an unreadable facts file stays a whole-target refusal in the cockpit and harness-mcp**
(a corrupt ledger is worth stopping on); `perf show` alone treats it as missing, and the docs say so.

**`bench check --replay`** after all three passes: 198 reproduce (1 conformant, 197 drifted), 2
expected divergences, 0 problems — OK, no regression (27 min 44 s).

**Decided: harness-mcp copies the 999-unit words rather than calling the oracle** (its read model
stays harness-core only, and the cockpit's gate must not select units on the UI thread): the cockpit's
and harness-mcp's tests pin the same string as `check_plan_size`'s; a reword must change both.
**Decided: the own-build rule stays by location** — a build-identity check would need new CLI
surface; after reinstalling harness-cli alone the dialog's launcher line can be wrong, said in
PERF-DESIGN §3.11 and README (reinstall harness-tui with harness-cli). `perf_launcher_cached()` of the
2026-10-07 entry above is now `perf_launcher_cache()` with the three-way answer.

**Recorded, not fixed:** a bounded-depth pid rule (launchd neither parent nor grandparent) passes the
decoy test — no fixed decoy catches every such rule; the early `--as-it-stands-only` check runs
before the units-folder check (both refusals genuine); the cockpit refuses a target whose ledger unit
folder is unreadable where the CLI shows rows (pre-existing); `stale_launchers_go_but_never_one_in_use`
failed once at load ~100 (a flake to watch); without facts a unit row's `replaces` check is skipped.

## 2026-10-08 — Project map: design revision 1, and the person's decisions

**Revision 1** of docs/PROJECT-MAP-DESIGN.md (the four-lens review verified finding by finding, the
triage at the end of docs/reviews/2026-10-07-project-map-design-review.md, §9 of the design maps
every change). In plain words: one named build configuration per tool, stated by the person (or
proposed by a model and confirmed), with one closed flag grammar for every source of flags; entry
kinds `main` / `fuzz` / `driver`; duplicates held as pending alternatives, the closure recomputed
after a choice, a choice both of whose sides link left to the person; the link check described for
what it proves and does not; angle-bracket includes resolved; symbols read by `objsyms`, not `nm`;
the project root stays the target root with the ledger at `migration/` (first tool) or
`migration/tools/<id>/`; a shipped `migration/` refused until adopted; cargo and rustup run from
outside the project with the toolchain pinned; closed reasons instead of raw compiler lines; the
caps with one behaviour; the picture's security rules fixed before its design.

**Decided (the person, 2026-10-08): the seven proposals of §8 as recommended**, under one rule —
**simplicity means usability**: easy and intuitive, starting at the command line, and runnable from
the cockpit too. The cockpit therefore gets the same three acts (Map the project, Ask, Accept a tool)
as dialogs over the `harness project` commands in the build's step (e); only the project view is a
design of its own. The design is the session's to own.

**Next:** the revision's check (five Opus checkers), revision 2 if it finds more than wording, then
the build in §5's order — the map is chosen over the briefing's M5 for now (it is what stops a real
download at the door); M5 after the map's first accepted tool.

**Revision 2** (2026-10-08, after the check of revision 1 — five Opus readers: four lenses against the
code and the spike, one for the document whole; docs/reviews/2026-10-08-project-map-rev1-check.md
with the triage at its end). What it settles: every mapped tool under `migration/tools/<id>/`
(revision 1's "first tool at `migration/`" nested later tools inside the first's ledger); the
adoption of a ledger made elsewhere recorded **per computer, outside the project** and checked by
every command that opens a ledger (a record inside the project could be shipped); one
`config.toml` with several named configurations and the run name at `accept`;
`compile_commands.json` a proposal until named; the `-f` flag list named and `-fuse-ld` forbidden;
every cargo/rustc child run from the harness's work folder with the toolchain pinned and a
unit crate's files checked before cargo; the include rule fixed (whole path parts, the system's
search folders recorded, the system wins over a project header unless the configuration says so);
`objsyms` extended (kinds, weakness, commons) and verify's own `nm` uses moved to it; linking settles
duplicates in `map`, a model is asked only what linking leaves open, and a kind is a label the
harness guesses; multi-source compiles go per file on perf's path; staleness keeps today's rule and
`root_hash` is a notice. **Decided: a project with its own `migration/` folder is refused in
place** (move, rename or map a copy) rather than a second ledger location. The re-check of the
changed sections runs next; then the build in §5's order.

**Revision 2.1** (2026-10-08, after the re-check of revision 2 — three Opus readers,
docs/reviews/2026-10-08-project-map-rev2-check.md): two rules of revision 2 would have broken what
exists — the unit-crate file check refused zopfli's hand-written M0 crate and the harness's own
`target/` folder (now: the rule is what makes cargo run code — no `build.rs`, no `.cargo/`, a manifest
with no build, links, dependency, patch or target keys, and `build = false` in the harness's own
manifest), and the per-computer adoption rule refused the benchmark suite in every fresh worktree
and made the test suite write the person's real trust file (now: the file's path is injectable,
each record carries a token also written in the ledger, the benchmark suite is adopted as one root,
and adoption deletes only the harness's build folders). Also fixed in place: the walk's four
additions named and scheduled; the map profile's real reads and the dependency list's blind spot
(inline assembly); a file neither parsed nor compiled marks every closure with an outside symbol
incomplete; the header a compile actually used is recorded and an unsettled ambiguous include stops
`accept`; a need already met in the closure is met, not a collision; definer sets numbered once per
project, nested sets counted; `ask` builds nothing and `ask --build` is always allowed; the reply
and response files' lifecycles written honestly; the duplicate question carries bounded source
slices; `compile_commands.json`'s two-argument forms; verify's staticlib checks keep `nm` until
`objsyms` reads archives; `RUSTUP_TOOLCHAIN` takes the harness's own value with auto-install off.
**Decided: the compile's disk use stays unbounded for now** (a resource limit needs the launcher
pattern; the first item to add before the map runs on untrusted downloads at scale), said in §6.
A final reader checks 2.1; then the build in §5's order: (a), (c), (b), (d), (e), (f), (g).

## 2026-10-08 — Project map, step (a) built: the walk, adoption, objsyms, the sandbox and work folder, the first `project map`

**Built** from docs/PROJECT-MAP-DESIGN.md revision 2.2, §5 step (a), by five Opus builders in their
own worktrees, merged at b8e19bb (1 378 tests green; `bench check --replay --adopt` 198 reproduce,
2 expected divergences, 0 problems, no regression). In plain words: the walk prunes dot-folders by
name and the ledger by canonical path, never follows links (a file reached under two paths is one
file with aliases), records every issue instead of dropping it, and counts skipped folders; `--json`
escapes every character the display filter hides (serde never did). A ledger made elsewhere is
adopted once per computer before anything reads it: the file `$RUHARNESS_ADOPTED` or
`~/Library/Application Support/ruharness/adopted.toml` pairs each root with a token also kept in the
ledger; every command that opens a ledger checks it (the CLI with `--adopt`, the cockpit with a
question, harness-mcp refusing and never adopting; `bench` adopts a suite as one root); adopting
deletes only the harness's build folders; RuHarness's fixtures carry committed tokens and the tests
use a temporary adoption file. `objsyms` reads external symbols with kinds, weakness and commons.
Every compiler, cargo and rustc child starts in the harness's work folder with the toolchain pinned
and auto-install off, a `PATH` of absolute entries outside the project, and a unit crate's files
checked before cargo runs (a build script or a manifest that makes cargo run code refuses the unit);
the map has its own sandbox profile. `harness project map --target DIR` maps one folder: per-file
facts, include folders by whole path parts with the ambiguous-include fact and the header actually
used, a sandboxed compile with the dependency list, the closed compile reasons, the caps — and writes
nothing yet.

**Decided: `build = false` is not added to the harness's crate manifest** — it would move every
recorded attempt's candidate digest and break replay; the file check before cargo already refuses a
build script. Revisit with a re-record or a replay tolerance. **Decided: the walk's additions apply
to every caller** (the cockpit's tree and the features mirror stop descending folder links), since
one walk is the rule; the committed facts, findings and digests stayed byte-identical.

**Not yet (step b):** set-aside counts per folder, the project's-own-`migration/` sentence as its
own refusal, the 30-minute budget and the 200 000-name cap, the configuration's flags in the compile.

## 2026-10-08 — The map's step (c), parts 1 to 3, built and merged

**What was built** (main at 68649d2; fmt, clippy and the whole workspace green). Part 1: the ledger
folder beside the root with mapped tools under `migration/tools/<id>/`; `harness.toml` read version
first, with the file-list form (`files = [{path, include_dirs}]`, `configuration = {name, from,
flags}`, optional `map` and `picks`) and the flag grammar checked at load; `--tool <id>` on every
command with the lookup order (a root file, else the only tool, else "pick one"); `sync-runtime` one
block per tool. Part 2: the oracle builds a file-list target — `Base` carries the configuration's
flags and a per-file include table and every compile asks it for a file's arguments (the judge's
flags, the configuration's, then that file's folders); builds with several C sources compile each
file to an object with its own folders and link once, on perf's path; the driver's folders are the
unit file's own, its listed ones, then the folders of every header in the unit's include closure;
verify's driver-shape check reads the object with `objsyms` and refuses a name that is not an
identifier (`nm` stays for the Rust staticlib checks); the features mirror and the boundary check are
confined to the listed files and their reached headers, inside the root and never under the ledger.
Part 3: the scanner reads a file list through both include forms to closure, records walk errors,
too-large and non-UTF-8 files as notes instead of stopping, and prunes the ledger for both forms (a
`source_dir = "."` no longer scans the ledger's own `driver.c`); harness-detect dropped its own walk
for the confined one; the program digest hashes a v2 record (files, folders, configuration, link
arguments, run name) while v1 stays byte for byte; staleness keeps the rule "stale only where a scan
would record otherwise"; what a prompt may read is confined the same way.

**Decided.** A v2 unit verdict's `inputs` does not record the configuration's flags: a flag change
reaches the verdict through the v2 program digest, so folder-form `inputs` stay untouched. The
scanner's return value changed (`scan_reporting` gives `ScanNotes`), which no committed file
records. Every folder-form target stayed byte-identical: the 101 committed facts, zopfli's findings,
digests and verdict `inputs`. `bench check --suite targets/tractor --replay --jobs 6 --adopt` on 68649d2: 198
reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems, no regression.

**Part 4, merged** (fmt, clippy, 1 424 tests green): every command opens a file-list target
(`load_folder` and `TargetSection::folder` gone; `is_program_file` is the one rule for "the
program's own files": directly in `source_dir`, or listed); the cockpit's tree walks the whole
project confined with the ledger pruned and greys files that are neither listed nor reached as
"not part of this tool"; its snapshot carries the form (`target: TargetSection`) instead of a
`source_dir` string; every dialog names the target's own ledger; harness-mcp reads a tool through
the read model unchanged; end-to-end tests over a liblzg-shaped tool (scan, plan, detect, features
init, state status, sync-runtime) and a `verify` green through the CLI with the configuration's
flag reaching the build. **Step (c) is complete.** The planner needed no change: `source_hash`
already hashes the include closure the facts record.

## 2026-10-08 — The map's step (b) built; the review of steps (a) and (c) and its fix passes

**Step (b)** (three builders, merged): the configuration file `migration/map/config.toml` and
`--configuration`, `compile_commands.json` read as a proposal (POSIX word splitting, the
two-argument options, paths against `directory`, flags-differ facts), the caps (200 000 names, a
30-minute budget) and the set-aside counts per folder; programs, closures with pending
alternatives recomputed after a choice, the linker's weak/common rules, collisions, incomplete
closures with their reasons, shared files, between-program duplicates, libraries, ids and indexes;
the link checks of §3.5 (choices linked with the closure recomputed, nested sets, held programs);
the map file `migration/map/project-map.json` written only in full with `root_hash` and
`inputs_hash`, the screen, the project lock `migration/map/.lock` (taken by `map` and by
`sync-runtime` for the shared `AGENTS.md`), the first map's `migration/.gitignore`, and the
"project changed" notice in `state status` and the cockpit's read model.

**The review of (a)+(c)** (four Opus checkers at high: the oracle, the readers, security,
usability and the tests; docs/reviews/2026-10-08-map-steps-a-c-review.md) found two real gaps
that the design's words had hidden: every compile honoured the configuration's include flags
while no reader did (so the scan recorded one header, the compile read another, and the digest
never moved), and the scanner and the staleness check read includes differently (an X-macro
project stayed stale forever). The triage decided nine points (listed in the design's §9) and
three fix passes built them: A the readers (the shared resolver, one reader, staleness both ways,
unreadable files as facts, the loader's refusals), B the oracle (the runtimes without the
configuration, the configuration entry in file-list verdicts, mutants' `-iquote`, absolute
symbols seen by the driver-shape check, object and name-byte caps, the unit-crate check's gaps,
the mirror through `read_regular`, `/private/var/tmp` denied), C the person's side (a fresh token
on every `--adopt` with the two committed tokens untracked, a results-free `migration/` made
here, "an agent never adopts", hints with `--tool`, the tool's id on screen, one "no target here"
sentence, the `⊖` mark, the tests the reverts showed uncovered).

**Decided.** `-iquote` folders apply to quoted includes only (the triage's wording was loose; the
compiler decides). The boundary wrapper's template moved to version 4 and the pinned runtime
digest with it, so new verdicts' `boundary` toolchain entry differs from older ones; `status` does
not compare that entry, so nothing reads stale. The facts an ambiguous include lands on are in the
program digest but in no unit's include closure (recorded, not fixed: the configuration settles
such includes, and `accept` refuses while one is unsettled). The tokens are no longer committed:
once per checkout (adoption is by canonical path) the person runs `harness state status --target targets/zopfli --adopt` and
`harness bench status --suite targets/tractor --adopt`. Dot-folders are no longer scanned in the
folder form (none of the 101 committed roots has one).

**Gates.** fmt, clippy, the whole workspace green after each merge (one test fixed at the merge of
fix pass C: the cut-short-walk test now reads the facts from the map file, where the design puts
them). `bench check --suite targets/tractor --replay --jobs 6 --adopt` on c8d5f58 (all four
merged): 198 reproduce (1 conformant, 197 drifted), 2 expected divergences, 0 problems, no
regression — the boundary wrapper's new template and the fresh-token adoption changed nothing the
replay scores.

## 2026-10-08 — The check round over step (b) and the fix passes; fix passes D, E and F

**The check round** (four Opus checkers at high; docs/reviews/2026-10-08-map-step-b-and-fix-check.md)
closed the review of (a)+(c): every experiment re-run passed, the byte-identity claims held, all
sixteen reverts were caught. Step (b) worked on zopfli, the benchmark, liblzg and lz4, but its
link check compiled with the configuration's flags only (so a program whose `compile_commands.json`
carried a needed `-D` read "did not link" with nothing missing), its time budget covered only the
compile loop, nothing bounded memory or the map file's size, a failed compile could leak file
existence outside the root into the map, and a `config.toml` shipped by the download counted as the
person's statement. The triage decided thirteen points (in the bundle) and three fix passes built
them.

**Fix pass D, the map:** the link check compiles each file exactly as the map did (its own flags,
`-idirafter` for `system_headers`), refuses objects over 64 MiB and frees each after its last link;
a name the 64-probe budget leaves undecided is "not checked"; a need met only weakly pulls in its
strong definer (recorded `strong_over_weak`); `system_headers` joins the configuration digest (left
out when empty, so no digest moved); one deadline through hashing, parsing, the evidence reader,
every compile, link and probe (`limits_hit: budget`); a file over 8 MiB is hashed over its size and
first 8 MiB; names over 4 KiB are odd names, 64 MiB of name bytes is a limit, parser facts are kept
only for `.c` files that did not compile, `compile_commands.json` is read into typed entries (50 000
entries, 64 flags and 16 KiB per entry), a map over 64 MiB is a limit; a failed compile reads its
`-MD` list and drops `at` and `header` when they name anything outside the root; a shipped
`config.toml` is proposed (its hash recorded at first sight in the adoption file) until `--adopt` or
the person's edit states it; the closing line names what exists; the `under` label, text-included
`.c` files, `not_in_compile_commands`, the no-C refusal before the lock, bookkeeping flags dropped,
numeric index order, a driver's needs in each fuzzer's link, the between-programs list from
closures only; `-Wl,-ignore_auto_link` on Apple; ids cut to 64 minus the suffix. The session added
the one table for path flags (the map's own lacked `-idirafter`).

**Fix pass E, the person's side:** adoption records its time and a verdict older than it is marked
"made elsewhere" in `state status`, harness-mcp's status and the cockpit until `verify` runs it here
(a verdict carries no time of its own, so its file's modification time stands in — the weakness is
named in the code); the adoption line says each half only when true; one helper
(`runtime_view::command_line`) spells every hint with `--tool` when a tool is open and `--target`
when the process was started elsewhere; harness-mcp's adoption sentence carries the tool;
`features::invalid()` takes the ledger path so verify and promote name the tool's file; the first
command that makes a ledger writes `migration/.gitignore`; the tree keeps a selected outside file's
name and shows unlisted headers neutrally before a scan; the dialog names the tools' build folders;
`cockpit-drive` passes `RUHARNESS_ADOPTED` through; fourteen sentences made one plain sentence with
a next step; SCHEMAS' adoption paragraphs rewritten; "once per checkout".

**Fix pass F, the residuals:** each unit's hashed closure is the resolver's (`sources::unit_closure`,
used by `compute_inputs`, the planner's `source_hash` and status), so a header an ambiguous include
lands on stales the verdict; `-idirafter` in the grammar and the resolver (after the system);
`#import`, `#include_next`, `%:include`, a lone `\r` and a splice followed by blanks are read; a
folder named by both `-I` and `-isystem` is searched at the `-isystem` position; the include
reader's `<` rule keeps a running line start (a 1 MiB line in well under a second); the scanner's
and detect's tree walks use their own stack; the mirror's per-file cap is 64 MiB; a manifest with
`[project]` is refused; a dot-folder header the rule reaches is readable by a prompt; the
`configuration` entry hashes resolved forms and only the unit's own files; one skip note for scan
and detect. Benchmark case 043 (`%:include "driver.h"`) gained one include edge in its committed
facts; no plan or verdict changed.

**Gates.** fmt, clippy, the whole workspace green (one pty timing test in the cockpit's chat
end-to-end failed under the bench replay's load and passes alone). `bench check --suite
targets/tractor --replay --jobs 6 --adopt` on 6dd80d6: 198 reproduce (1 conformant, 197 drifted),
2 expected divergences, 0 problems, no regression.

**Left for later** (named in docs/NEXT-SESSION.md): the link check compiles its own objects
instead of reusing the map's; status builds a resolver per unit without caching reads; attempts,
driver validation, perf, bench, promote, gen-driver and `read_sources` still hash the facts'
closure (the safe direction); about seventy recursive walks remain in harness-scan's lint,
interface and mutate modules; the map profile's read allow-list (§8).

## 2026-10-08 — The map's steps (d) and (e) built; the check round over them

**Built and merged** (0682103): `harness project ask` — the model step through the `external`
hand-off like every other model call, `--build` proposing a configuration from the build files
into `config.proposed.toml`, the held duplicate sets and named programs as questions with strict
reply contracts, the reply file bound to the map's digests, traces under `migration/map/traces/`,
replay fixtures committed so CI needs no model; `harness project accept` — re-maps and compares
the digests, applies the person's picks, links again, writes the tool's `harness.toml` in the
file-list form with the configuration's own copy (`system_headers` as `-idirafter`), the guessed
link arguments, the run name, the map stamp and the picks; a library accepted without a link; an
accepted id kept across maps; a later map reporting what changed per tool; the cockpit's project
mode with the three dialogs (Map the project, Ask, Accept a program) over the same commands.

**The proof the design asked for:** zopfli mapped on a copy without its root `harness.toml`,
`config.toml` stating `flags = []`, accepted as `t-zopfli_bin --run-name zopfli` (the id keeps the
`_` of `zopfli_bin.c`): `scan --tool` writes `facts.jsonl` byte-identical to the committed one,
every unit's `source_hash` equals the committed plan's, and u001 verifies green. Three checkers
re-verified it.

**The check round** (docs/reviews/2026-10-08-map-steps-d-e-check.md): correctness, security and a
newcomer's cold start by the docs alone, which reached a verified unit on liblzg and on zopfli.
Nothing high. The findings that mattered: `accept` took a set's "settled by linking" choice from
the map file; re-accepting dropped sections the person had added; the map's closing line sent a
newcomer to `accept` before the build was stated and nothing showed `config.toml`'s shape;
`verify` printed an unrun check as a pass; the hand-off envelope was written down only in the
testing guide; `ask` called an untried choice "did not link". Fourteen decisions in the triage;
fix passes G (the map, ask and accept) and H (the person's side and step (f)'s docs) build them.

**Gates on 0682103.** fmt, clippy, 1 624 tests green; `bench check --suite targets/tractor
--replay --jobs 6 --adopt`: 198 reproduce, 2 expected divergences, 0 problems, no regression.

## 2026-10-08 — Fix passes G and H, the cockpit's narrator, the docs of step (f): the map is complete

**Fix pass G, the map, ask and accept.** `accept` takes nothing from the map file: it maps the
project again, recomputes programs, closures, duplicate sets and libraries with their link checks,
refuses when the file disagrees, and counts a set settled only when linking again finds exactly
one choice that links. Accepting again keeps every key and section the person added (the closing
line names what was kept). A program's written file carries a commented `[oracle.whole_program]`
example and the screen says the check is off until it is filled in. While the configuration is a
guess, the map's closing line shows `config.toml`'s lines and does not name `accept`. `ask` says
"the map did not link these choices (too many to try)" for a set over the limit, tells a guessed
map with nothing held to use `--build`, and names a driver as such. The cockpit's acts follow the
open question (Ask runs `--build` under a guess; Accept says a stated configuration is needed
before any picker; the picker skips an unreached set and shows the reply's advice labelled as the
model's, none preselected); the pre-terminal project mode now also opens when tools exist,
leading with "Open <id> — <program path>". Library ids follow their files. The map file records,
per accepted tool, what changed and the sentence to say; `state status`, the cockpit and
harness-mcp read that record. Slices sent to a model are capped in bytes with a one-pass search;
kept reply items are re-validated; repeated JSON keys and mark-only names are refused; the
"fixed field order" sentence left the design. `config.toml` refusals come all at once with the
relative path, the joined-`-I` hint and "warning or tuning flags: drop them". `migration/tools`
must be a real folder before any lock.

**Fix pass H, the person's side and step (f).** `verify` and `promote` print an unconfigured
whole-program check as "[SKIP] whole-program — not run: …" while the recorded verdict is byte for
byte what it was (zopfli's and two bench cases' verdicts proved identical). The hand-off envelope
is stated once in SCHEMAS and named on every awaiting line; a response that is not the envelope
is refused by name; the awaiting lines say to pass `--model` before answering, since the model's
name is part of the request key. `harness project --help` gives the order; scan and plan print
their next step; gen-driver says what it is doing; `state status` names the only tool it read;
harness-mcp's status note carries the notice. README gained "Start from your own C project",
the tutorial "Mapping a whole C project", the testing guide Part 12 "liblzg by map" (every
command run on a real copy, the real lines pasted), SCHEMAS a `config.toml` section. The cockpit's
narrator counts an unrun check as "not run" (fixtures and goldens re-rendered). A docs pass
re-ran Part 12 on the merged binary so every quoted screen matches.

**Gates on 91ec836 and after.** fmt, clippy, 1 646 tests green; `bench check --suite
targets/tractor --replay --jobs 6 --adopt`: 198 reproduce, 2 expected divergences, 0 problems,
no regression.

**Where the map stands.** Steps (a) to (f) of docs/PROJECT-MAP-DESIGN.md §5 are built, reviewed
in three rounds and fixed; a newcomer reached a verified unit on liblzg and on zopfli from a cold
start by the docs alone, and the zopfli tool accepted from the map is byte-identical to the
hand-written target. Step (g) is a process step: after the first accepted tool, record its
features before its first unit moves. Open, in words, in docs/NEXT-SESSION.md: four wording
items, the ask prompt's "in this order", `accept` link-checking every program again, the map
profile's read allow-list, and the leftovers from the earlier passes.
