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
