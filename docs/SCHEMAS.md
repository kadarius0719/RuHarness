# RuHarness ledger schemas — v1 (normative)

These are public contracts (§13.4): versioned, changed only deliberately. This document
is the normative spec; code implements it. Reviewed by an adversarial design panel
2026-09-17 (spike + review recorded in DECISIONS.md).

## Global rules

- **Versioning.** Every machine surface carries `schema_version` (integer). Additive,
  optional changes never bump it; a bump means breaking. A reader meeting a *newer*
  version refuses with a message naming the needed harness version. A reader meeting
  an *older* version: `facts.jsonl` → regenerate via `harness scan` (rescan IS the
  migration); `plan.toml` and verdicts → an explicit migration command producing a
  reviewable diff (to be shipped with the first v2).
- **Unknown-field policy.** Readers of all ledger files ignore unknown fields (no
  `deny_unknown_fields`); writers must preserve them. `plan.toml` is therefore never
  round-tripped through typed structs for mutation — it is edited surgically with
  `toml_edit` (preserves unknown fields, comments, formatting).
- **Enums.** `facts.jsonl` enums and record kinds are *open*: readers skip records
  with unknown `k` and pass through unknown enum strings. `plan.toml` enums are
  *closed*: growing `status` is a breaking change (skipping a status you don't
  understand is behavior-bearing).
- **Canonical serialization.** Deterministic byte output: fixed field order (struct
  order as specified here), per-kind sort keys (below), final tiebreak = full
  canonical-line byte order, one record per line, `\n` line endings, trailing
  newline, no wall-clock timestamps in any committed canonical file. A change to
  canonical bytes for identical input is a `schema_version` bump. Enforced by a
  golden byte-for-byte fixture test in `harness-core`.
- **Hashes.** `blake3:<64-hex>` of file bytes. **File-set hash**: for the sorted list
  of (repo-relative path, per-file hash) pairs, hash the concatenation of
  `<path>\0<hex-of-file-hash>\n`; render as `blake3:<64-hex>`. Never key anything by
  commit SHA.
- **Symbol identity.** `name` is the frontend's canonical unique identifier. C
  frontend: the linkage name for external symbols; `<repo-relative-file>::<name>`
  for internal (static) symbols. Refs carry the canonical id when `resolved`, the
  raw source name otherwise.

## facts.jsonl (committed, canonical; scanner-output-only)

Written only by `harness scan` (full regeneration, byte-identical on no-op).
Non-scanner facts (detector findings, agent annotations) will live in separate files
when their writers land (M2) — the reserved record kinds `annotation`, and symbol
kinds `type|global|macro`, ref kinds `type_use|global_read|global_write` are prose
reservations, not shipped variants.

Line 1 (frozen version-detection preamble — `k`, `schema`, `schema_version` will
never move or rename):

```json
{"k":"header","schema":"ruharness-facts","schema_version":1,"frontend":"c-tree-sitter"}
```

Records (sorted by the per-kind keys, kinds in this order: `file`, `symbol`, `ref`):

```json
{"k":"file","path":"src/zopfli/katajainen.c","hash":"blake3:...","includes":["src/zopfli/katajainen.h"]}
{"k":"symbol","name":"ZopfliLengthLimitedCodeLengths","kind":"function","file":"src/zopfli/katajainen.c","visibility":"public","signature":"int ZopfliLengthLimitedCodeLengths(const size_t* frequencies, int n, int maxbits, unsigned* bitlengths)","span":[172,262]}
{"k":"ref","from":"ZopfliLengthLimitedCodeLengths","file":"src/zopfli/katajainen.c","to":"src/zopfli/katajainen.c::BoundaryPM","refkind":"call","resolved":true}
```

- `file.includes`: project-local includes only, resolved repo-relative, sorted. Sort
  key: (path).
- `symbol.visibility`: `public` (reachable outside its defining file/module) |
  `internal`. Each frontend documents its mapping (C: extern/static). Sort key:
  (file, name).
- `ref`: sort key (file, from, to, refkind). `resolved:false` targets keep the raw
  name (libc/externals).
- Known gap (recorded): calls through function pointers are invisible to the C
  frontend (M2 detector material).

## plan.toml (committed; multi-writer, reconciled — never regenerated)

```toml
schema_version = 1
target = "zopfli"

[[unit]]
id = "u001-katajainen"
status = "pending"        # pending | in-progress | verified | merged | blocked
files = ["src/zopfli/katajainen.c"]
source_hash = "blake3:..."  # file-set hash of files + transitive project includes
symbols = ["ZopfliLengthLimitedCodeLengths"]   # canonical ids of owned public symbols
interface = ["int ZopfliLengthLimitedCodeLengths(const size_t*, int, int, unsigned*)"]
                          # informational display strings in source-language syntax;
                          # the authoritative contract is units/<id>/contract.md + the oracle
depends_on = []           # unit ids
test_strategy = "..."
done_criteria = "..."

[unit.oracle]
kind = "c-abi-differential"
# All other keys in this table are owned and validated by the OracleStrategy the
# `kind` names. For c-abi-differential:
driver = "migration/units/u001-katajainen/driver.c"
rust_crate = "katajainen_rs"        # dir name relative to units/<id>/
replaces = ["src/zopfli/katajainen.c"]
```

- **Block order is advisory.** The CLI topologically re-derives execution order from
  `depends_on` (unit-id tiebreak) on every load, and hard-fails on a reference to a
  missing unit or a cycle.
- **`harness plan` is reconciliation keyed by unit id** (Cargo.lock-style, via
  toml_edit): existing units keep `status`, human/agent-added fields, and comments;
  the planner updates its derived fields (`files`, `symbols`, `depends_on`,
  `source_hash`), appends new units as `pending`, and marks units whose files
  vanished `blocked` (with a comment) rather than deleting them. It never touches
  `status` otherwise.
- **Writer model:** planner owns membership/derived fields; `harness verify` owns
  `status`; humans/LLM own everything else (annotations, strategy text).

## Verdicts (committed evidence: units/<id>/oracle-latest.json, oracle-last-green.json)

A verdict is bound to what was tested — digests, not timestamps:

```json
{
  "schema": "ruharness-verdict",
  "schema_version": 1,
  "unit": "u001-katajainen",
  "green": true,
  "inputs": {
    "unit_source": "blake3:...",      // file-set hash: unit files + transitive includes
    "driver": "blake3:...",           // per-file hash
    "rust_crate": "blake3:...",       // file-set hash over a CLOSED list: Cargo.toml,
                                      // Cargo.lock (when present), and every non-dotfile
                                      // under src/ of the unit crate — nothing else in
                                      // the crate dir affects this digest, so unit
                                      // crates keep all compiled code under src/
    "replaces": ["src/zopfli/katajainen.c"],
    "toolchain": ["rustc 1.94.1 ...", "Apple clang ..."]
  },
  "checks": [ {"name": "differential-driver", "passed": true, "detail": "183832 bytes identical"} ]
}
```

- `harness verify` recomputes digests from the tree it actually tested and writes
  them atomically with the result. On green it also writes `oracle-last-green.json`
  and sets plan `status = "verified"`. On red it writes the red verdict, demotes
  `status` `verified → in-progress`, and leaves `oracle-last-green.json` intact.
- `verify` refuses to run (exit 1) when the unit's plan `source_hash` no longer
  matches the working tree — re-plan first.
- Verdict files are authoritative over the plan `status` field; `harness state
  status` flags contradictions in both directions: a done-claiming status
  (`verified`/`merged`) without fresh green evidence, and fresh green evidence
  the status never absorbed. `verify` never auto-demotes a `merged` unit — a red
  on merged is surfaced loudly for a human.
- `oracle-latest.md` is a human rendering of the same record (no timestamps); run
  logs with wall-clock time live in the gitignored build dir.

## harness.toml (committed, per-target root)

```toml
schema_version = 1

[target]
name = "zopfli"
source_dir = "src/zopfli"

[oracle]
# core-owned; the c-abi-differential kind needs all three of these tools
allowlist = ["cc", "cargo", "rustc"]
# all other keys are owned by the oracle kind, e.g.:
extra_link_args = ["-lm"]
```

(`[state]` profiles from the briefing §14.2 are design-recorded, implementation
deferred until artifact-writing code exists.)

## CLI contract

Subcommands, flags, and exit codes are append-only stable within a major version.
Human stdout is NOT a contract — machine consumers read the ledger files.

- `harness scan [--target DIR]` — regenerate facts.jsonl.
- `harness plan [--target DIR]` — reconcile plan.toml against facts. Refuses
  (exit 1) when facts.jsonl is stale vs the tree — run `harness scan` first — and
  validates the reconciled plan (order, deps, ids) *before* writing it to disk.
- `harness verify <UNIT> [--target DIR]` — run the unit's oracle; write verdict;
  update status. A crash or build failure of the migration candidate is a red
  verdict (evidence), not a harness error.
- `harness state status [--target DIR]` — the staleness detector: recomputes file
  hashes vs facts.jsonl, unit `source_hash` vs tree, verdict input digests vs tree;
  prints per-unit fresh/verified-but-stale/contradiction.

Exit codes: `0` ok/green · `1` harness error (including stale-plan refusal) ·
`2` usage error (clap's own) · `10` oracle red. Pinned by integration test. On
SIGINT/SIGTERM/SIGHUP the harness kills every live sandboxed process group and then
terminates BY that signal (`code() == None`; shells report 130/143/129) — see "CLI
hardening" below.

## facts.db (deferred to M2 — design frozen on paper, no code at M1)

Derived, read-only, gitignored SQLite export, rebuilt drop+create by scan. NOT a
stable data contract — the JSONL is; each rebuild writes `meta('schema_version', N)`
so tools can detect incompatibility. Tables: `meta(key,value)`,
`files(id,path,hash)`, `symbols(id,name,kind,file_id,visibility,signature,
start_line,end_line)`, `refs(from_symbol,to_name,to_symbol,refkind)`. Ships with the
first query consumer (M2 detectors or the MCP server).

---

# M2 additions: observer surfaces (v1)

Reviewed by a 3-lens adversarial design panel 2026-09-17; the id-stability,
freshness-binding, attribution, and review-surface blockers it found are fixed below.

## Writer model (all observer surfaces)

| File | Writer | Regenerated? |
|---|---|---|
| `migration/observer/findings.jsonl` | `harness detect` only | Yes — pure function of (tree, detector suite) |
| `migration/observer/annotations.jsonl` | humans/oracle via append | Never — human-owned evidence |
| `migration/observer/triage.jsonl` | `harness observe` only | On re-triage |
| `migration/observer/reviews.jsonl` | humans via `harness review` | Never |
| `migration/observer/observations.md` | render step of `observe` | Always — never hand-edited |
| `migration/observer/traces/` (gitignored) | adapters | May contain source (§12.3) |

## findings.jsonl

Header: `{"k":"header","schema":"ruharness-findings","schema_version":1,"detector_suite":"c-treesitter-v1","facts_hash":"blake3:..."}`
(`facts_hash` = file-set hash over the facts file records; `detect` refuses when facts
are stale vs the tree, like `plan`.)

Finding records, sort key (file, id):
```json
{"k":"finding","id":"f-<16hex>","detector":"macros","category":"macro-statement-body",
 "severity":"high","blocker":false,"human_mandatory":false,
 "file":"src/zopfli/util.h","file_hash":"blake3:...","span":[135,144],"occurrence":0,
 "message":"function-like macro with statement body and embedded allocation",
 "evidence":"#define ZOPFLI_APPEND_DATA(...)"}
```
- **`id` is content-keyed, not position-keyed**: `f-` + first 16 hex of
  blake3(`detector` ‖ NUL ‖ `category` ‖ NUL ‖ `file` ‖ NUL ‖ blake3-hex of the exact
  spanned source bytes ‖ NUL ‖ `occurrence`), where `occurrence` is the 0-based index
  among findings in the same file with identical (detector, category, spanned-bytes).
  Line shifts from unrelated edits and message rewording do NOT change ids; `span` is
  display data (1-based, end-inclusive, for every detector group).
- **Identity bytes per detector group.** "Spanned source bytes" means the flagged
  node's exact byte range for tree-driven groups (`macros`, `layout`, `fn-pointer`,
  `global`, `variadic`). Groups derived from the fact graph rather than a syntax
  node key on canonical identifiers instead: `nonlocal`/`concurrency` hash
  `callee ‖ NUL ‖ caller-canonical-id`, `alloc` hashes the function's canonical
  symbol id. Either way the id is independent of line position and message text.
- `file_hash` = the file's content hash at detect time. `observe` refuses (exit 1)
  when any finding's `file_hash` mismatches the current tree — run `harness detect`.
- `category` is an **open** enum, but behavior travels on the record: `blocker` and
  `human_mandatory` are set by the detector; readers obey the flags, never a
  category list (unknown categories are therefore fail-safe).
- No plan-derived fields: unit attribution happens at observe/render time via
  plan.toml + the facts include-closure. There are no unit-risk records here.

## annotations.jsonl (human/oracle findings the detectors cannot produce)

Same record shape as findings with `detector` = `"human"` or `"oracle"`, ids keyed the
same way. Appended, never regenerated; exempt from LLM triage (implicitly confirmed);
consumed by risk scoring (e.g. `ub-reliance`) and rendering. M0's two oracle-found
hazards (comparator UB, `maxbits ≤ 15`) are the founding entries.

## Risk score (computed at render time — never committed as records)

Deterministic pure function of (findings + annotations + facts + plan), v1: capped
weighted sum with **fixed absolute normalization caps** (documented in DECISIONS.md;
score churn from re-normalization is forbidden). Signals: pointer-density proxy 25%,
size×coupling 30%, UB/impl-defined (annotations + bitfield/union findings) 20%,
macro+global 15%, alloc 10%. Any finding with `blocker:true` pins the unit score to
≥90. A `dismiss` verdict removes a finding's weight **only after** a human
`uphold-dismiss` review exists; until then scores are computed with the finding
counted (asymmetric authority).

## triage.jsonl

Header: `{"k":"header","schema":"ruharness-triage","schema_version":1}` — no
provider/model/usage here (they are run metadata, recorded in the gitignored traces;
canonical bytes must not depend on which adapter ran).

Verdict records, sort key (finding): one verdict per finding (shared-header findings
are triaged once and joined into every affected unit at render time):
```json
{"k":"verdict","finding":"f-...","content_hash":"blake3:...","verdict":"confirm",
 "confidence":"high","rationale":"...","evidence":["src/zopfli/lz77.c:120-133"]}
```
- `content_hash` is **computed by the harness** (the model's echo is validated for
  pairing, then discarded): blake3 over `system_prompt_hash` ‖ NUL ‖ sorted batch
  finding-id list (comma-joined) ‖ NUL ‖ the finding's canonical JSONL line ‖ NUL ‖
  the exact slice bytes sent. It binds the verdict to the batch and slice actually
  serialized; it is a mispairing/staleness detector, not a proof of model behavior.
- `verdict`: `confirm | dismiss | uncertain` (closed). `confidence`:
  `high | medium | low` (closed).
- Verdicts whose finding id no longer exists in findings.jsonl render as
  **stale — re-triage**, never silently dropped.

## reviews.jsonl (human-owned)

Appended by `harness review <finding-id> (--uphold-dismiss | --reinstate) [--note ..]`:
```json
{"k":"review","finding":"f-...","action":"uphold-dismiss","note":"..."}
```
Discharges human-mandatory flags and unlocks score exclusion for dismissals.

## Triage call contract (injection posture, §12.1)

- One call per batch; batches group findings by owning unit (file-owner via plan;
  files owned by no unit form the `shared` batch), ≤10 findings per call, split
  preserving canonical order. Order within a call: sort by
  blake3(batch-key ‖ finding-id) — RNG-free; a recorded tradeoff: deterministic
  replay over position-bias mitigation.
- The prompt's trusted region contains ONLY harness-generated text: role, taxonomy,
  adjudication criteria, zero-authority policy, output schema, and per-finding
  metadata limited to id/category/severity/span/content_hash (never `message` or
  `evidence`, which embed source text).
- Source slices (span ±10 lines, ≤120 lines) are JSON-string-encoded with `<`
  escaped as the JSON escape `\u003c` (backslash-u-003c), wrapped in nonce delimiters
  `<c_source_<12hex> id="f-..." trust="untrusted">` where the nonce is the first
  12 hex of the request's content hash — deterministic for replay, not forgeable
  from inside a slice.
- Responses are validated: exactly the requested finding ids, closed-enum fields,
  content-hash echo match; rationale must be single-line (control characters
  rejected, ≤2000 chars) and every evidence entry must match `file:start-end`;
  one retry with the error appended (live), hard error (replay/external). A
  trace pair is recorded only after validation succeeds, under the ORIGINAL
  request's key (so a live run that needed its retry still replays). Trace files
  are keyed by the first 8 hex of blake3 over the canonical JSON of the
  `CompletionRequest`: `<8hex>.{request,response}.json` — any change to the
  request (findings, slices, prompt text) yields a new key, so stale traces never
  match. Findings are validated before prompt assembly (id `^f-[0-9a-f]{16}$`,
  category/severity `^[a-z0-9-]+$`, clean relative file path) so the trusted
  region can only ever carry harness-shaped text. The live provider's
  `api_key_env` must start with `ANTHROPIC_` — the target's `harness.toml` is
  hostile input and must not be able to name an unrelated secret.

## observations.md (rendered)

Deterministic render (no timestamps): units ranked by risk score; per unit: score +
signal breakdown, findings affecting it (via include closure) with verdicts,
unresolved human-mandatory items, stale verdicts, ordering implications; standing
caveats (categories the detector suite cannot cover: pointer arithmetic, type
punning, aliasing, indirect-call resolution — libclang-frontend material). Render
fails (exit 1) unless every current finding has a verdict or is an annotation —
stage-2 done-criterion, enforced.

## CLI additions

- `harness detect [--target DIR]` — regenerate findings.jsonl (refuses on stale facts).
- `harness observe [--target DIR]` — triage + render (refuses on stale findings;
  in `external` provider mode with missing responses: writes request files, exit 1
  with "awaiting N response(s)").
- `harness review <FINDING> (--uphold-dismiss|--reinstate) [--note S] [--target DIR]`.
- `harness sync-runtime [--target DIR] [--check]` — managed AGENTS.md block (§14.3);
  `--check` exits 1 when regeneration would change the block.

## harness.toml [llm]

```toml
[llm]
provider = "external"        # external | anthropic | replay
model = "claude-sonnet-5"    # Tier-2 default for observer triage (briefing §16)
max_tokens = 8192
api_key_env = "ANTHROPIC_API_KEY"
```

---

# M3 additions: executor + provider profiles (v1)

Reviewed by a 3-lens adversarial design panel 2026-09-19 (security lens verdict was
"flawed": three blockers — link-arg injection, plan-field path traversal, symbol
shadowing forging green — all fixed below).

## Trust boundaries

- **Target-owned files are hostile input**: `harness.toml`, `plan.toml`, all C source,
  and the differential driver. Consequences, all enforced in code:
  - Plan `id` and `[unit.oracle] rust_crate` must be single clean path segments
    (`^[A-Za-z0-9][A-Za-z0-9._-]*$`); `files`, `driver`, `replaces` must be clean
    relative paths (no `..`, not rooted, no control characters). Validated at plan
    load; every write/copy destination is additionally checked to be inside
    `migration/units/<id>/` after canonicalization.
  - `[oracle] extra_link_args` accepts ONLY `-l<name>` (`^-l[A-Za-z0-9_+.-]+$`).
  - Provider endpoints and credentials never come from the target. `harness.toml`
    names a **provider profile** and a model string; profiles live in USER-level
    config (`$RUHARNESS_PROVIDERS`, else `~/.config/ruharness/providers.toml`). The CLI
    never loads a target-local `.env`; the model string is only ever placed in the
    request body, never in a URL or header.
- **Model output is untrusted code.** The harness owns the candidate's `Cargo.toml`
  (no dependencies, no build script, `panic = "abort"`, empty `[workspace]`) and
  `src/lib.rs`:
  ```rust
  #![deny(unsafe_code)]
  #[forbid(unsafe_code)] mod logic;
  #[allow(unsafe_code)] mod ffi;
  ```
  so the compiler confines `unsafe` to `ffi.rs`. The model emits exactly
  `src/logic.rs` and `src/ffi.rs` (allowlist lookup, never path sanitization).
  Security boundaries are: the compiler-enforced lint structure, the **symbol-set
  check**, and the sandbox. The textual deny-scan is *quality feedback only*.
- **Symbol-set check** (oracle check `symbol-set`, applies to every verification):
  the candidate staticlib's defined external symbols that are not Rust-mangled, minus
  a baseline captured from an empty harness-owned crate built with the same
  toolchain, must equal the unit's `symbols` exactly. A candidate exporting `printf`,
  `malloc`, or any other extra global cannot go green. The same check rejects
  **pre-main constructors**: symbols placed in `__mod_init_func` / `__mod_term_func` /
  `__init_offsets` (macOS) or `.init_array` / `.fini_array` / `.ctors` / `.dtors`
  (Linux, when `objdump` is allowlisted and available) beyond the baseline's count —
  a static initializer could otherwise print forged output and exit before `main`.
- **Sandbox** (`sandbox-exec` on macOS) wraps every build AND every run of target- or
  model-derived code: network denied; reads under the user's home denied except the
  target root and the Rust toolchain dirs; writes confined to the unit's build/
  attempt dirs and temp. All oracle child processes get a scrubbed environment
  (`PATH`, `HOME`, `TMPDIR`, `CARGO_HOME`, `RUSTUP_HOME`, `RUSTUP_TOOLCHAIN` only) and
  a wall-clock timeout (`[oracle] timeout_secs`, default 120, from 1 to 604 800 — a week; a
  value outside that is refused at load; expiry = failed
  check). The mode applied is recorded in the verdict (`inputs.toolchain` gains
  `sandbox: <mode>`). Built-binary runs additionally deny `process-exec` of anything
  but the binary itself, and a timeout kills the child's whole process group. Where
  no sandbox exists (`sandbox: none`), EVERY command that builds or runs target- or
  model-derived code — `verify` and `migrate`, with any provider (an `external`
  candidate and the target's own driver run just the same) — refuses unless
  `--allow-unsandboxed` is passed.
- **What the oracle does NOT prove:** the `sanitizers` check instruments the C
  baseline and driver only. Stable Rust has no ASan, so the candidate's `ffi.rs` shim
  is not sanitizer-verified; its safety rests on the compiler-enforced shim structure
  plus the differential checks. (Nightly `-Zsanitizer` is recorded future work.)
- **Target-configured LLM spend is bounded:** `max_repairs ≤ 10` and
  `max_tokens ≤ 65536` are enforced at config load — a hostile `harness.toml` cannot
  turn one command into an unbounded stream of billable calls.

## Provider profiles (user-level, never target-owned)

```toml
[providers.ollama-anthropic]
kind = "anthropic"                 # anthropic | openai-compat
base_url = "http://127.0.0.1:11434"
# api_key_env = "ANTHROPIC_API_KEY" # optional; omitted = no auth header
context_tokens = 32768              # optional; enables truncation preflight
timeout_secs = 600
```
Built-ins needing no file: `external`, `replay`, `anthropic` (api.anthropic.com,
`ANTHROPIC_API_KEY`). `openai-compat` adds `max_tokens_field = "max_tokens" |
"max_completion_tokens"`. Adapters never send sampling parameters. When
`context_tokens` is set: preflight refuses (harness error, no attempt record) if
`prompt_bytes/3 + max_tokens > context_tokens`; after each call, reported
`input_tokens < prompt_bytes/6` is a harness error "prompt truncated by server" —
never recorded as a model outcome.

`CompletionResponse.stop_reason` stays the raw provider string (trace format
unchanged from M2); the normalized kind is derived: `end_turn|stop` → EndTurn,
`max_tokens|length|model_context_window_exceeded` → MaxTokens,
`refusal|content_filter` → Refusal, else Other.

## harness.toml additions

```toml
[llm.migrate]            # optional stage override (§13.2 per-stage routing)
provider = "ollama-anthropic"
model = "llama3.2-1b-32k"
max_tokens = 8192
max_repairs = 3          # 1 translate + up to 3 stateless repair turns
```
(`api_key_env` from M2 is removed from target config; ignored if present.)

## Attempts ledger: migration/units/<id>/attempts/<attempt-id>/

Committed evidence per attempt (source only; `attempts/**/target/` is gitignored):
`attempt.json` + `candidate/src/{logic.rs,ffi.rs}` (the last candidate written) +
`attempt-verdict.json` (last oracle verdict, when one ran).

```json
{"schema":"ruharness-attempt","schema_version":1,
 "id":"a-<12hex>","unit":"u001-katajainen",
 "provider":"ollama-anthropic","provider_kind":"anthropic","model":"llama3.2-1b-32k",
 "prompt_digest":"blake3:...",
 "unit_source":"blake3:...","driver":"blake3:...","toolchain":["rustc ...","sandbox: sandbox-exec"],
 "outcome":"red",
 "turns":[{"kind":"translate","result":"build","request_key":"8hex",
           "response_hash":"blake3:...","input_tokens":9120,"output_tokens":1400}],
 "candidate_digest":"blake3:...","promoted":false}
```
- `id` = `a-` + 12 hex of blake3(unit ‖ NUL ‖ unit_source ‖ NUL ‖ driver ‖ NUL ‖
  provider_kind ‖ NUL ‖ model ‖ NUL ‖ translate request_key) — content-derived, never
  a counter; re-running the same attempt (the normal path in `external` mode, where
  the process exits awaiting each response) resumes the same directory.
- `prompt_digest` = blake3(system ‖ NUL ‖ user) of the FIRST turn — the translate turn,
  or the steer turn of a seeded attempt (empty for a human attempt). Equal digests
  across attempts prove the same question was posed to different providers.
- Additive, optional (omitted when absent): `seeded_from` + `steer_note` +
  `seed_verdict` (a steer attempt: its seed, the reviewer's note, and blake3 of the
  seed's `attempt-verdict.json` bytes its first turn was rendered from — so the turn
  renders from the ledger alone, and a seed verdict modified afterwards is an
  integrity error; the three are recorded together or not at all), `note` (a human
  attempt's note). `Turn.kind` gains `steer` (the first turn of a seeded attempt) and
  `human` (the one turn of a hand edit). **The ledger defines no order over a unit's
  attempts** (content ids, no timestamps): every seed is named explicitly.
- **The requester label** (docs/CHAT-PANE-DESIGN.md §4, 2026-09-27): additive, optional
  `requester` — closed set `chat`: the act that created the attempt was asked for by a
  chat agent (the cockpit's chat, harness-mcp's acts) and its model turns may be answered
  there. A record carrying it is written with `schema_version: 2` (an older build refuses
  it — `SchemaTooNew` — rather than rewriting it without the label or scoring it); a record
  without it stays version 1, byte-identical. Readers accept both and refuse a record whose
  label is not `chat` or whose version does not match its label. The id of a labelled
  attempt mixes the label in: blake3(… ‖ request_key ‖ NUL ‖ `requester:chat`); unlabelled
  ids are unchanged. A labelled attempt's `external` hand-offs live in
  `migration/units/<u>/traces/chat/` (its live samples in `traces/chat/<id>/`), never the
  flat `traces/` — the request key alone would collide with a blind attempt of the same
  model. Samples inherit the label. Authorship: an unseeded labelled attempt is `chat`
  (a seeded one stays steered or human); provenance buckets pipeline, steered, chat, human;
  the benchmark reports a verified crate of chat provenance as a PROBLEM and replays chat
  attempts (reported `(chat)`). `blind` (harness-core): unseeded, `external`, no label —
  only the audited protocol answers or retries it.
- `harness migrate … --requester=chat` records the label; `--answer=FILE
  --answer-key=KEY [--answer-bytes=N]` (`external` and `--requester=chat` only; FILE `-` is
  stdin, as harness-mcp passes it — then N is required and a read of another length refused,
  so a writer killed midway never has a prefix filed; a terminal is refused) files the answer
  as the response to the pending request KEY:
  refused up front — typed `answer-refused` (exit 1), before the
  run writes anything of its own and without creating a directory (only an interrupted
  promotion is recovered first, as by every migrate) — unless the answer is UTF-8, at most
  512 KiB, not empty (a FILE read as the regular file looked at, never a link or a FIFO), the
  provider is `external`, the traces dir exists, and the
  attempt the run will resume (the base in progress; with `--retry`, the latest sample,
  n ≥ 2, in progress) is labelled (its model is the run's through the id), `KEY.request.json` exists there
  (re-serializing to KEY, naming that model; while the run's first request — the one the id
  was derived from — has no response, KEY is that request) and `KEY.response.json` does not;
  the adapter writes it (a new file, never over one; counts 0) only when the attempt asks for
  exactly KEY; a run that
  finishes or waits on another request without asking for it ends `answer-unused` (exit 1)
  after its own events (the `awaiting` event naming the request it now waits on); any
  other failure keeps its own kind.
- A human attempt whose judge wrote no `candidate/` (a deny-scan red) keeps its two
  files verbatim in `attempts/<id>/edit/src/{logic.rs,ffi.rs}` — so every human
  attempt holds what its `response_hash` hashes (in `candidate/src/` otherwise).
- `attempt.json` is rewritten atomically after EVERY turn (`outcome: "in-progress"`
  until the trajectory ends), so a crash leaves an accurate record.
- `outcome` (closed): `in-progress | green | red | blocked | truncated | format`;
  reserved for later milestones without a schema bump: `budget`, `thrash`.
  Turn `result` (closed): `green | format | check | build | oracle | crash-timeout |
  truncated | blocked`.
- Token fields are nullable: `null` = unknown (external hand-off, or a provider that
  reports no usage) — never `0`.
- `candidate_digest` = crate-content hash: the closed file list with paths relative
  to the crate dir, so it is location-independent and matches after promotion.
- **Finished attempts are immutable.** For a LIVE provider, re-running an attempt
  whose record is finished refuses unless `--retry` is passed; `--retry` records a
  NEW sample `a-<12hex>.r<N>` (N = 2, 3, …) in its own directory. Live traces are
  recorded per sample under `traces/<attempt-id>/`, so samples never overwrite each
  other. (`external` resumes are deterministic re-derivations of the same record.)
  For `external` (trace-backed), `--retry` (post-M4) first re-verifies the LATEST
  sample of the base id: if it reproduces it is returned and nothing is recorded; if
  it does not (the oracle or toolchain changed) a new sample `<base>.r<N>` is
  recorded, its requests answered by the recorded response files wherever the
  request key recurs and handed off where it does not. `bench check --replay`
  replays only the latest `external` sample of a base; earlier ones are reported
  `skipped (superseded by sample …)`. Without `--retry`, a finished `external`
  attempt that no longer reproduces is an error naming the flag.
- **Verification is evidence-first** (post-M4; docs/REPLAY-DESIGN.md, §R
  authoritative). `provider = "replay"` VERIFIES the attempt pinned with `--attempt`,
  else the one whose first-turn `request_key` is the one HEAD would send (none →
  refused, listing the unit's finished attempts). A re-run of a finished trace-backed
  attempt verifies it the same way. Verification never uses a provider adapter:
  1. **Integrity** (else an error, never a divergence): ≥ 1 turn; bound to the
     current `unit_source`/`driver`; the id re-derives from the recorded fields and
     turn-1 key; per turn, `<key>.request.json`/`.response.json` (the sample's own
     trace dir when it has one, else the root; `key` = 8 lowercase hex; regular files
     only, ≤ 16 MiB) — the request re-serializes to its key, names the record's
     model, turn 1 matches `prompt_digest`; the reply hashes to `response_hash`.
  2. **Strict tier:** the trajectory is driven on the RECORDED replies with HEAD's
     parser and judge in a scratch dir (budget = the recorded turn count); per-turn
     `result`, turn count, `candidate_digest` and `outcome` must match. A repair turn
     whose HEAD-rendered request differs from the recorded one ONLY inside
     `[EVIDENCE]` is a strict failure too (the judge's evidence changed); a difference
     anywhere else is a template or input change — drift, reported. Under `--provider
     replay` a strict failure is the typed error `Diverged` ("does not reproduce …");
     a re-run of a finished trace-backed attempt that diverges is a harness error that
     names `--retry`.
  3. **Conformance:** a turn is `drifted` when HEAD would render its request
     differently; reported as `prompt: conformant | drifted (turns …)`.
  It writes nothing to the attempts ledger and sends nothing. Every HEAD-rendered
  repair request is also checked to quote no scrub-list machine path in
  `[EVIDENCE]` (a harness error; nothing is sent).
- **`superseded.jsonl`** (`units/<id>/`, hand-written, append-only by convention;
  `{"schema":"ruharness-superseded","schema_version":1,"attempt","stage":
  "migrate"|"driver","reason","superseded_by","loosening"?}`, one object per line,
  the latest line per `(stage, attempt)` wins). `bench check --replay` requires, per
  entry: the attempt exists, is intact and DIVERGES (strict tier); its candidate is
  not what the benchmark scores (the unit crate / the promoted driver); its successor
  is finished, reproduces, has the same `unit_source` and `driver`, and is green if
  the superseded attempt was; the divergence is a TIGHTENING (recorded green, replayed
  not green) unless the entry says `"loosening": true` (listed as LOOSENING). An entry
  whose attempt was legitimately skipped (bound to superseded inputs, or a redundant
  `.rN` sample) is reported as no longer applying, not a problem. Any other failure is
  a problem; an entry never excuses an integrity failure. The reader refuses symlinks,
  files over 1 MiB, non-segment ids, and reasons outside 1..=400 bytes. Attempt
  records whose id is not their directory name are refused everywhere.
- **Prompt fixtures:** `crates/harness-llm/tests/prompt-fixtures/*.txt` hold HEAD's
  rendered requests per prompt branch; `cargo test` compares byte for byte
  (`RUHARNESS_UPDATE_PROMPT_FIXTURES=1` rewrites). A prompt edit lands with its
  fixture diff. The attempt-id DERIVATION is frozen for unlabelled attempts (a
  `requester` is mixed in, below); prompt BYTES are locked, not frozen.
- The `prompt truncated by server` and context-preflight checks apply to every
  stage (`observe` and `migrate`); a rejected call leaves no replayable trace.

## Emission contract (translate and repair turns)

For each file: the path alone on a line, a column-0 ` ```rust ` fence, the ENTIRE
file, a closing fence; then a final line `RUHARNESS_END_OF_OUTPUT`. Exactly
`src/logic.rs` and `src/ffi.rs` are accepted (exact allowlist match; last duplicate
wins). `<blocked>reason</blocked>` instead of code = outcome `blocked`. Truncation
(stop kind MaxTokens, `output_tokens >= max_tokens - 8`, or EOF inside a block) →
nothing is written, outcome `truncated`. Leading `<think>…</think>` spans are
stripped.

## Promotion protocol

On green, when promotion is due (see "CLI hardening": `--promote` > `--no-promote` >
`[llm.migrate] promote_on_green` > default; never from a replay run), or on
`harness promote`: (1) the final attempt record is written first; (2) the candidate's
closed file list is staged into the marker `units/<id>/.promote-<attempt>/`, whose
digest must equal the record's `candidate_digest`; (3) two renames: `<crate>` →
`.<crate>.prev` (when a crate exists), staged → `<crate>` — the marker stays; (4) the
normal `verify` runs IN PLACE — red, or an ERROR, ⇒ the crate is removed, `.prev`
renamed back, the marker removed, status/verdicts untouched; (5) green ⇒ the tail, every
step idempotent, in this order: `oracle-latest.json` → `oracle-last-green.json` →
`oracle-latest.md` → plan status `verified` → attempt `promoted: true` → `.prev`
removed → the marker removed LAST. Recovery (`recover_promotion`, run by every writing
command right after the writer lock) resolves every marker by EVIDENCE: the crate on
disk is the attempt's candidate (digest) or not — if not, the old crate is untouched or
moved aside, so `.prev` is restored when the crate is absent and the marker dropped; if
so, the committed verdict is green and bound to it (finish the tail) or it is not (roll
back as in (4)). A bare `.<crate>.prev` with no marker (the pre-marker protocol) is
resolved the same way. A promoted attempt is bound to the CURRENT `unit_source` and
`driver` digests; `harness promote` refuses otherwise (stale).

## CLI additions

- `harness migrate <UNIT> [--target DIR] [--provider P] [--model M] [--promote]
  [--retry] [--attempt ID] [--allow-unsandboxed]` — exit 0 green · 10 red/blocked/truncated/format · 1 harness
  error (incl. awaiting external responses, stale refusals, truncated-by-server).
- `harness verify` gains `--allow-unsandboxed` (see Trust boundaries).
- `harness state status` additionally flags a done-claiming status whose verdict is
  STALE as a CONTRADICTION, and prints a per-unit attempts summary.

---

# M4 additions: driver generation, benchmark suites (v1)

Reviewed by a 4-lens adversarial design panel 2026-09-23 (security and measurement
validity: "flawed"; architecture and feasibility: sound/feasible with fixes). The
reviewed design and its twelve resolutions are in docs/M4-DESIGN.md (§R is
authoritative); this section is the contract as implemented.

## harness.toml additions (all optional; additive)

```toml
[target]
include_dirs = ["test_case/include"]  # clean relative paths INSIDE source_dir; searched
                                      # after the including file's own dir

[oracle.whole_program]                # OPT-IN (was implicit and zopfli-specific before M4)
args = ["-c"]                         # flags only: ^-{1,2}[A-Za-z0-9][A-Za-z0-9-]*$, <= 4;
                                      # the harness appends the sample path
[llm.driver]                          # stage override, same keys + clamps as [llm.migrate]
provider = "external"
model = "claude-sonnet-5"

[driver]
max_mutants = 24                      # range 16..=64 (clamped from BELOW too)
min_kill_ratio = 0.6                  # range 0.5..=1.0 (recorded as permille)
```
Without `[oracle.whole_program]` the verdict carries one `whole-program` check,
passed, detail `not configured for this target` (migration note: zopfli gained
`args = ["-c"]`; its verdict is unchanged).

## Oracle checks added to every `verify` (c-abi-differential)

Order: `symbol-set` → `capabilities` → `driver-shape` → `differential-driver` →
`whole-program:*` → `sanitizers` → `boundary` (opted-in units only, and only when
everything before it passed — see "The boundary check"). A red `symbol-set`,
`capabilities` or `driver-shape` ends the run (nothing is linked or run).

- **Every C compile passes `-ffp-contract=off`** (Apple clang on arm64 fuses
  multiply-add even at -O0; the reference Linux build and Rust do not);
  `inputs.toolchain` gains `cflags: -ffp-contract=off`.
- **`capabilities`**: undefined symbols of the candidate crate's OWN archive members
  may not reach the classes `fs env process net os thread time dl syscall mem signal` (std paths
  by legacy mangling; libc names incl. process control: `kill raise ptrace sigaction
  signal getppid _exit …`) unless the C unit's own unresolved calls use that class;
  no `asm!`/`global_asm!`/`naked_asm!` anywhere in `src/`. `std::thread::local` is
  exempt (`thread_local!`). No candidate member found → fails closed.
- **`driver-shape`** (the driver is target-owned or model-written): compiled alone,
  its object defines exactly `main`; its undefined symbols ⊆ unit symbols ∪ a fixed
  libc allowlist (stdout/stderr printing, pure mem*/str*, malloc family, abs/div,
  libm, ctype, errno, compiler-emitted fortify/stack-protector names — fortify only
  as `__<allowed>_chk`); no weak references; plus a tree-sitter source lint (no asm,
  attributes, pragmas, `__` identifiers, function-like macros, `##`, digraphs,
  `#include` beyond the unit's headers and a fixed system set, unit symbols only as
  callees or prototypes, no `%p`, no `uintptr_t`/`intptr_t`).
- **Run confinement** for every run of a built binary: a fresh per-run `TMPDIR` (the
  only writable place), no reads under the home dir or the target root except the
  binary and listed inputs, `exec` of nothing but itself.

**Observable output = stdout AND stderr** (post-M4 oracle fix, 2026-09-23). A built binary's
run that exits 0 is compared on both streams by `differential-driver` and every
`whole-program:*` check (verdict and driver-validation `inputs.toolchain` gain a final
`observable: stdout+stderr` entry; a record without it was judged on stdout only); C and Rust sides are written to `build/<unit>/drv_{c,rs}.out`
(stdout) and `drv_{c,rs}.err` (stderr). Detail wording is replay-stable: when both
stderrs are empty and equal the detail is exactly the stdout-only form (`N bytes
identical` / `outputs differ (lens a vs b, first diff at byte i)`); equal non-empty
stderr appends ` (stderr: M bytes identical)`; a stderr difference reads `stdout
identical (N bytes); stderr differs (lens …, first diff at byte …)` or appends
`; stderr differs (…)` to a stdout difference. Repair evidence quotes stderr line
diffs after stdout's under `differential driver stderr, …`. Before any comparison,
every occurrence of the run's own fresh `TMPDIR` path in either stream reads `$TMPDIR`
(harness-injected per-run state, different for every compared run). Validation details
(`determinism`, `opt-levels`) label the stream: none (stdout only, the pre-fix
wording), ` on stderr`, or ` on stdout and stderr` (lens/first diff of stdout).
Migration note: before
this, only stdout was compared — M4's `014_pow_subfunction` (reports on stderr)
verified while wrong.

## The boundary check (design B, post-M4; docs/ORACLE-HARDENING.md §B, §B.R)

Opt-in per unit: `[unit.oracle] boundary = true` (kind-owned; a non-boolean is a plan
error). Every `verify` of an opted-in unit runs a ninth check, **`boundary`**, LAST and
only when every earlier check passed (recorded red turns keep their evidence). A unit that
does not opt in gets no `boundary` entry and no toolchain entry — its verdicts stay
byte-identical. When the unit opts in, `inputs.toolchain` gains `boundary:
sancov+guard-pages rt=<8 hex>` (blake3 of the harness-owned runtime, its headers, the
probe and the wrapper template), placed before `observable`, whether or not the check ran.

- **What it proves (§B.2):** for every call the unit's validated `driver.c` makes, the
  Rust touches only the driver objects the C touches during the same call, and within
  each only the elements inside the C's window (element = `sizeof *p` of the parameter;
  1 byte for `void *`/incomplete pointees). Objects are located by AddressSanitizer in
  the measure build (stack, heap, global; anything else passes through unshadowed and is
  counted); each object a call receives is copied into a fresh guarded shadow per call;
  pointers into shadows are relocated back on exit. Windows are the C's traced
  in-call hulls (unit compiled at `-O1` with sancov; `-O0` fallback), widened to the
  whole object when a learned untraced access falls outside (widened objects are named).
- **Details (closed shapes):** green — `Rust stays inside the C's footprint: <n> call(s),
  <m> guarded object(s) (<u> untouched by the C, <p> partially touched, <w> widened), <a>
  argument(s) unshadowed; tail and head layouts clean[; note: in call <k> the C reads the
  driver's stack through a pointer field (unchecked)]`; red (the candidate's, class
  `oracle`) — `in call <n> of <sym>, the Rust touched the object passed as `<param>` (<c> x
  <e> bytes) the C does not touch it in that call | below the C's window (elements [lo,
  hi)) | above the C's window (…)[ widened]; <tail|head> layout. …` (a widened window reads
  `[window widened to the whole object by an access the measurement did not trace]`), or
  `in call <n>, the Rust touched the object passed as `<param>` to call <m> of <sym>, which
  no longer exists: a pointer retained across calls (<layout> layout)`, or `the guard was
  tampered with (signal | handler | exception-port | canary | protection): …`, or `the Rust
  changed the driver's control flow (call <n> | arg <n>:<p>): …`, or `the guard record was
  altered (…)`; a different output — the standard `outputs differ …`; a crash elsewhere —
  `candidate run failed: …` (class `crash-timeout`), including `candidate run failed: the
  Rust ended the process inside call <n> of <sym>, which the C never does (…)` and
  `candidate run failed: the guard runtime stopped the run (<reason>; …)`; C side —
  `boundary driver invalid (C side): <harness reason>` (never candidate evidence: `migrate`
  turns it into a harness error exactly as a red `driver-shape`; `verify` demotes; `bench
  check` reports a PROBLEM). C-side reasons include an interface line that does not parse
  or names a type the unit's headers do not declare, a unit with no data-pointer
  parameter, a driver that does not run clean under its own measured windows, unit sources
  that disable instrumentation, and runtime limits (65 536 calls, 16 objects per call, 16
  MiB per object). A green detail names every widened object (`; widened (object-level
  only): call <n>: <sym>.<param>, …`, at most 6). The migrate judge applies the boundary
  explanation only to the fault and retained-pointer shapes; every other red keeps its
  class's explanation.
- **Fail-closed integrity (§B.R-1):** after every call the runtime verifies signal
  accounting (`ru_nsignals`), its own handler, the Mach exception ports, a canary page, and
  that every reservation's closed pages are still `PROT_NONE` and unaliased; a candidate
  that alters fault delivery or the guarded pages is red. A process that ends inside a unit
  call, or a run that makes fewer calls than the C, is red (the candidate's). `capabilities` gains the classes `mem`
  (mapping/protection) and `signal` (dispositions, exception ports, raw Mach messaging,
  thread creation, the traps); an opted-in unit's candidate is never granted either, and
  no candidate may reference `ruharness_*` or `__sanitizer_cov_*`.
- **Not proven (§B.9):** slice creation without access; memory reached only through a
  pointer field of an object (the driver-stack note above is the partial detector);
  unshadowed arguments and widened objects are object-level only. The unit's C is the
  reference by construction (a hostile unit can weaken its own check, never cause a false
  red). macOS + clang only until the Linux sandbox.
- **CLI:** `harness bench boundary --suite DIR [--case NAME]… [--allow-unsandboxed]` —
  calibration: the check alone on every verified unit, written nowhere; prints per case
  `GREEN | RED | VACUOUS | N/A | skipped` with the detail — VACUOUS is a passing check
  that guarded nothing (every data-pointer argument NULL, unshadowed or widened; not
  counted as green) — then one line per (symbol, parameter) with calls / untouched /
  partial / full / widened / unshadowed / null and a "no power" flag when the C never
  leaves the object untouched or partly touched, then totals; exit 1 only on harness
  errors. Nothing new is written to any ledger: the check is a function of `driver.c`,
  the unit source, the crate and the harness (`rt=`).

## Driver generation: `harness gen-driver <UNIT>`

Same trajectory engine as `migrate` (generate turn + ≤ `max_repairs` stateless
repairs; external/replay/live semantics; `--retry` samples; replay verification).

- **Attempts**: `units/<id>/driver-attempts/<d-id>/{attempt.json, candidate/driver.c,
  validation.json}`; traces/hand-offs in `units/<id>/driver-traces/`.
  `attempt.json` = the M3 attempt schema plus `"stage": "driver"` (optional field,
  omitted for migrate — pre-M4 records stay byte-identical); `driver` = `""`.
  Id: `d-` + 12 hex of blake3(`driver` ‖ NUL ‖ unit ‖ NUL ‖ unit_source ‖ NUL ‖
  provider_kind ‖ NUL ‖ model ‖ NUL ‖ generate request_key). Migrate ids are frozen
  (golden test). `Turn.kind` is OPEN, display-only: `translate | generate | repair`.
- **Emission**: the path `driver.c` alone on a line, a ```c fence (```C or an
  untagged fence with the path label tolerated), the entire file, closing fence,
  final line `RUHARNESS_END_OF_OUTPUT`; or `<blocked>reason</blocked>`.
- **Turn results** from the first failed validation check: `driver-build` → `build`;
  `driver-shape`/`symbols-called` → `check`; `determinism`/`opt-levels`/
  `sanitizers`/`mutation` → `oracle`; a timed-out run → `crash-timeout`.
- **Prompt confinement**: every source file put in any prompt (both stages) must
  resolve inside `source_dir`.

### driver-validation.json (`ruharness-driver-validation`, v1)

```json
{"schema":"ruharness-driver-validation","schema_version":1,"unit":"u-lib","green":true,
 "inputs":{"unit_source":"blake3:…","driver":"blake3:…","toolchain":["rustc …","Apple clang …","sandbox: sandbox-exec","cflags: -ffp-contract=off","observable: stdout+stderr"]},
 "policy":{"max_mutants":24,"min_kill_permille":600},
 "checks":[{"name":"driver-build","passed":true,"detail":"…"}, …],
 "mutation":{"sites":36,"sampled":24,"compiled":22,"equivalent":2,"killed":20,"survivors":[{"file":"…","line":5,"function":"rev16","operator":"literal"}]}}
```
Checks, in order, stopping at the first failure: `driver-build` (strict `-Werror=`
set on the driver's own TU), `driver-shape`, `symbols-called`, `determinism` (3 runs,
byte-identical stdout AND stderr, exit 0, stdout 1 B–256 KiB), `opt-levels` (-O0 ==
-O2, both streams), `sanitizers`, `mutation` (a mutant is killed when EITHER stream
differs from the pinned run, or it fails/times out).

**Mutation adequacy.** Sites: every operator site in function bodies of the unit's
`.c` files (outside preprocessor conditionals) — `arith relational logical bitwise
shift literal not-delete cast-delete signedness string-literal` — plus
`table-element` in file-scope initializer lists. Sampling (no RNG): each unit symbol
first gets up to `ceil((max/2)/|symbols|)` of its own body's mutants in
blake3-key order, the rest of the budget fills from all mutants. **Trivial Compiler
Equivalence**: a mutant whose `-c` object is byte-identical to the original file's is
`equivalent` — discarded, never counted. Gate over the counted (compiled,
non-equivalent) mutants n: n ≥ 10 → killed ≥ ratio·n; 1 ≤ n < 10 → killed ≥ n − 1;
every unit symbol with ≥ 2 counted mutants in its own body has ≥ 1 kill. 0 sites or
all equivalent → passes, flagged `n/a`. Sites but nothing compiles → harness error.

**Promotion.** Green → `units/<id>/driver.c` is written, RE-VALIDATED IN PLACE, and
only then `driver-validation.json` is stored (red → rolled back). gen-driver never
replaces a driver that has no validation record (human-written) or a configured
driver elsewhere; replacing a generated one needs `--promote`. **Provenance (R6):**
once `driver-attempts/` exists (or a validation record does), `migrate` requires a
FRESH green validation. A red `driver-shape` during `migrate` is a harness error (the
driver's fault), never translator evidence.

## Benchmark suites: `targets/<suite>/`

Layout: `suite.toml`, `corpus.lock`, `cases/<upstream path>/` (one harness target
each: upstream `test_case/` + harness files), `heldout/<upstream path>/{test_vectors,
runner}` + `heldout/tools/…` (never inside a target root), `scores.json`.

- **suite.toml** (v1): `name`, `[upstream] repo tag commit` (40-hex), `[[battery]] dir
  split` (`public|hidden`), DERIVED `[[case]] path split library symbol runner`
  (`library`/`symbol` from the runner's `harness!` literals, else the case-dir
  defaults), `[[excluded]] path reason`.
- **corpus.lock** (canonical JSONL, sorted by path): header
  `{"k":"header","schema":"ruharness-corpus-lock","schema_version":1,"repo","tag","commit"}`,
  then `{"k":"file","path","upstream","hash"}`; `upstream = ""` marks a
  harness-authored LOCKED file (`heldout/Cargo.toml`, `heldout/Cargo.lock`,
  `heldout/patches/**`). Verification: every locked file a regular file, `nlink == 1`,
  exact name, matching hash; every regular file under `cases/`, `heldout/` locked or
  harness-owned (`cases/<case>/{harness.toml,AGENTS.md,CLAUDE.md}`,
  `cases/<case>/migration/**`, anchored to suite cases); no symlinks or special files;
  no case-folded path collisions.
- **scores.json** (`ruharness-bench-scores`, v1): `suite`, `corpus_lock`,
  `scorer_lock` (digests), `environment` (rustc, cc, OS + arch, sandbox, baseline
  cflags), `totals[]` per split (`cases scorable verified strict_pass blind_spots
  oracle_false_negatives unscorable c_baseline_invalid vectors vectors_passed
  vectors_skipped`), `cases[]` sorted by path: `class` (closed: `strict-pass |
  blind-spot | unverified | unscorable | c-baseline-invalid`), `pipeline`, `inputs`
  (unit_source, driver, validation, rust_crate, candidate digests), `c_baseline`,
  `rust`, optional `candidate` counts, `vectors[]` sorted by name with results
  (closed: `pass skip timeout not-run fail:<cando ResultType> fail:dylib-build
  fail:build fail:no-report fail:bad-report fail:runner-exit-<n> fail:runner-killed`).
  A vector's `has_ub` → `skip`, excluded from every denominator.
- **scores.json — unmarked-UB additions** (post-M4, additive, `schema_version` stays 1;
  no `deny_unknown_fields`, so older readers ignore them and newer ones default them;
  every field is omitted when absent/zero, so earlier records stay byte-identical).
  Design and rationale: docs/ORACLE-HARDENING.md §A, §A.R, §A.2.
  - `vectors[].c_sanitized` (optional): the SANITIZED C pass, run only on a vector the
    plain C passed and the verified Rust or scored candidate did not (on a non-infra
    result). Closed: `clean` · `ub:<kind>` — EXCUSED; `<kind>` ∈ {`bounds-safety-trap`,
    `stack-buffer-overflow`, `stack-buffer-underflow`, `heap-buffer-overflow`,
    `global-buffer-overflow`, `dynamic-stack-buffer-overflow`, `heap-use-after-free`,
    `stack-use-after-return`, `stack-use-after-scope`, `use-after-poison`} ·
    `sanitizer:<other ASan kind | malformed-report>` (recorded, not excused) ·
    `fail:<cando ResultType>` · `timeout` · the infra strings above (a PROBLEM, not
    excused). An EXCUSED vector (`unmarked-ub`) is excluded from the case's non-UB set
    exactly like `has_ub`.
  - `cases[].sanitized_build` (optional): `asan+bounds-safety` | `asan` (the C does not
    compile with `-fbounds-safety`) | `none` (did not build: a PROBLEM).
  - `c_baseline`/`rust`/`candidate` counts gain `unmarked_ub`: an excused vector is
    counted there INSTEAD of pass/fail on every side. `totals[]` gain
    `vectors_unmarked_ub`.
  - `environment` gains exactly one of `sanitized-pass: asan+bounds-safety` ·
    `sanitized-pass: skipped (runtime-not-found)` · `sanitized-pass: skipped
    (unsupported-platform)` — so a baseline recorded without the pass is incomparable
    (exit 1) with one recorded with it: re-baselining is deliberate.
  - `bench check`: a vector whose excusal is lost while its MEASURED Rust result is
    not `pass` is a regression (exit 10); any other `c_sanitized` change is drift.

## CLI additions

- `harness gen-driver <UNIT> [--target] [--provider] [--model] [--promote] [--retry]
  [--attempt ID] [--allow-unsandboxed]` — exit 0 green · 10 red/blocked/truncated/
  format · 1 harness error (incl. awaiting external responses).
- `harness bench vendor --suite DIR --from CHECKOUT` — checkout's detached HEAD must
  equal the pin; never overwrites a vendored file with different bytes.
- `harness bench verify-corpus | status | init [--check] --suite DIR`.
- `harness bench score --suite DIR [--case NAME]… [--write]`.
- `harness bench check --suite DIR [--replay]` — re-verifies verified units,
  re-validates generated drivers, re-scores, compares per vector with the committed
  `scores.json`: exit 0 ok · 10 a Rust vector `pass` → not pass with unchanged
  inputs, or a re-verify/re-validate problem · 1 incomparable (environment, lock or
  membership differs, or a case's inputs changed — re-score required). A C-side
  flip is reported as environment drift, never a regression. With `--replay`, every
  finished attempt bound to the current inputs is verified evidence-first (above) and
  printed `reproduces; prompt: conformant|drifted (turns …)` or `expected divergence
  (superseded by …)`; a divergence without a valid `superseded.jsonl` entry, an
  integrity failure, or an invalid entry is a problem (exit 10). Totals line:
  `N reproduce (C conformant, D drifted), E expected divergence(s), S skipped, P
  problem(s)`. The benchmark's "promoted attempt" is the unique green attempt whose
  `candidate_digest` equals the unit crate on disk (not the `promoted` flag); none
  for a verified unit, or several UNASSISTED ones, is a problem. One implementation
  (`harness_core::attempts::provenance`): green, bound to the current `unit_source`
  AND `driver`, digest equal to the crate's; a steer attempt that reproduced its seed's
  candidate collapses into the seed; then by AUTHORSHIP (`attempts::authorship`, the
  `seeded_from` chain followed through every record of the unit): an unseeded model
  attempt is pipeline output; a steer attempt whose chain reaches a `human` attempt is
  that hand edit (`human`); any other steer attempt is `steered`. The benchmark scores
  UNASSISTED pipeline output only: a verified crate whose provenance is human (a hand
  edit, or a steer attempt of one) or steered (a reviewer's note guided it — free
  human text, which could carry the fix or the vectors, docs/M4-DESIGN.md §2) is a
  PROBLEM ("promoted from human (override) attempt … — not pipeline provenance", "…
  from steer attempt … (seeded from …; a reviewer's note guided it) — not unassisted
  pipeline provenance"), so `--write` refuses it; an unassisted model attempt with the
  same candidate outranks both. Every other pipeline figure of a case
  (`migrate_outcome`, the scored unverified `candidate_attempt`) counts unassisted
  attempts only. `--replay` reports human attempts `skipped (human)` (nothing to
  replay) and verifies a steer attempt from its record (`seeded_from`, `steer_note`,
  `seed_verdict`), its seed checked intact.

## Writer table additions

| File | Writer |
|---|---|
| `units/<id>/driver-attempts/**`, `driver-traces/**` | `gen-driver` |
| `units/<id>/driver.c`, `driver-validation.json` | `gen-driver` (promotion) |
| plan `[unit.oracle]` (only when absent) | `bench init`, `gen-driver` |
| `suite.toml` `[[case]]`/`[[excluded]]`, `corpus.lock` | `bench vendor` |
| `scores.json` | `bench score --write` |
| `units/<id>/superseded.jsonl` | a human (reviewed); verified by `bench check --replay` |

---

# CLI hardening (post-M4; docs/CLI-HARDENING.md, §R authoritative)

The milestone the review cockpit and the MCP bridge need from the CLI: they read the
ledger and spawn `harness` for every write.

## CLI contract additions

- `harness --json <cmd> …` — a global flag: stdout carries the `ruharness-events`
  stream (below) and nothing else; human logs and errors go to stderr. Without it,
  nothing changes.
- `harness migrate` gains `--no-promote` (record a green attempt without promoting;
  conflicts with `--promote`). Promotion precedence: `--promote` > `--no-promote` >
  `[llm.migrate] promote_on_green` > default (promote on green unless the unit is
  already verified). The awaiting-response hint prints the exact resume command,
  flags included.
- `harness promote <UNIT> <ATTEMPT> [--replace] [--target DIR] [--allow-unsandboxed]`
  — promote a recorded green migrate attempt (full id, positional) and verify it in
  place: exit 0 verified · 10 rolled back on a red in-place verdict · 1 refused. Refusals,
  in order, before any write: the id is a clean path segment; the record carries that id
  and belongs to the unit; it is a migrate record (`stage` absent), green, its last turn
  green, with a candidate digest; not already promoted (or `--replace`); the migrate
  preconditions hold (plan `source_hash` matches the tree; R6 — a unit with
  driver-generation history has a `validated` driver); the record's `unit_source` and
  `driver` equal the current tree's (`stale` otherwise); `candidate/` exists and matches
  the digest; the unit is not already verified/merged (or `--replace`). Requires the
  sandbox like `migrate`.
- Signals: on SIGINT/SIGTERM/SIGHUP the harness marks itself cancelled (no child is
  spawned after it, and a child that ends after it is an `interrupted` harness error —
  never journaled evidence), SIGKILLs every live sandboxed process group, emits the
  events-mode `result`, and dies BY the signal.
- Writer lock: every writing subcommand (`scan`, `plan`, `verify`, `detect`, `observe`,
  `review`, `migrate`, `gen-driver`, `promote`, `sync-runtime` without `--check`, and
  `bench score|check|boundary|init` per case) holds an exclusive `flock(2)` on
  `<target>/migration/.lock` (gitignored, never deleted) from just after loading the
  target to exit; a `bench score|check` locks EVERY selected case up front and keeps
  the locks across both of `check`'s passes. Contention is a `locked` error, exit 1:
  `ledger is locked by another harness command (pid N, \`migrate u-lib\`, since
  <RFC 3339>); wait for it or stop it`. Readers (`state status`, clients) never lock:
  they READ the holder line, and a unit that looks inconsistent while a LIVE writer is
  at work is reported `write in flight`, not a contradiction (a dead holder's leftover
  line — every signal death leaves one — is ignored).
- SIGHUP is handled only when stdout or stderr is a terminal; a detached run (`nohup`,
  output redirected or piped) keeps its inherited disposition. SIGINT/SIGTERM are handled
  unconditionally. The handler's stderr line and events-mode `result` are written from a
  helper thread with a 250 ms budget, so a stalled consumer or a closed stderr never
  delays dying by the signal.

## harness.toml additions (optional; additive)

```toml
[llm.migrate]
promote_on_green = false   # default true; false = only `harness promote` (or an
                           # explicit --promote) promotes — "Accept = an explicit act",
                           # sticky across the several runs one `external` attempt takes
```

## The writer lock file: `migration/.lock` (gitignored)

Created on first use, never deleted. Its content is one JSON line written by the holder
under the lock — `{"pid":N,"command":"migrate u-lib","started":"<RFC 3339 UTC>"}` —
and truncated to empty on a clean release; a crashed holder leaves its line (the kernel
freed the lock; the line names the dead pid). Diagnostics only, never consulted for
staleness. Wall-clock is allowed: the file is outside every hashed set and non-canonical.
The harness never truncates through a link: a symlink or hard-linked `.lock`, or a
symlinked `migration/`, is refused.

## The events stream: `ruharness-events` (v1, `--json`)

Newline-delimited JSON on stdout, one compact object per line, no timestamps. First line
`{"k":"header","schema":"ruharness-events","schema_version":1,"command":"migrate",
"args":[…],"pid":N,"harness":"<version>"}`; last line `{"k":"result","exit":N}` (plus
`"signal":"SIGINT"` when the harness dies by a signal, best effort). `k` is an open
enum: consumers skip unknown kinds and ignore unknown fields; additive changes never
bump the version. Every value is the ledger's own, verbatim.

| `k` | fields |
|---|---|
| `message` | `text` — every human line the command prints |
| `facts` | `files`, `stale` (`state status`) |
| `unit` | `id`, `status`, `source_fresh`, `verdict {state: present\|missing\|unreadable, green (present only), stale: [source\|rust-crate\|driver]}`, `contradiction`, `write_in_flight {pid, command, started}` (only when a LIVE writer holds the ledger — `kill -0`; a dead holder's leftover line is ignored), `attempts [{id, provider_kind, outcome, bound}]` (`state status`, one per unit) |
| `turn-start` | `unit`, `attempt`, `index` (1-based), `kind`, `request_key` (HEAD's rendering) |
| `turn-end` | `unit`, `attempt`, `index`, and the Turn verbatim: `kind`, `result` (the closed set `green \| format \| check \| build \| oracle \| crash-timeout \| truncated \| blocked`, treated as open), `request_key`, `response_hash`, `input_tokens`, `output_tokens` |
| `attempt` | `unit`, `id`, `outcome`, `provider`, `model`, `promoted`, `promotion` (the reason, courtesy) |
| `check` | `unit`, `name`, `passed`, `detail` — one per oracle check (`verify`, `migrate`'s final judged turn, a promotion — including a rolled-back one, whose verdict is not stored) |
| `verdict` | `unit`, `green`, `path` — only after the verdict was stored at `path` (`verify`, migrate's final judged turn at `attempts/<id>/attempt-verdict.json`, a green promotion); a rolled-back promotion emits its `check` lines and `promote{result:"rolled-back"}` but no `verdict` |
| `promote` | `unit`, `attempt`, `result` (`verified` / `rolled-back`) |
| `awaiting` | `attempt` (null for triage), `path`, `resume` (the exact re-run command), `request_key` (the pending request's trace key, when the path names one) |
| `error` | `kind` (`locked` / `stale` / `awaiting` / `interrupted` / `answer-unused` / `answer-refused` / `harness`), `message`, `holder` (locked only) |

The kinds come from typed `harness_core::Error` variants (`Locked`, `Stale`,
`Awaiting`, `Interrupted`), not from prose matching.

## Writer table additions

| File | Writer |
|---|---|
| `migration/.lock` | every writing command (holder line; truncated on release) |
| `units/<id>/<crate>/**`, `oracle-latest*.json`, plan status | `migrate` (promotion), `promote`, `verify` |
| `units/<id>/.promote-<attempt>/` (marker, transient) | `migrate` (promotion), `promote`; resolved by recovery |
| `units/<id>/<crate>/target/**`, `Cargo.lock`; `units/<id>/.replay-*/` | `bench score`, `bench check` (builds; scratch) |

---

# Review acts: steer attempts and human attempts (docs/TUI-DESIGN.md §5, §R authoritative)

## CLI contract additions

- `harness migrate <UNIT> … --steer <NOTE> --from <ATTEMPT>` — a NEW attempt seeded
  from a finished attempt of the unit. The two flags go together (either alone is
  refused, exit 1, listing the finished attempts bound to the current inputs).
  Refused before anything is sent: `--from` not a clean attempt id, not an attempt of
  the unit, a driver attempt, still `in-progress`, bound to superseded inputs (the R-5
  binding), without a `candidate/`, with a `candidate/` that no longer matches its
  `candidate_digest` (an integrity error), or without `attempt-verdict.json`; a note
  that is empty, over 2000 bytes, holds a control character other than `\n`/`\t`, or
  has a line that looks like a prompt section header (`[WORDS]`). The first turn (kind
  `steer`) is repair-shaped and built from COMMITTED evidence only: `[CURRENT RUST]` =
  the seed's candidate files; `[FAILURE CLASS]`/`[EVIDENCE]` = the seed's stored verdict
  (green: a fixed "passed every check" lead-in; red: the failed checks quoted as a
  repair turn quotes them, WITHOUT the driver-output excerpt, which lives in the
  gitignored build dir of whatever ran last); `[HISTORY]` names the seed; `[GUIDANCE]` =
  the note, verbatim (after `[HISTORY]`, outside the `[EVIDENCE]` range the render and
  drift rules look at); `[TASK]` = the steer task. Every later (repair) turn of a steer
  attempt carries the same `[GUIDANCE]`. Prompt fixtures: `migrate-steer-red.txt`,
  `migrate-steer-green.txt`, `migrate-steer-repair.txt`. A note that starts with `-`
  must be passed attached (`--steer=<note>`): clap reads a separate `-…` word as a
  flag (clients always pass `--steer=<note>` and `--note=<text>` as one argv
  element). **Integrity on every verification path** (a pinned replay, the re-run of
  a finished trace-backed attempt, `--retry`'s re-verification, `bench check
  --replay`): the record's `seeded_from`/`steer_note`/`seed_verdict` must equal the
  first turn being verified, the recorded turn 1 must be of kind `steer` and pose the
  note as `[GUIDANCE]` right before `[TASK]` with its `[HISTORY]` naming `attempt
  <seed> ` (a non-steer record's turn 1 poses no `[GUIDANCE]`), and the seed's
  `attempt-verdict.json` must still hash to `seed_verdict` — any mismatch is an
  integrity error, never drift and never a divergence.
- `harness override <UNIT> <DIR> [--note <TEXT>] [--target DIR] [--allow-unsandboxed]` —
  a hand edit's only way into the ledger. Reads exactly `DIR/src/logic.rs` and
  `DIR/src/ffi.rs`; refused (exit 1): DIR inside the target's `migration/`; any other
  `src/` entry but a `lib.rs` equal to the harness-owned one; a `Cargo.toml` that differs
  from the harness-owned manifest; a symlink or non-regular file; a file over 1 MiB; a
  note over 400 bytes or with a control character; source byte-identical to an existing
  attempt bound to the current inputs ("identical to attempt …; nothing to record").
  Takes the writer lock, recovers promotions, checks the migrate preconditions (plan
  staleness, R6). The migrate stage's one judge runs over the two files in a new
  `attempts/<id>/` (deny scan, harness-owned `Cargo.toml` + `src/lib.rs`, oracle,
  `attempt-verdict.json`, `candidate_digest` post-build). Record: `provider` and
  `provider_kind` `human`, `model` `-`, `prompt_digest` empty, one turn `{kind: human,
  result: green | the judge's class, request_key: "", response_hash: blake3(logic ‖ NUL
  ‖ ffi), tokens null}`, `outcome` green/red, `note`. Id = the frozen derivation over
  `(unit, unit_source, driver, "human", "-", response_hash)`. A judge harness error
  records nothing. A deny-scan red prints each violation (`override: deny scan: …`,
  a `message` event under `--json`) and keeps the two files in `edit/src/`. An
  `in-progress` human record under the edit's id (or a record-less dir) is an override
  killed mid-judge: the same edit reclaims it and is judged again (the identical-
  source refusal skips unfinished HUMAN records; an unfinished MODEL attempt still
  counts). Exit 0 green · 10 red · 1 refused. Never promotes; `harness promote`
  promotes a green human attempt like any other.
- `awaiting` event: additive `args` — the command line after the program name,
  verbatim, without `--json`, for a client that re-runs instead of parsing `resume`.
  `resume` is the human hint: every value POSIX single-quoted and attached
  (`--steer='- keep it'`, `--target=…`), `--from`/`--steer` kept, global flags
  (`--json`) omitted.
- `state status` / the `unit` event: additive `promotion_interrupted` — the attempt id of
  a `.promote-<id>/` marker, or `legacy` for a bare `.<crate>.prev`, reported only when no
  live writer holds the ledger, INSTEAD of `contradiction`; human line `<< promotion of
  <id> interrupted — the next writing command recovers it`.

## Writer table additions

| File | Writer |
|---|---|
| `units/<id>/attempts/<human id>/**` (incl. `edit/`) | `override` |


---

# The person's features (docs/FEATURES-DESIGN.md governs; additive)

A target without `migration/features/features.toml` is unchanged in every byte: verify's checks,
verdicts, attempt records, `state status`, harness-mcp's reports, the events.

## `migration/features/features.toml` (`ruharness-features` v1, hand-written)

`schema_version = 1`; `[[feature]]` (`id`, `name`) and `[[scenario]]` (`feature`, `id`, `args`,
`input`). Ids `^[a-z0-9][a-z0-9-]{0,23}$`; names 1–60 characters, no control characters,
display-only (never in a check name, verdict, event or prompt); args 0–8, each `{input}`, a flag
`^-{1,2}[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$` or a word `^[A-Za-z0-9][A-Za-z0-9_.#+=:,-]*$`, 1–64
bytes, never `/` or `..`; input one of `sample:text`, `sample:rand`, `sample:empty` (the
whole-program samples), `{input}` exactly once iff an input; ≤ 16 features, ≤ 8 scenarios per
feature, ≤ 16 in all, every feature ≥ 1 scenario; ≤ 64 KiB. **Strict**: an unknown key, a
wrong type, a duplicate or unknown id is refused with its key path — so **every new key bumps
`schema_version`** (an exception to the pass-over-unknown-fields rule). A file that cannot be
used is a value on every read path (the snapshot's `Invalid`), never an error.

## `migration/features/map.json` (`ruharness-features-map` v1, written by `features map`)

`{schema, schema_version, inputs {facts, features, program, platform, probe?}, unwatched [[file,
id]], unwatched_reasons? [{file, id, kind, detail?}], scenarios [{feature, scenario, end,
stdout_bytes, stderr_bytes, stderr_head, stable, probe_agrees, noted, reason?, functions [[file,
id]]}]}`. `probe` is the probe that made it (today `compiler-guided-2`; a map from another probe reads "made by another version of the harness" — its reasons' details shown safely, never refused); a map without it reads out of
date ("made by an older harness"). `unwatched_reasons` (docs/FEATURES-PROBE-REDESIGN.md §3.7):
`kind` ∈ `parser | not-a-block | conditional-brace | skipped-branch | naked | stringized | data |
compile | elimination | link | file-limit | not-checked`, `detail` ≤ 160 bytes with no control
character (a compiler's message may quote the target's source: display-only, never in prompts,
events or harness-mcp), each pair in `unwatched`. `end` ∈ `exit N | signal N | timed out | too
much output | could not start`; `noted` ∈ `complete | unavailable` (`reason`: `none written |
unreadable | the probe's setup did not run`); `stdout_bytes`/`stderr_bytes` count the first run's streams with paths as `$TMPDIR`/`$PROGDIR`
and no `$$` escape (for a program that prints no path, its own bytes);
`stderr_head` printable ASCII, ≤ 100 bytes, from the rewritten stream (`$TMPDIR`, `$PROGDIR`, `$$`). Current iff all five inputs equal
today's. Read strictly (hostile, committed); pairs today's facts do not know are dropped.

## Verdict and attempt additions

`VerdictInputs.features` (the digest the feature step ran under, or `invalid`),
`.program` (the program digest), `.features_skipped` (`<feature>/<scenario>: <reason>`, reason ∈
`c-side-unstable | c-side-crashed | c-side-timed-out | c-side-overflow | c-side-exec-failed |
c-side-build-failed | not-in-program`) — all omitted when empty, filled only by a run that
reached the feature step. `AttemptRecord.features`, `.program` — omitted when empty; a replay
judges with the recorded features when it can (none or `invalid` → none), else explains a
divergence first ("the features changed since it was recorded" / "other C changed since").
Checks `feature:<feature>/<scenario>` run last (after sanitizers and boundary): pass iff the
mixed program exits with the C's code and byte-identical streams; details are numbers only.

`UnitReport.features` / the `unit` event's `features` / harness-mcp's unit report `features`:
`"current"` or reasons from `not-yet | changed | invalid | program | skipped` — a marker beside
the verdict, **never** part of `stale`, `fresh_green` or the contradiction rule.

## Digests

`features` = blake3 over each feature's scenarios by id (args, input bytes) + the program's run
name + `[oracle] timeout_secs` (names excluded). `program` = blake3 over the top-level `.c` of
`source_dir` (canonical when a contained symlink), their facts include closure and every `.h`
under `source_dir` and `include_dirs`, as `(path, hash | missing)` (over 64 MiB reads as
missing), + `source_dir`, `include_dirs`, `[oracle] extra_link_args` — or the sentinel
`facts-stale` when the facts do not describe the program: a recorded file changed or gone, or
a file under `source_dir` they do not record (what a scan changes; never the same program as
any digest, itself included). Taken
before anything is built. Gap: non-C includes (`.inc`) are not in it.

## CLI

- `harness features init [--target]` — a starter (no feature); never overwrites; never through a
  symlink. Exit 0/1.
- `harness features save --expect <blake3|none> --bytes N [--target]` — the text on stdin
  (exactly N bytes, ≤ 64 KiB, UTF-8, not a terminal), saved only when it validates and the file
  is still the one `--expect` names. Exit 0/1. An outside editor racing it is not covered.
- `harness features map [--target] [--allow-unsandboxed]` — refuses without a file, scenarios,
  fresh facts, or a sandbox; writes `map.json`; events `scenario {feature, scenario, n, of, end,
  stable, probe_agrees, noted, functions}`. Exit 0 when written, 1 otherwise.
- `harness verify`/`promote` print what the features will do and each skipped scenario.

## Trust boundaries

`features.toml`, `map.json` and verdicts' skip lists are target-owned: only ids and closed
reasons reach checks, prompts, events or harness-mcp; everything shown is display-filtered.
Scenario runs: cwd = `run/` in their own temp dir (fixed-width name; `TMPDIR` the temp dir);
the binary at one path per scenario; streams
rewritten (`$`→`$$`, the temp dir → `$TMPDIR`, the program dir → `$PROGDIR`); the run profile
plus `(deny signal)` `(allow signal (target self))` `(deny process-fork)`; the process group
killed when the leader exits. Without the sandbox the candidate's inability to spawn or signal
rests on the deny scan and the capabilities check alone, and **a candidate run can write the
build dir** — the C binaries and the whole-program samples: the C program's bytes are noted
when it is built and checked before every C run, whole-program and feature checks alike (a
change fails every later check, never a skip); a rewritten sample is read by both sides alike;
C reads elsewhere are not covered (residual; the sandbox is the boundary). Every tool child (the compiler included) runs with
`SOURCE_DATE_EPOCH=0`. The map is shaped by the target's own C (it can write its notes file,
interpose libc): it gates nothing. The read preflight counts the program digest's files
(each once; ≤ 50 000) against its hash budget. Human CLI lines and errors show control
characters as `?` (a hostile file's key or a parse error's source line never drives the
terminal); `--json` escapes them.

## Writer table additions

| File | Writer |
|---|---|
| `migration/features/features.toml` | a person; `features init`; `features save` |
| `migration/features/map.json` | `features map` |
| `migration/build/.features/**`, `migration/build/<unit>/f/**` (gitignored) | `features map`; `verify` |

---

# C-vs-Rust speed (docs/PERF-DESIGN.md governs; additive)

perf only measures: it never changes a verdict, an attempt, the plan or a unit's crate sources.
A target without `migration/perf/workloads.toml` is unchanged by it in every byte; the cockpit
shows `Speed (no file)` and harness-mcp's `speed` is `null`. macOS only for now: elsewhere every
`perf run` is refused by name ("perf runs on macOS only for now — the Linux launcher is not built
yet").

## `migration/perf/workloads.toml` (`ruharness-perf-workloads` v1, hand-written)

`schema_version = 1`, then `[[workload]]` tables of `id`, `args`, `input`, `runs`. Ids
`^[a-z0-9][a-z0-9-]{0,23}$`, unique; `args` 0–8 strings, each ≤ 256 bytes, no NUL, `{input}`
only as a whole argument and exactly where an `input` is named; `input` (optional) a path relative
to the project — no control character, no empty part, no part starting with `.` or `-`, not under
`migration/`; `runs` 5–31 (default 15) runs a side. ≤ 16 workloads; ≤ 64 KiB. **Strict**: an
unknown key, a wrong type or a broken rule is refused with its line, column and workload — every
new key bumps `schema_version`. A file that cannot be used is a value on every read path (the
cockpit's `Speed (file error)`), never an error; a newer `schema_version` is refused.

The input is read once per measure through one confined, bounded read shared with the cockpit:
a regular file (not a link; a linked folder must stay inside the project and out of `.git` and
`migration/`), ≤ 64 MiB; otherwise the row is `input-unusable` with a closed reason `missing |
link | outside | into-git | not-a-file | too-large | under-migration | permission-denied |
unreadable`. The workload's digest = blake3 over its id, its args and, with an input, its name
and bytes (not `runs`: a row records the n it used); an input perf could not read is hashed as
absent, not as empty, so one that comes back as an empty file reads "your workload changed".

## `migration/perf/program.json` and `migration/perf/units/<id>.json` (`ruharness-perf` v1)

`{schema: "ruharness-perf", schema_version: 1, c_alone: [Row], as_it_stands: [Row]}` and
`{schema, schema_version, unit, rows: [Row]}`, one row per workload, each file ≤ 4 MiB, written
atomically after every row. A Row: `workload`, `outcome` (closed: `baseline | measured |
behaves-differently | stopped-by-sigkill | too-short | run-failed: timeout | run-failed: exit |
run-failed: signal | c-unstable | c-crashed | c-timed-out | output-too-large | c-could-not-start |
not-verified | replaces-mismatch | crate-does-not-build | does-not-link | mixed-panic |
input-unusable | could-not-start | run-failed: unmeasurable`), and as the outcome needs: `short`,
`runs` (5–31), `platform_metrics` (`macos-v6-pnorm | macos-v6-cycles | macos-v6-cycles-phases |
macos-v6-share | macos-v4-cycles | linux-cycles | linux-hybrid-summed | cpu-time`), `c` and
`other` (the timed runs: `instructions`, `cycles`, `cpu_us`, `wall_us`, `memory`,
`p_instructions`, `p_cycles`, `switches_voluntary`, `switches_involuntary`, `load`, `end`),
`std`, `fat_lto`, `profile` (`opt-level | lto | codegen-units | panic` set away from the
defaults), `step1`, `failed_run`, `setup` (closed facts: `runtimes`, `cause` ∈ `no-std |
two-no-std | lto | two-lto | unknown`, `units`, `index`, `log`, `input`, `reason` ∈ `not-fresh |
replaces-changed | rust-changed | accept-interrupted`, `attempt`, `never_started`),
`first_difference` / `found_before` (`stream` ∈ `stdout | stderr | exit`, `c_len`, `other_len`,
`offset`, `c_end`, `other_end`, `over_cap`, `kept [{name, size, blake3}]`; with `over_cap`,
`stream` is the stream that passed the 64 MiB cap, `c_len` the C's length on it, `offset` 0 and
`other_end` `signal 9`, the kill that stopped the run, as in its `step1`), `last_try`
(`{outcome, setup}`). `short` is true when one side's step-1 run was under both legs of the
floor (fewer than 1e9 instructions and under half a second of CPU); never on the C alone.
`inputs`: `workload`, `program` (the features' program digest), `crates [{id, digest}]` (unit
rows; a crate that does not build records its real digest), `replaces` (unit rows),
`program_name`, `units [{id, crate_digest}]` and `left_out [{id, crate_digest, reason}]`
(every as-it-stands row, set-up ones included; reasons `not-fresh | replaces-mismatch |
replaces-changed | crate-does-not-build | does-not-link | accept-interrupted`), `recipe`
(`perf-recipe-2`: rows of `perf-recipe-1`, whose short runs were judged by either leg of the
floor, read out of date), `launcher` (`perf-launcher-2`), `computer {os, build, arch, cpu,
two_kinds, fast_cores}`, `compilers {cc, rustc?}`. **Strict** (unknown fields refused; every
field checked against its outcome); free text ≤ 160 bytes, no control character; a
difference's lengths and kept files ≤ 64 MiB (the output cap), its `offset` no further than the
shorter length, and 0, 0, 0 on `exit`; a replaces-mismatch's `index` below 65 536 (and within
the row's `replaces` when it holds them).

**The replace rule** (one, in `harness-core`): a set-up outcome never replaces an earlier row that
is not itself a set-up row — it is kept beside it as `last_try`; on the C-alone rows the C's own
outcomes replace a C-side or too-short row and are otherwise kept as `last_try`; a
behaves-differently finding survives every re-measure that does not end `measured` or
`too-short`, kept as `found_before`. A SIGKILL perf did not send that a unit's or the program's
step 1 meets on the C is written to the C alone's row only when no baseline is there (perf
says when it keeps the baseline). Rows of workloads no longer in the file are dropped on the
next write.

**Current** iff every input equals today's: the workload's digest, the program digest, the
program's name, the recipe, the launcher, each unit's crate digest and its `replaces` (unit
rows), the held units, those left out and the plan's order (as-it-stands rows); the computer
and the compilers are checked only by `perf show` (when the launcher cache is current), never by
the cockpit or harness-mcp. Each reason has a closed token: `workload | workload-gone | program |
program-name | recipe | launcher | rust | replaces | left-out | accepted | verified | plan-order |
computer | compilers`, and the cockpit's own `measuring | too-large | input-unusable`.

## CLI

- `harness perf init [--target]` — the starter (no workload); never overwrites, never through a
  symlink. Exit 0/1.
- `harness perf save --expect <blake3|none> --bytes N [--target]` — the text on stdin (exactly N
  bytes, ≤ 64 KiB), saved only when it validates and the file is still the one `--expect` names.
  Exit 0/1.
- `harness perf run [--target] [--unit ID]… [--workload ID]… [--runs 5–31]
  [--as-it-stands-only] [--allow-unsandboxed]` — the writer lock (`perf run …` is the holder's
  command); refuses without workloads, with stale facts ("scan the project first"), unknown ids
  (naming the known ones). Without `--unit` / `--as-it-stands-only`: the C alone, each measurable
  unit (verified or merged, verdict green and fresh, no interrupted Accept), and the program as it
  stands; `--unit` alone builds and links only the units named. Each build step may write only
  its own folder (the compiles `.perf/obj`, each link its slot, each crate build its `target/`
  and `Cargo.lock`), and every object and staticlib is checked against its hash before each link.
  When the C fails on a workload in step 1 (one of its own outcomes, or a SIGKILL perf did not
  send), that workload's other rows are not run and keep their earlier rows. The progress names
  each left-out unit in words ("u-tree left out: its crate does not build"); the summary counts
  only what this run measured, and a row the replace rule kept is said to be kept. Exit 0 when
  it ran (a difference is a row, not a failure), 1 refused, 2 usage.
- `harness perf show [--target] [--no-check] [--allow-unsandboxed]` — every stored row's words,
  rebuilt, with why it is out of date. Creates and writes nothing in the target; never builds the
  launcher; refuses a link (or a non-folder) at `migration/`, `migration/perf` or
  `migration/perf/units` instead of reading through it. Besides the launcher's own `perfrun facts`
  (only when its cache is current), it starts only `cc --version` and `rustc -V`, as tool runs
  (the tool sandbox, the tool environment, the allowlist, `[oracle] timeout_secs`); "compilers not
  checked" when either is not allowlisted, either run fails, or no sandbox is available and
  `--allow-unsandboxed` was not given. `--no-check` checks neither the computer nor the compilers.
  Exit 0/1.
- Events (`--json`): `perf-row {side: c | program | unit, unit (unit rows), workload, outcome,
  words}` — `words` is a display-only courtesy (the CLI's line), never parsed.

## harness-mcp

`harness_status.speed`: `null` without a workloads file, else `{state: no-workload | file-error |
not-yet-run | c-only | units, units_measured, units_measurable, measuring, c_alone [row],
as_it_stands {units, units_omitted?, left_out [{id, reason}], left_out_omitted?, rows [row]}}` —
`units_measured` the units with a row perf timed or ran (not only set-up rows), `units_measurable`
those it would measure now; the held and left-out units listed up to 20 each, with how many more;
rows only of workloads still in the workloads file (one each, so at most 16 a side) — bounded
whatever the plan holds. Each unit's `speed` is its worst row; `harness_unit.speed` all of the
unit's rows, worst first. A row: `workload`, `answer` (closed:
`about-as-fast | slower | faster | probably-slower | probably-faster | close-call-slower |
close-call-faster | no-clear-difference | cant-tell-estimate | cant-tell-short-run |
cant-tell-too-few | cant-tell-slow-cores | baseline | too-short | behaves-differently |
stopped-by-sigkill | run-failed-timeout | run-failed-exit | run-failed-signal` and the C's and the
set-up's outcomes), `outcome`, `platform_metrics`, `short`, `runs`, `shift_percent {estimate,
low, high}` (only when the answer tells), `current`, `out_of_date` (the closed tokens above, each
once), `environment_checked: false`; the C alone's adds `cpu_seconds` and `memory_bytes`
(medians).
Ids and the workload are fenced as untrusted text; everything else is closed or a number.

## Trust boundaries

The results are **forgeable and non-canonical**: committed files any writer of the repository can
edit, and numbers of one computer at one time (another computer, or the same one busy, measures
differently). They gate nothing — no verdict, no promotion, no plan state — and are read strictly
(hostile, committed). The program runs only under perf's profile through the harness-owned
launcher (`perfrun`, outside the sandbox) and trampoline (`perfgo`, inside it): no fork (killed on
trying), no signal out, no network, no reads under the home folder or the target beyond its
binary, writes only its temp dir, and nothing started for it by the system — opening an app, a
document or a web address (LaunchServices), Apple events and launchd jobs are denied, and so is
reaching the services that do them; nothing it starts outlives its run. Other services of the
same user stay reachable (the profile starts from "allow by default"). Kept outputs are the
program's own bytes: shown only after their size and blake3 match the row, control characters
escaped. The launcher cache is per user, outside the target, built only from harness sources
with a compiler found through root-owned paths, and every sandbox profile denies writes to it.

## Writer table additions

| File | Writer |
|---|---|
| `migration/perf/workloads.toml` | a person; `perf init`; `perf save` |
| `migration/perf/program.json`, `migration/perf/units/<id>.json` | `perf run` |
| `migration/build/.perf/**` (fresh each run), `migration/build/.perf-out/**`, `migration/build/perf-logs/` (last 20) (gitignored) | `perf run` |
| `units/<id>/<crate>/target/**`, `Cargo.lock` | `perf run` (builds, as `verify` does) |
| `~/Library/Caches/ruharness/perf/perf-launcher-2-<hash>/` (outside the target) | `perf run` (only when stale) |
