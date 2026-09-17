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
`2` usage error (clap's own) · `10` oracle red. Pinned by integration test.

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
