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
