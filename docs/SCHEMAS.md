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
