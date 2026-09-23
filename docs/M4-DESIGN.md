# M4 design — TRACTOR benchmark, driver generation, scores as regression suite

Status: REVIEWED 2026-09-23 by a 4-lens adversarial panel (security: **flawed**;
measurement validity: **flawed**; architecture: sound-with-fixes; feasibility:
feasible-with-fixes). §R below is authoritative where it contradicts §0–§9 (kept as the
reviewed draft so the reasoning survives). Normative parts move into docs/SCHEMAS.md
"M4 additions" with the code.

## R. Review resolutions (authoritative)

**R1 — Driver forgery (security BLOCKER 1).** The driver runs on both sides and could
detect which side it is linked against (weak refs to Rust-runtime symbols, cached
stdout in temp, address arithmetic) and print a copy of the C behavior on the Rust side.
Fix — a gating `driver-shape` check, run by `validate_driver` AND by every `verify`
(human drivers included; u001's must pass it):
- *Object check* (`nm` on the driver compiled alone): defined external symbols =
  exactly `{main}`; undefined ⊆ unit `symbols` ∪ a fixed libc allowlist (stdio print
  family to stdout only, `mem*`/`str*` pure functions, `malloc` family, `abs` family,
  libm, compiler-emitted `___stack_chk_*`/`___*_chk`/`___stdoutp`/`_memset_pattern*`/
  `___chkstk_darwin`); no weak references (`nm -m`). No file, env, time, dl, process
  or syscall function is reachable.
- *Source lint* (tree-sitter): no `asm`, `__attribute__`, `#pragma`/`_Pragma`, no
  identifier starting `__`, `#include` only of the unit's own headers and a fixed system
  set, no `#define`/`#undef` of a unit symbol, unit symbols used only as callees (or in
  their prototype) — never address-taken, no `%p`, no `uintptr_t`/`intptr_t`.
- *Run confinement*: every built-binary run (driver, whole-program, mutants, scorer)
  gets a fresh per-run `TMPDIR`, may write only there, and may read NOTHING under the
  target root (a whole-program run may read exactly its sample). This also closes a
  latent M3 hole: a candidate could read the previous turn's `drv_c.out` from the build
  dir and replay it.
- Residual (recorded): address-distance side channels between unrelated objects that
  the lint cannot see without types.

**R2 — Held-out leakage (security BLOCKER 2, architecture BLOCKER 1, measurement 4).**
- *Layout*: held-out material is never inside a target root.
  `targets/tractor/cases/<upstream-path>/` = harness target (`test_case/` + harness
  files only); `targets/tractor/heldout/<upstream-path>/{test_vectors,runner}` +
  `heldout/tools/cando2`. Prompt-bound reads (both stages) are confined to
  `source_dir`; `include_dirs` must lie inside `source_dir`; the scanner's include
  resolution must land inside `source_dir` (code-enforced, not by discipline).
- *Capability parity (candidate gate, new oracle check `capabilities`)*: the candidate
  crate's OWN archive members' undefined symbols may not reach fs/env/process/net/os/
  thread/time std APIs (legacy-mangled `__ZN3std2fs…`, `3env`, `7process`, `3net`,
  `2os`, `6thread`, `4time`) nor libc `open*`/`fopen`/`stat`/`opendir`/`getenv`/`dlopen`/
  `dlsym`/`syscall`/`fork`/`exec*`/`posix_spawn`/`socket`, UNLESS the C unit's own
  unresolved refs use that capability class. `ffi.rs` may not contain the token `asm`
  (`asm!`, `global_asm!`, `naked_asm!`). A translated pure function cannot then read
  a vector (or `drv_c.out`) at verify or score time.
- *Scorer sandbox*: runner treated as a built binary — reads: the runner binary, one
  scratch case dir (copies, never links), system libs; writes: its output file's dir +
  fresh TMPDIR; exec: only the runner binary (procspawn self-re-exec verified to work
  under this profile by the feasibility reviewer). Output parsed as a closed enum with a
  size cap. Corpus lock re-verified after scoring.
- *Hand-off audit*: requests are copied to an isolated scratch dir outside the repo;
  answering subagents are instructed to touch only those files, AND their transcripts are
  mechanically audited afterwards (tool-call count, any path under `targets/`) — the
  audit result is recorded with the attempt. Blindness becomes checked, not assumed.
- *Contamination disclosed*: public vectors (Feb 2026) and the released hidden split
  (also Feb 2026, tag v1) predate the answering model's cutoff; the report says vectors
  were made "by hand or with the help of an LLM". The score is a **public-vector
  score**, not a contamination-free held-out measure.

**R3 — Wrong library/symbol names (measurement + feasibility BLOCKERs).** 42/42
B01_synthetic runners override `library:`/`symbol:` (34 use `"driver"`), and cando2's
default library name is the basename of `--test-root-dir`. `targets/tractor/suite.toml`
records per case `library` and `symbol`, extracted at vendoring time from the runner's
`harness!` literals (fallback: case dir / case minus `_lib`) and human-reviewable; the
scratch root is named `<case>`; `bench init` refuses a case whose `symbol` is not in the
unit's `symbols`.

**R4 — Platform artifact: FMA contraction (measurement BLOCKER 2, measured).** Apple
clang on arm64 fuses multiply-add even at `-O0`; the official Linux x86-64 build does
not, and Rust never does. Every harness C compile (oracle, driver validation, mutants,
scorer baseline) passes `-ffp-contract=off`; the flag is recorded in `toolchain`
(`cflags: -ffp-contract=off`). u001 is re-verified.

**R5 — Mutation gate (measurement 6, security 6).** Stratified sampling: up to
`ceil(max/|symbols|)` per symbol first (by enclosing function; statics' mutants count
toward their callers' symbols only via the kill rule below), then fill by hash order.
Operators add: cast deletion, `signed`/`unsigned` flip in body declarations, file-scope
static table element `n→n+1`, string-literal first-char change. Gate: n = compiled
mutants; n ≥ 10 → kill ratio ≥ `min_kill_ratio`; 1 ≤ n < 10 → killed ≥ n−1; plus every
unit symbol with ≥2 compiled mutants in its own body has ≥1 kill. Target config is
clamped from BELOW too: `min_kill_ratio ∈ [0.5, 1.0]` (default 0.6, recorded as
uncalibrated), `max_mutants ∈ [16, 64]` (default 24). Zero sites → passes with an
explicit `n/a (0 sites)` flag that the score reports. Kill-ratio distribution reported.

**R6 — Generated-driver provenance (security 6c/6d).** If `units/<id>/driver-attempts/`
exists, or the unit belongs to a bench suite, `migrate` and `bench` require a fresh
green `driver-validation.json` (deleting it cannot relabel a generated driver as
human-written). `bench check` re-runs `validate_driver`; it never trusts the committed
record.

**R7 — Scorer supply chain (security 4).** `heldout/Cargo.toml` + `heldout/Cargo.lock`
are harness-authored but hash-locked in `corpus.lock` (not exempt). Crates come from a
one-time `cargo vendor` into a gitignored dir, built `--frozen` with source replacement
and an empty `CARGO_HOME`: cargo checks every vendored file against
`.cargo-checksum.json`, whose package checksum must match the committed `Cargo.lock`.
Lock must contain only crates.io sources with checksums (no git/path deps outside the
corpus). Scorer builds run in the tool sandbox; `scores.json` records a `scorer_lock`
digest.

**R8 — `whole_program` (security 5, architecture 4).** `[oracle.whole_program] args`
accepts flags only (`^-{1,2}[A-Za-z0-9][A-Za-z0-9-]*$`, ≤ 4); the harness appends the
sample path. Without the table the verdict carries an explicit
`whole-program` check, passed, detail `not configured for this target`.

**R9 — corpus.lock hardening (security 8).** lstat: regular files only, `nlink == 1`, no
symlinks; case-folded path collisions rejected; exact byte-equal names; local patterns
anchored to suite case dirs (`cases/<case>/harness.toml`, `cases/<case>/migration/**`,
`cases/<case>/AGENTS.md`). Scorer builds and runs from a COPY of `heldout/` verified
after copying (verify what you use).

**R10 — Scores (measurement 3, 5, 7; architecture 5; security 9).**
- Suite `tractor-b01-lib` = B01 library cases, two splits: `public` (80) and `hidden`
  (20, released at v1). Vector discovery = regular `test_vectors/*.json` only (stray
  `.bak` files ignored); a case with zero scorable vectors (none, or all `has_ub`) is
  **unscorable**, listed, excluded from denominators (e.g. `008_long_run_lib`,
  `update_md5_lib`).
- Headline = **per-case strict pass** (every non-UB vector passes) over scorable cases,
  per split, with n; also per-vector rate and per-case mean. Label: "TRACTOR B01
  library subset (public / released-hidden), macOS arm64" — explicitly NOT comparable to
  the First Evaluation Report (150 tests incl. 51 executables, Linux containers).
- Per case also: verified?; held-out result of the verified crate; **blind spot** =
  verified but ≥1 non-UB vector fails; **oracle false negative** = the latest unverified
  candidate passes all vectors; driver mutation score + survivors; turns to green per
  stage; tokens (null here, stated).
- Canonical: cases sorted by path, vectors by file name; per-case recorded inputs
  (`unit_source`, driver, validation, `rust_crate` digests) + `scorer_lock` + an
  environment fingerprint (rustc, cc, OS version, sandbox mode, cflags).
- `bench check`: fingerprint mismatch → exit 1 "incomparable environment" (never a
  silent pass or a false regression); a C-side vector flip = environment drift
  (reported, not a regression); a Rust vector `pass → not pass` with unchanged inputs =
  **regression, exit 10**; a recorded input changed = re-score required (exit 1); a
  timeout is retried once and classified separately. Comparison is per vector, never
  whole-file bytes. `bench check --replay` additionally replay-verifies every recorded
  attempt (driver + migrate) — zero tokens; the claim is "regression suite for the
  deterministic pipeline; `--replay` extends it to prompts and trajectories".
- `bench status`: aggregate per-case pipeline progress from the ledgers (cheap).

**R11 — Contracts & refactor safety (architecture 2, 3, 6, 7).**
- Migrate prompt bytes and the migrate attempt-id derivation are FROZEN: a golden test
  asserts the recorded u001 translate `request_key`s and attempt ids reproduce under the
  refactored engine. Driver ids use their own derivation (`d-` prefix, stage mixed in);
  migrate ids never mix in a stage.
- Emission generalized to a file spec `{paths, fence_lang}`; the migrate spec keeps
  today's parser behavior byte-for-byte (existing emission tests unchanged + a
  golden over recorded responses).
- `Turn.kind` declared OPEN (display only; no behavior keys on it): `translate |
  generate | repair`. `AttemptRecord.stage` optional, omitted for migrate.
- Writer table: `bench init` becomes a writer of `[unit.oracle]` (only when absent; via
  `toml_edit`); `bench init --check` reports drift from freshly generated tables.
- Phase 2 note: the turn loop is reused; `validate_driver`/`mutants` are C-specific
  and would be rewritten, not reused.

**R12 — Operations (feasibility 2, 4, 6, 7).** The scorer workspace lists only suite
runners + `tools/cando2` (never `runner/fuzz`, which declares its own workspace);
`build-ninja/` is only an inherited directory name (no ninja/cmake/nix needed); hand-
offs are answered in BATCHES (one subagent answers a fixed batch of request files for
one stage); driver-stage and migrate-stage batches use different subagents and, where
practical, different model tiers.

---

The reviewed draft follows (superseded where §R says so).

## 0. What M4 must deliver (briefing §5, §8.4; kickoff)

1. The TRACTOR public corpus under `targets/`, pinned and checksummed (§11.3).
2. The executor stage the corpus forces: **LLM driver/test generation** (§3.5 step 1),
   self-validated C-vs-C before any Rust exists. Today `migrate` refuses units with no
   driver; every TRACTOR case has none.
3. A recorded score on the corpus, and that score as the **regression suite** for
   harness changes.

## 1. Spike facts this design rests on (verified 2026-09-23, sources in DECISIONS.md)

- Corpus: `github.com/DARPA-TRACTOR-Program/PUBLIC-Test-Corpus`, MIT, "Distribution A".
  Tags `v1` (6ec7ae6, first evaluation) and `v2` (37960ee08a7c…, 2026-09-14, second
  evaluation). No checksum manifest upstream; integrity = git object ids.
- 346 case dirs; 230 `*_lib` (80 of them P01 SPHINCS+ sub-cases with no own C);
  **150 non-P01 library cases**, each `test_case/{CMakeLists.txt, src/*.c,
  include/*.h}` + `test_vectors/*.json` + `runner/` (a per-case Rust crate using the
  corpus's `tools/cando2`). B01 has 80 lib cases (38 organic, 42 synthetic), all
  single-`.c`.
- Scoring: `cando2` runner, ONE vector per invocation:
  `runner --log-level none --test-root-dir <case> --output <out.json> -v <vector> [--rust] lib`.
  It `dlopen`s `<case>/build-ninja/lib<case>.dylib` (C) or
  `<case>/translated_rust/target/release/lib<case>.dylib` (Rust), calls ONE symbol
  (default: case name minus `_lib`), serializes the post-call state and compares with
  `lib_state_out` (+ optional stdout/stderr patterns). Output: `{"<ResultType>": …}` with
  ResultType ∈ Pass | VectorComparisonFailed | Panic | SegmentationFault | Timeout |
  Skip (vector has `has_ub`) | NoCompare | UnknownFailure.
- Upstream has no `Cargo.lock`; `cando2` pulls ~20 crates.io deps (libloading, nix,
  clap, serde_json/arbitrary_precision, regex, …). Toolchain pinned 1.94.1 (= ours).
- Driver-generation evidence: determinism re-runs, sanitizers, and **mutation
  adequacy** are supported; line coverage is a poor quality gate (advisory only);
  same-model generation of tests and code risks correlated blind spots → use a
  different model/pass for driver vs translation, keep a held-out set.

## 2. The measurement design (the core idea)

Three independent layers, each unable to see the next:

| Layer | Written by | Sees | Role |
|---|---|---|---|
| Generated **driver** | model A (driver stage) | unit C source + headers + ABI | pins current C behavior; the oracle's test |
| Rust **translation** | model B (migrate stage) | unit C source + ABI + oracle evidence | the candidate |
| TRACTOR **public vectors** | MIT LL (humans) | — | **held-out score**; never in any prompt |

The oracle (driver differential + symbol-set + sanitizers) decides `verified`, exactly as
at M3. The corpus vectors never gate anything and never reach a prompt: they MEASURE
how good "oracle-green" is. A verified unit that fails public vectors is the most
valuable finding M4 can produce — it quantifies the oracle's blind spots (driver
weakness), which is the product (§2.1).

Consequences:
- Score = held-out vector pass rate of oracle-verified Rust, over a FIXED denominator
  (every case in the suite; unattempted/blocked = 0).
- The driver sees the C body (deviation from the spike's "header-only" advice, on
  purpose): that evidence is about models writing *expected values* that encode the
  implementation's bugs. Here the model writes only *inputs*; expected output is always
  the real C's stdout. Seeing the body lets it target branches, which mutation adequacy
  then measures. Independence from the translator is by separate model/pass, not by
  hiding source.

## 3. Corpus under `targets/tractor/` (pinned, checksummed)

```
targets/tractor/
  suite.toml          # harness-authored: pin, suite membership, exclusions + reasons
  corpus.lock         # harness-generated: per-file blake3 of every vendored upstream file
  corpus/             # byte-identical subset of upstream at the pinned commit
    LICENSE README.md PUBLIC_VECTORS_UB.md
    tools/cando2/**   # scorer library (src, Cargo.toml, procspawn/)
    Public-Tests/B01_organic/<case>/{test_case/**,test_vectors/**,runner/Cargo.toml,runner/src/**}
    Cargo.toml        # LOCAL (harness-authored): scorer workspace = cando2 + suite runners
    Cargo.lock        # LOCAL: resolved once, committed — the scorer's dependency pin
  cases/ → no: each case dir IS a harness target (harness.toml + migration/ are LOCAL files)
```

- **Acquisition** (dev-time, once, documented in `targets/tractor/README.md`): sparse
  `git clone --filter=blob:none` of the pinned commit, `git rev-parse HEAD` must equal the
  pin, copy the suite's paths. No downloader code in the harness (lean core).
- **`corpus.lock`** (canonical, sorted): header `{schema: ruharness-corpus-lock, v1,
  upstream, tag, commit}`, then one line per upstream file `path  blake3:…`, then the
  `local` path patterns the harness owns (`corpus/Cargo.toml`, `corpus/Cargo.lock`,
  `**/harness.toml`, `**/migration/**`, `**/AGENTS.md`). `harness bench verify-corpus`
  (and implicitly every `bench` command) refuses on: a hash mismatch, a missing file, or
  any file under `corpus/` that is neither locked nor matches a local pattern.
- Scorer dependencies: `cargo fetch` once (network, outside the sandbox — the ONLY
  networked step, recorded); every scorer build afterwards is `--offline --locked`
  inside the sandbox.
- **Suite v1** = the 80 B01 library cases (`B01_organic` + `B01_synthetic`, all
  single-`.c`). Denominator fixed at 80 cases / their vector count. B02/P00–P02/bin
  cases are listed as out of scope with reasons (multi-file units, executables, SPHINCS
  shared sources) — not silently dropped.

## 4. Per-case harness targets (config generalizations)

Each case dir gets `harness.toml`:
```toml
schema_version = 1
[target]
name = "rev16_lib"
source_dir = "test_case"
include_dirs = ["test_case/include"]      # NEW, optional
[oracle]
allowlist = ["cc", "cargo", "rustc", "nm"]
[llm] …
```
Generalizations (all additive/optional):
- `[target] include_dirs`: clean relative paths inside the root. Scanner resolves a
  quoted include against the including file's dir, then each include dir (first hit);
  every oracle `cc` gets `-I` for each. Without this, `lib.h` falls out of the include
  closure → the unit-source digest would not cover it (staleness bug) and prompts would
  lack it.
- `[oracle.whole_program] args = ["-c"]` (NEW): the whole-program check becomes
  **opt-in**. Today it hard-codes zopfli's `-c <sample>` invocation for every target —
  a latent bug that TRACTOR libs (no `main`) expose. zopfli's `harness.toml` gains the
  table in the same commit so its verdict check list is unchanged. (Behavior change for
  a config without the table: recorded as a migration note; zopfli is the only target.)
- Prompt-bound source reads (migrate + driver stage) are confined to `source_dir` ∪
  `include_dirs` — defense in depth for the held-out property (a hostile
  `#include "../test_vectors/1.json"` cannot pull vectors into a prompt).
- `harness bench init` (deterministic, idempotent) writes the case's `harness.toml`,
  runs scan + plan, and sets each unit's `[unit.oracle]` table
  (`kind = "c-abi-differential"`, `driver = "migration/units/<id>/driver.c"`,
  `rust_crate = "<id>_rs"` with `-`→`_`, `replaces = unit.files`) only where no table
  exists.

## 5. Driver-generation stage: `harness gen-driver <UNIT>`

### 5.1 Trajectory
Same engine as `migrate` (refactor: a private `Stage` trait inside harness-llm; the
turn loop, journaling, external/replay/live semantics, `--retry` samples, per-sample
traces, verification-by-replay are shared, not duplicated). Stage differences:

| | migrate | gen-driver |
|---|---|---|
| first turn kind | `translate` | `generate` |
| emission | `src/logic.rs` + `src/ffi.rs` (```rust) | `driver.c` (```c) |
| judge | oracle `verify` on candidate crate | `validate_driver` (§5.3) |
| attempts dir | `units/<id>/attempts/` | `units/<id>/driver-attempts/` |
| attempt id | `a-<12hex>` | `d-<12hex>` over (stage ‖ unit ‖ unit_source ‖ provider_kind ‖ model ‖ request_key) |
| routing | `[llm.migrate]` | `[llm.driver]` (same keys, same clamps) |

`AttemptRecord` gains optional `stage` (`"driver"`; absent = migrate — existing records
byte-identical). `driver` field is `""` in driver attempts. Outcomes and turn results
reuse the closed enums (`build` = driver does not compile, `check` = a validation check
failed, `crash-timeout`, …).

### 5.2 Prompt (trusted/untrusted split as at M3)
System prompt: role; the driver contract; zero-authority policy. User: `[ABI CONTRACT]`
(unit symbols + signatures from facts), `[C SOURCE]` (unit files + include closure,
nonce-delimited JSON strings), `[DRIVER CONTRACT]`:
- `int main(void)`; includes only the unit's headers and `<stdio.h> <stdint.h>
  <stddef.h> <string.h> <stdlib.h> <inttypes.h> <limits.h> <float.h> <math.h>`;
- calls EVERY `[ABI CONTRACT]` symbol, many times, with deterministic inputs (fixed
  vectors and/or an in-file fixed-seed PRNG); prints every return value and every
  output buffer/struct field it can observe, in hex for floats (`%a`);
- never prints pointer values, never reads argv/stdin/env/files/clock, never calls
  `rand`/`time`; no input may violate a documented precondition or trigger UB (the
  harness runs it under ASan/UBSan);
- output ≤ 256 KiB; exit status 0.
Repair turns: current `driver.c`, failure class, evidence (bounded, `| `-quoted), history.

### 5.3 C-vs-C self-validation (`harness-oracle::validate_driver`, deterministic, sandboxed)
All against the ORIGINAL C only (no Rust exists). Checks, in order (stop at first
failure of 1–2):
1. `driver-build` — `cc -O2 -Wall -Werror=implicit-function-declaration
   -Werror=int-conversion -Werror=incompatible-pointer-types -Werror=format
   -Werror=return-type -Werror=uninitialized` driver + unit files.
2. `symbols-called` — `nm -u` of the driver object ⊇ unit `symbols`.
3. `determinism` — 3 runs, byte-identical stdout, exit 0, 1 ≤ bytes ≤ 256 KiB.
   *(Superseded post-M4: stdout AND stderr are compared everywhere — docs/SCHEMAS.md.)*
4. `opt-levels` — `-O0` build output == `-O2` output (UB smell).
5. `sanitizers` — ASan+UBSan build, clean run (same flags as the oracle).
6. `mutation` — see 5.4.
Result: `DriverValidation` record (below). Green iff all pass.

### 5.4 Mutation adequacy
- Mutant generation is a pure function in harness-scan (tree-sitter): sites inside
  function bodies of the unit's `.c` files only; operators: arithmetic `+ - * / %`
  swaps, relational `< <= > >= == !=` swaps, logical `&& ||` swap, bitwise `& | ^`
  swaps, shift `<< >>` swap, integer-literal `n → n+1` (decimal, non-zero), unary `!`
  deletion. Each mutant = (file, byte range, replacement, operator, line, enclosing
  function). Ordered by blake3(file ‖ start ‖ operator ‖ replacement); the first
  `max_mutants` (default 24, clamp ≤ 64) are sampled — deterministic, no RNG.
- Each mutant: the mutated file is written to the build dir, compiled with `-I` the
  original file's dir first, then include dirs; linked with the driver; run with a short
  timeout (10 s). Compile failure → discarded. Killed = stdout differs from the pinned
  C output, non-zero exit, crash, or timeout. Survivor = identical stdout.
- Gate (judgment call, recorded): kill ratio ≥ `min_kill_ratio` (default 0.6) over
  compiled mutants AND every unit symbol whose body has ≥1 compiled mutant has ≥1 kill.
  Zero sites → check passes with detail "no mutation sites". Sites exist but zero
  mutants compile → HARNESS error (never fed to a model: §16.2 deterministic failure).
- Survivors are repair evidence: `line N in fn: '<' -> '<='` (harness-generated text;
  no source bytes quoted).
- Config: `[driver] max_mutants`, `min_kill_ratio` in harness.toml (target-owned →
  clamped; the effective values are recorded in the validation record).

### 5.5 Records and promotion
- `units/<id>/driver-attempts/<d-id>/{attempt.json, candidate/driver.c,
  validation.json}`.
- On green: `units/<id>/driver.c` + `units/<id>/driver-validation.json`
  (`ruharness-driver-validation` v1: unit, `unit_source`, `driver` digest, toolchain,
  thresholds, checks, mutation stats + survivor list; no timestamps). Refuses to
  overwrite an existing `driver.c` whose digest differs unless `--promote`; never
  overwrites a driver that has no validation record (a human-written driver, e.g. u001)
  even with `--promote`.
- `migrate` precondition: if `driver-validation.json` exists it must be green and its
  `driver`/`unit_source` digests must match the tree (stale → refuse). A driver with no
  record = human-authored, allowed as at M3.
- `harness state status` shows driver validation freshness per unit.

## 6. Held-out scoring: `harness bench score` / `bench check`

Per case (sandboxed, offline):
1. Verify corpus lock.
2. Build the C baseline dylib: `cc -shared -fPIC -O0 -I… test_case/src/*.c` →
   `<scratch>/build-ninja/lib<case>.dylib` (O0 = CMake's default for an empty build type).
3. If the case's unit(s) are `verified`: build the promoted crate's staticlib (the same
   build the oracle does) and wrap it: `cc -shared -Wl,-force_load,<lib.a>` →
   `<scratch>/translated_rust/target/release/lib<case>.dylib`.
4. Build the case runner once per suite: `cargo build --release --offline --locked -p
   <runner-pkg>` in `corpus/` with the target dir in the gitignored build dir.
5. For every vector: run the runner (C side, then Rust side) with the case's
   `test_vectors` visible to it via `--test-root-dir <scratch>` (scratch holds copies/
   links of `test_vectors/` and the two dylib dirs). Timeout per vector (30 s).
6. Classify per vector: `pass | fail(<ResultType>) | skip(has_ub) | not-run`.

Scores (`targets/tractor/scores.json`, canonical, no timestamps, committed):
```json
{"schema":"ruharness-bench-scores","schema_version":1,
 "suite":"tractor-b01-lib","corpus_lock":"blake3:…","toolchain":[…],
 "totals":{"cases":80,"cases_passed":N,"vectors":V,"vectors_passed":P,"vectors_skipped":S,
           "c_baseline_invalid":K},
 "cases":[{"case":"B01_organic/rev16_lib","unit":"u-lib","status":"verified",
           "driver":"validated","rust_crate":"blake3:…",
           "c_baseline":{"pass":5,"fail":0,"skip":0},
           "rust":{"pass":5,"fail":0,"skip":0,"not_run":0},
           "vectors":[{"name":"1.json","c":"pass","rust":"pass"}, …]}]}
```
- A case with `c_baseline.fail > 0` is `c-baseline-invalid` on this platform: reported,
  excluded from `cases_passed` numerator AND called out, never hidden.
- `bench check` recomputes everything and compares with the committed file:
  any vector that was `pass` and is not now → **regression, exit 10**; improvements are
  reported and exit 0 ("run `bench score --write` to record"). This is the regression
  suite: it needs zero LLM tokens (§16.2), and re-running `harness verify` on every
  verified unit is part of it (a verified unit whose oracle goes red is a regression).

## 7. Who answers the model calls in M4

No cloud keys; the local 1B model cannot hold the emission contract (M3). M4 uses the
`external` hand-off answered by Claude subagents — with two M3 weaknesses fixed:
- `--model` is set to the ACTUAL answering model id (not a placeholder string);
- driver stage and migrate stage are answered by DIFFERENT subagents (different model
  tiers where practical), each told to read only the request file(s) and write only the
  response file(s). Blindness is by instruction, not enforced by a sandbox — recorded as
  a limitation. Usage stays `null` (unmeasured).
Offer the user: a capable local coder model (download needs consent) or an API key
would make runs live and measured.

## 8. Crate impact (keeps the core small)

- harness-core: config (`include_dirs`, `[llm.driver]`, `[driver]`), attempts
  (`stage`, driver id/dir), `DriverValidation`, `Mutant`, bench types (`CorpusLock`,
  `Scores`). No new deps.
- harness-scan: include-dir resolution; `mutants()`.
- harness-oracle: `-I` include dirs; opt-in whole-program; `validate_driver`; bench
  builds + runner execution + result parsing. No new deps (JSON via core's serde_json).
- harness-llm: `Stage` refactor; driver stage; emission generalized to a file-spec.
- harness-cli: `gen-driver`, `bench {init, verify-corpus, score, check}`, status lines.
- No new crates.io dependencies in the harness. The scorer's deps are the CORPUS's,
  locked in `corpus/Cargo.lock`, built offline in the sandbox.

## 9. Explicitly out of scope for M4 (recorded, with revisit triggers)
Coverage gate (advisory only; revisit if mutation gate proves too slow); executables
(`bin` cases) — needs a whole-binary unit kind; multi-`.c` cases; P01/P02; the official
Nix/Docker/Falco runner (we reuse its cando2 scorer, not its container orchestration);
idiomaticity/safety/perf scoring (the corpus's `rust_eval` tools); escalation tiers and
budget enforcement (§16 carry-forward).
