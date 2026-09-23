# Oracle hardening after M4 — designs

Status: **A implemented** (design review 3 lenses; code review 2 lenses; §A.R + §A.2 +
§A.3 authoritative; normative contract in docs/SCHEMAS.md). B follows A.
Spikes and the experiment behind A: DECISIONS.md, "Research spikes (§15) for the two
remaining blind-spot classes".

## A. Unmarked-UB vectors: a targeted sanitized-C pass in scoring

### Problem
A held-out vector not marked `has_ub` can still drive the C into undefined behavior
(`decorrelate`: a write one past a 5-element array through a decayed pointer). The
uninstrumented `-O0` baseline "passes" by luck; a correct Rust fails; the case counts
as a **blind spot** — a false finding against the oracle. The benchmark must not
charge the harness for the corpus's UB, and must disclose every vector it excuses.

### Rule (normative once approved)
A non-`has_ub` vector *v* of a case is **`unmarked-ub`** iff, in the SANITIZED C pass,
*v*'s run ended abnormally AND its captured stderr carries a sanitizer report header
(`ERROR: AddressSanitizer:` or `runtime error:` from UBSan). The pass runs *v* only
when *v* matters: the plain C side did not pass, or the verified Rust / scored
candidate did not pass. A vector every side passes is never re-run (its UB-ness cannot
change any class). `unmarked-ub` vectors are excluded from a case's non-UB set exactly
like `has_ub` (so from `scorable`, `vectors`, strict pass), and counted and listed
separately — never silently dropped.

### Mechanism (experimentally verified, see DECISIONS.md)
1. **Sanitized runner.** `Scorer::prepare` builds the corpus's scorer workspace a
   second time into `scorer-target-<lock>-asan/`, identical except
   `--config build.rustflags=[...]` adding `-C link-arg=<asan runtime dylib>` and an
   rpath to its dir. The runtime path comes from `cc -print-file-name=
   libclang_rt.asan_osx_dynamic.dylib` (Linux: the static runtime is linked by
   `-fsanitize=address` into the dylib's host differently — out of scope until the
   Linux sandbox; the pass is skipped with a recorded reason where unsupported).
   No vendored file changes; the runtime is loaded at startup as a link dependency,
   so no `DYLD_*` variable is needed (SIP's `sandbox-exec` would purge it).
2. **Sanitized C dylib.** Same sources/includes as the baseline, flags
   `-shared -fPIC -O0 -ffp-contract=off -fsanitize=address,undefined
   -fno-sanitize-recover=all -g` (the oracle's sanitizer set at the baseline's -O0).
3. **Run.** Same confinement as every vector run (fresh run root, run profile; the
   profile must allow reading the runtime dylib — verified in the implementation's
   integration test). `RunStatus` gains captured stderr (bounded, as elsewhere);
   the sanitizer kind is taken from the `SUMMARY: <Sanitizer>: <kind>` line, reduced to
   `[a-z0-9-]`, ≤ 48 bytes (`address:stack-buffer-overflow`, `undefined:…`), else
   `unknown`.

### Schema (docs/SCHEMAS.md, additive)
- `VectorScore.c_sanitized: Option<String>` — absent = not run; `"clean"` | `"ub:<kind>"`
  | `"fail:<…>"` (abnormal without a report: NOT excused) | infra strings.
- `SplitTotals.vectors_unmarked_ub: u32` (`#[serde(default)]`).
- `Scores.environment` gains `sanitized-pass: asan+ubsan` | `sanitized-pass: skipped
  (<reason>)` — a baseline recorded without the pass is incomparable with one with it
  (R10), forcing a deliberate re-baseline rather than a silent denominator change.
- `schema_version` stays 1 (additive, defaulted); older readers ignore the fields.

### Classification change
`classify_case`: non-UB = `c != "skip"` AND `c_sanitized` is not `ub:*`. Everything
else unchanged. Expected effect on the recorded baseline: `decorrelate` → its failing
vectors become `unmarked-ub`; if no non-UB vector remains it is `unscorable`, else
re-classified on the rest. Reported per case in `bench score` output
(`unmarked-ub: 2 (address:stack-buffer-overflow)`).

### Threat model notes
- The sanitized runner and dylib run confined exactly like the plain ones.
- A hostile C baseline could print a fake report and crash to excuse a vector; the C
  baseline is the reference by construction (a hostile one can fail its own vectors
  just as well). Both conditions (abnormal end AND header) are required; the kind is
  recorded for audit.
- Sanitizers on the *C* side only; the Rust is never excused by this pass.

### Not doing
Per-vector generated C mains (re-implements cando's marshalling); dlopen + DYLD
injection (SIP); trap-only UBSan (misses decayed-pointer overflow); running the pass on
all vectors (cost without effect on any class).

### Revisit when
A runner cannot be relinked (e.g. a static-only toolchain), the Linux sandbox lands
(runtime selection differs), or sanitizer reports prove noisy on synthetic
intentionally-UB cases (then restrict excusal to the ASan kinds).

### A.R — Adversarial design review (3 lenses) and resolutions (authoritative)

Verdicts: measurement validity **sound-with-fixes**, security **sound-with-fixes**,
feasibility/contracts **sound-with-fixes**. Where this section and the text above
disagree, this section wins.

- **R-A1 Trigger (measurement).** The pass runs a vector only when the plain C
  **passed** it and the verified Rust or the scored candidate did not (and not on an
  infra result). A vector the plain C fails is never excused: a
  `c-baseline-invalid` case can never be laundered into a scorable one.
- **R-A2 ASan only, allow-listed kinds (measurement, security).** The sanitized dylib
  is built with `-fsanitize=address` only (UBSan fires on intentional two's-complement
  idioms that correct Rust must reproduce, and with `-fno-sanitize-recover` it could
  abort before the memory error). Excusal requires a kind in a closed allow-list of
  memory-access errors: `stack-buffer-overflow stack-buffer-underflow
  heap-buffer-overflow global-buffer-overflow dynamic-stack-buffer-overflow
  heap-use-after-free stack-use-after-return stack-use-after-scope use-after-poison`.
  Any other or unparseable kind is recorded, never excused.
- **R-A3 Signal (feasibility, security, measurement).** cando runs each vector in a
  re-exec'd child whose stdout/stderr it captures and embeds in its report
  (`output.stderr`); the harness's own capture of the runner would see nothing. The
  signal is therefore read from the report: cando's result is a failure type other
  than `Timeout`, AND its `output.stderr` (bounded) contains a line
  `==<pid>==ERROR: AddressSanitizer: <kind> ` and a line `SUMMARY: AddressSanitizer:
  <kind>` with the same allow-listed kind. A timeout, a killed runner, a non-0/1
  runner exit, or a missing report is never excused. Residual risk (recorded): a
  hostile reference C can print a fake report and abort; the kind and the C's plain
  result are recorded per vector for audit.
- **R-A4 Tallies (feasibility).** `Counts` gains `unmarked_ub` (`#[serde(default)]`);
  an excused vector moves from pass/fail into `unmarked_ub` on EVERY side, so
  `SplitTotals.vectors`/`vectors_passed` and the class agree. `SplitTotals` gains
  `vectors_unmarked_ub`.
- **R-A5 Regression semantics (measurement).** `compare()` compares `c_sanitized` per
  vector; an excused vector that stops being excused while the Rust does not pass is a
  regression (exit 10); every other change of `c_sanitized` is listed.
- **R-A6 Runtime path hygiene (security, feasibility).** `cc -print-file-name=
  libclang_rt.asan_osx_dynamic.dylib` echoes the bare name when not found: the result
  must be absolute, exist, canonicalize, lie outside the home dir and the suite, and
  pass a strict character allow-list (`[A-Za-z0-9._/+-]`) before it is spliced into
  cargo config; otherwise the pass is skipped with a closed reason. `cc` is resolved
  exactly as for every other oracle compile (allowlisted name) — the existing posture.
- **R-A7 Link-arg scope (security).** Not `build.rustflags` (it would also link build
  scripts and proc-macros): the runner build passes `--target <host triple>` (from
  `rustc -vV`) and `--config target.<triple>.rustflags=[…]`, which applies to target
  artifacts only. Runners are then under `<target-dir>/<triple>/release/`.
- **R-A8 Profile (security).** Run profiles are deny-listed (home, suite); the Xcode
  toolchain is already readable — no widening unless the integration test proves it
  needed, and then a single `literal` read of the canonical runtime path.
- **R-A9 ASAN_OPTIONS (security).** Harness-set via explicit env, never inherited:
  `detect_leaks=0:abort_on_error=1:halt_on_error=1:symbolize=0:print_summary=1` — no
  `log_path` (reports stay on the captured, capped stream); `symbolize=0` because the
  run profile denies exec of `atos`.
- **R-A10 Environment entry (feasibility).** Closed set: `sanitized-pass: asan` |
  `sanitized-pass: skipped (runtime-not-found)` | `sanitized-pass: skipped
  (unsupported-platform)`.
- **R-A11 Sanitized-side infra (feasibility).** A sanitized dylib that does not build,
  or a sanitized run with an infra result, is `c_sanitized = "infra:<…>"` and a
  `bench score` PROBLEM line — never silent, never excused.
- **R-A12 Disclosure (measurement).** Each excused vector is printed with its kind and
  the paired Rust (and candidate) result; SCHEMAS.md is updated with the code.

### A.2 — Revision after the first end-to-end run (supersedes the mechanism where it conflicts)

**Finding.** The implemented ASan pass ran end to end and excused NOTHING: `decorrelate`'s
two vectors pass cleanly under ASan. The spike's premise (a C-owned stack struct) is
false for cando: the RUNNER (uninstrumented Rust) owns every vector's state; the C gets
`&raw mut self.tflac`, and `residuals[5]` lands inside the runner's own state object —
memory ASan never poisoned. ASan still covers C-internal objects (C locals, C heap,
globals), but not the dominant layout of this corpus.

**Mechanism added: Apple clang `-fbounds-safety`.** Local pointers are bounds-carrying by
default, so `residuals_0 = t->residuals; residuals_0[5] = …` traps (brk → SIGTRAP)
whatever owns the memory — no runtime library, no runner change. Measured on this
machine: both decorrelate vectors → cando `UnknownFailure`, `wait_status` 5, under the
PLAIN runner. It does not compile unannotated code that does arithmetic on parameter or
struct-member pointers (`read_scalefactors`: 4 errors) — for such code the pass falls
back to ASan only, and that C stays clean (its vectors, a genuine Rust bug, are NOT
excused — the case the rule must never excuse).

- **R-A13 Build.** Sanitized dylib: first `-fbounds-safety -fsanitize=address` (both
  combine); if that does not compile, `-fsanitize=address` alone. Variant recorded.
- **R-A14 Signal.** Under the bounds-safety variant, cando `UnknownFailure` with a raw
  wait status whose termination signal is SIGTRAP (5), on a vector the plain C PASSED,
  is `ub:bounds-safety-trap`: the only new trap source between the plain and the
  instrumented build is an inserted pointer-validity check. (Linux traps with SIGILL;
  the pass is macOS-only until the Linux sandbox.) ASan reports keep R-A2/R-A3.
- **Residual false-positive risk (recorded).** `-fbounds-safety` also checks when a
  bounds-carrying pointer is converted to a plain one (e.g. passing a one-past-the-end
  pointer to a function) — legal C that may trap. Such a trap excuses the vector; it is
  disclosed with kind `bounds-safety-trap` for audit. Revisit if an audit finds one.

### A.3 — Code review (2 lenses) and fix pass

Security **no defect** (runtime-path allow-list applied to what is spliced; triple
validated; `--target` scoping; `ASAN_OPTIONS` explicit on a cleared env; identical run
confinement; forged single-line reports rejected). Correctness/measurement: (1) a lost
excusal on an UNVERIFIED case (Rust `not-run`) was reported as a Rust regression —
fixed (a measured Rust result only), regression test; (2) the bounds-safety fallback
was silent — each case now records `sanitized_build` and a fallback is printed (a flaky
fallback that loses an excusal is additionally a `bench check` regression by R-A5).
Contracts: **R-A10 amended** — the environment entry is `sanitized-pass:
asan+bounds-safety` (not `asan`); **R-A11 amended** — sanitized infra results reuse the
existing infra strings (`fail:dylib-build`, `fail:no-report`, …) rather than an
`infra:` prefix, recognized by `is_infra_result`; an unusable `rustc -vV` degrades to
`skipped (unsupported-platform)` instead of aborting scoring.

## B. FFI-boundary blind spots — DRAFT proposal (not yet reviewed; next session)

### Problem (from the verified `read_scalefactors` shim)
The Rust `ffi.rs` eagerly builds `slice::from_raw_parts(buf, (limit+7)/8)` and
`from_raw_parts_mut(scf, 3*bands)`: lengths INFERRED from fields, over memory the C
contract never promised. The C touches only what a call path consumes. A slice over
unallocated memory is UB in Rust even if never read; on held-out inputs it segfaults
(3/6). The LLM driver's buffers were always larger than any inferred length, so the
differential driver could not see it.

### What cannot work here (checked)
Miri and `-Zsanitizer` need nightly; libFuzzer runtime absent from Apple clang;
ASan-instrumenting the C driver does not check reads made by uninstrumented Rust
(only intercepted libc calls such as `memcpy`); Rust's stable `ub_checks` do not check
allocation extent; rewriting driver buffers with tree-sitter is fragile.

### Proposal B1 — a guarded "boundary driver" as a NEW stage (no replay invalidation)
1. **Harness-owned header `ruharness_guard.h`** (vendored into every build, not
   model-written): `void *rh_buf(size_t n)` returns `n` bytes placed flush against a
   `PROT_NONE` page (mmap + mprotect; `rh_buf_under` for the underflow side), and
   `rh_free`. Any access one byte past the requested extent faults — on either side.
2. **Boundary driver** = a second driver per unit, generated by the existing driver
   pipeline under NEW stage texts (a new stage has its own request keys, so every
   recorded driver/migrate attempt still replays): "exercise every pointer/length
   relation at 0, 1, N−1, N; allocate EVERY buffer passed to the unit with `rh_buf` of
   exactly the size the C contract requires". The prompt lists the pointer parameters
   and pointer-typed struct fields (tree-sitter, deterministic).
3. **Validation (C only)**: strict build; determinism; the C runs CLEAN with the guards
   and under ASan — the in-contract filter (an input on which the C itself faults is
   not a contract the Rust must meet; cf. RustAssure's C-defined-behavior filter);
   lint: `rh_buf` referenced, no stack arrays passed to unit symbols (tree-sitter).
   No mutation gate (its job is extent, not behavior).
4. **Oracle check `boundary-driver`** (after `differential-driver`): C-linked vs
   Rust-linked, both streams; a Rust-side fault on a guard page = red, with evidence
   naming the faulting call index (driver prints a marker per call). Absent boundary
   driver → check passes with detail `not generated` (opt-in per target, like
   whole-program), so existing verdicts are unchanged until a target opts in.

### Open questions for the design review
Allocation discipline enforcement (lint strength vs false rejects); units whose
contract is "caller-sized buffer, callee reads a length field" (the extent the C
REQUIRES must come from the C, not the model — how is N chosen?); per-call fault
attribution under `panic=abort`; runtime cost; whether a prompt hint to the TRANSLATOR
("never materialize a slice longer than the C provably accesses") is also warranted —
that one IS a prompt change (see the prompt-versioning question in DECISIONS).
