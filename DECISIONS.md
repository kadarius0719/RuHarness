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

