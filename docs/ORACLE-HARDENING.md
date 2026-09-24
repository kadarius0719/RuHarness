# Oracle hardening after M4 — designs

Status: **A implemented** (design review 3 lenses; code review 2 lenses; §A.R + §A.2 +
§A.3 authoritative; normative contract in docs/SCHEMAS.md). **B designed and adversarially
reviewed** (§B, 2026-09-23; §B.R is AUTHORITATIVE where it amends §B; B.R-2's mechanism choice
awaits the user's decision — DECISIONS.md).
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

## B. FFI-boundary blind spots: the measured-footprint boundary check

Status: **designed 2026-09-23, not yet reviewed.** Replaces the unreviewed DRAFT (guard
pages sized by the model). What changed and why is in §B.1. Spike reports and the
end-to-end prototype are summarized in DECISIONS.md ("Design B — research re-check").

### B.0 Problem

The verified `read_scalefactors` Rust passes every oracle check and fails 3/6 held-out
vectors with a segfault. Its `ffi.rs` converts every pointer eagerly. It dereferences
`bs` (`&mut *bs`), builds `from_raw_parts(scfcod, bands)` and reads `scfcod[i]` for every
band. The C reads `bs` only when some band is allocated (`ba != 0`), and `scfcod[i]` only
for such bands. Callers that pass buffers exactly as large as the C needs (or NULL where
the C never looks) crash the Rust. The LLM-written differential driver passed generous
stack buffers on every call, so no check could see the difference. The class is general:
published C→Rust shims derive slice lengths from fields or use "a conservative fixed
bound" (SACTOR, ENCRUST; DECISIONS.md), and no published oracle measures what the source
touches.

### B.1 What the spike established (all verified on this machine)

1. **The C's footprint can be measured exactly, for free.** Apple clang 21:
   `-fsanitize-coverage=edge,trace-loads,trace-stores` calls a user-supplied
   `__sanitizer_cov_{load,store}{1,2,4,8,16}(addr)` before every scalar load and store,
   with no runtime library. (`edge` is required: without it the flags are accepted and
   silently instrument nothing.) At `-O0` every stack spill is traced too, so a callback
   must filter by address.
2. **Not everything is traced.** Aggregate copies (struct assignment, pass/return by value,
   `= {0}`) lower to `llvm.memcpy`, and `mem*`/`str*` calls (fortified to `__*_chk` at
   `-O0`), read-modify-write atomics and >16-byte vector accesses are untraced.
   Measurement alone under-counts, so it needs a ground-truth check (B.4 phase L).
3. **Guard pages are exact.** On macOS arm64 (16 KiB pages) a `PROT_NONE` access raises
   SIGBUS (Linux: SIGSEGV), and `si_addr` is the first faulting byte, also for straddling
   and 16-byte accesses. mmap, mprotect, sigaction and sigaltstack all work under the run
   sandbox profile.
4. **Prototype, end to end, on the real case.** The existing validated driver was
   converted to allocate every unit argument through a copy-in guard API. The harness
   measured the C's per-allocation windows and re-ran with each allocation shrunk to its
   window. Result: the tightened C runs were clean and byte-identical to the measurement
   run in both layouts (window flush against the following page, or against the preceding
   page). The **verified Rust faulted** on `scfcod` in case 0 (the C touches 0 of 5
   elements). A Rust with one change (read `scfcod[i]` only when `ba != 0`) passed with
   identical output. The same results hold under a run-like `sandbox-exec` profile.
5. **One more requirement follows from the prototype.** The driver's own read-back after a
   call (it prints `bs->pos`) made `bs` "touched" in every case. That masks the other
   eager dereference (`&mut *bs` when the C never reads `bs`, the likely held-out crash).
   Windows must therefore be measured and enforced **per unit call**, not over the whole run.
6. **Nothing else works on stable** (re-checked): no Miri or `-Zsanitizer` on stable
   1.94–1.98; `ub_checks` in `from_raw_parts` checks alignment, null and size only; there
   is no libFuzzer runtime in Apple clang. Stable rustc can trace loads
   (`-C passes=sancov-module`), but it misses memcpy and depends on LLVM-internal flags, so
   it is not used; the Rust side is judged by guard pages only.

### B.2 Rule (normative once approved)

For a unit with a **boundary driver**, and for every call the driver makes to a unit
symbol: during that call, the Rust may touch (load or store) only allocations the C
touches during the same call, and within each allocation only elements inside the C's
window for it. The window is the union, over the whole run, of the elements the C touches
in that allocation during unit calls. Everything else about the run must also match the C
(both streams and the exit status), in both layouts.

- **"Allocation"** means one call of the harness copy-in API (B.3) by the boundary driver.
- **"Element"** means `elem_size` bytes as the driver declares them (the pointee's
  `sizeof`). Granularity is per element, so reading a whole struct is never a violation
  once the C reads any byte of it.
- **Accesses by the driver outside unit calls never count**, and are always allowed.
- **A Rust access outside the rule faults and the check is red.** Nothing is excused.
- **Scope.** Allocations made any other way (driver stack, statics, malloc), the unit's own
  stack, heap and globals, and slices *created* but never accessed are outside the rule.
  They are disclosed limits (B.9).

### B.3 The guard runtime (harness-owned C, `include_str!`)

`ruharness_guard.h` (the driver includes it) declares one function and one macro:

```c
void *rh_in_at(int line, const void *src, size_t count, size_t elem_size);
#define rh_in(src, count, elem_size) rh_in_at(__LINE__, (src), (count), (elem_size))
```

- **What it returns:** a pointer to `count` elements of `elem_size` bytes, initialized from
  `src` (zeros when `src` is NULL), valid until the process exits. It is never freed and
  never NULL, even when `count` is 0.
- **Limits:** at most 4096 allocations per run; each at most 16 MiB; at most 256 MiB in
  total. A violation prints `RH-ERROR <reason>` on stderr and calls `_exit(96)`.
- **Layout:** each allocation is its own reservation of `[guard page][elements][guard
  page]`, placed so that either the window end (tail layout) or the window start (head
  layout) falls exactly on a page boundary. Placement keeps `elem_size` alignment, because
  a page-aligned address plus or minus a multiple of `elem_size` stays aligned (B.1-4).

`ruharness_guard.c`, the runtime, is compiled without instrumentation and linked into every
boundary build. Its mode comes from harness-set environment variables. (The confined run's
environment is cleared; neither the driver nor a Rust candidate may read the environment,
because of driver-shape and capabilities.)

| `RUHARNESS_GUARD` | Used by | Behavior |
|---|---|---|
| unset / `full` | validation runs | every allocation fully accessible; tail-flush at `count` |
| `measure` | phase M | `full` + trace callbacks, counted only inside unit calls; writes `$TMPDIR/ruharness-guard.out` at exit |
| `learn-tail` / `learn-head` | phase L (C only) | windows enforced; an in-call fault is recorded, the allocation opened for the rest of that call, execution continues |
| `tail` / `head` | phases C and R | windows enforced; an in-call fault prints `RH-FAULT …` and `_exit(97)` |

`RUHARNESS_GUARD_WINDOWS` names the window table: a harness-written file, passed as a
listed input.

- **Wrapper.** The harness generates `ruharness_calls.c` from the unit's interface lines
  (B.6). The boundary driver's translation unit is compiled with
  `-D<sym>=ruharness_call_<sym>` for every unit symbol, so every call the driver makes goes
  through a wrapper. The wrapper calls `ruharness_enter(sym)`, classifies each pointer
  argument (`ruharness_arg`: NULL, inside allocation *a* at element *k*, or foreign), calls
  the real symbol, then calls `ruharness_exit()`. The same wrapper object is linked into
  the C-linked and the Rust-linked builds; only what it calls differs. Nothing in the unit's
  C or the Rust crate changes.
- **Enforcement (tight modes).**
  - Outside calls, every allocation the driver touches is opened lazily. The fault handler
    opens the whole allocation and returns, so the access retries.
  - On `enter`, every open allocation is closed (`PROT_NONE`). Then only allocations the C
    touched in this call are opened, each exposing exactly its window.
  - On `exit`, they are closed again. The cost per call is proportional to the allocations
    involved, not to all allocations.
- **Fault handler.** It handles SIGSEGV and SIGBUS with `SA_SIGINFO|SA_ONSTACK` on a static
  alternate stack. What it does depends on where the fault lands:
  - **In a reservation, during a call:** it writes
    `RH-FAULT call=<n> allocation=<a> element=<k> layout=<tail|head>` and `_exit(97)`
    (strict), or records the fault (learn modes).
  - **In a reservation, outside calls:** it opens the allocation and retries. If the fault
    is outside `[0, count)`, it prints `RH-OVERRUN` and `_exit(97)`.
  - **Anywhere else:** it restores the default disposition and returns, so the process dies
    by its own signal, exactly as without the runtime.
  - It prints no addresses, so evidence stays deterministic across ASLR.
- **Diverged allocation sequence.** If, in a tight run, allocation *a*'s `count` or
  `elem_size` differs from the table, the Rust has changed the driver's control flow. The
  runtime prints `RH-DIVERGED allocation=<a>` once on stderr (so the stderr compare is red)
  and gives every later allocation full access.
- **Canary.** A harness-owned `ruharness_probe.c` is compiled with the coverage flags in
  the measure build. In measure mode, `init` stores through it into a private page. If no
  callback fires, the output says `probe 0`, and the harness reports "load/store tracing is
  inactive in this toolchain" as a harness error.

### B.4 The check, phase by phase (in `verify` and in boundary validation)

Builds go into a fresh `build/<unit>/bd/`. They are compiled at `-O0` with
`-ffp-contract=off`. The boundary driver is **copied into `bd/`** next to the runtime
header, and `-I<bd>` comes before every target include. This way a target file named
`ruharness_guard.h` can never shadow the harness header (a quoted include searches the
includer's directory first).

| Build | Driver TU | Unit C | Plus |
|---|---|---|---|
| `bd_measure` | plain, `-D` renames | **instrumented** | wrapper, runtime, instrumented probe |
| `bd_c` | plain, `-D` renames | plain | wrapper, runtime, probe |
| `bd_rs` | plain, `-D` renames | — (staticlib) | wrapper, runtime, probe |

1. **Phase M: measure.** Run `bd_measure` once in `measure` mode. It must exit 0. Its
   output file is parsed strictly (B.5). The canary must be on. Every traced unit access
   must fall inside `[0, count)` of its allocation; otherwise the C itself overruns the
   driver's allocation, and the driver is invalid. The output gives:
   - per allocation: its source line, `count`, `elem_size`, and the window `[lo, hi)`
     (element-rounded union of traced in-call accesses);
   - per call: the set `touched(call)` of allocations the C touched;
   - per call and pointer argument: NULL, allocation and element, or foreign.
2. **Phase L: learn** the C's untraced accesses (C only, at most 4 rounds per layout). Run
   `bd_c` in `learn-tail`, then `learn-head`. For every recorded fault `(call c, allocation
   a, element k)`:
   - add `a` to `touched(c)`;
   - if `k` is outside `a`'s window, widen the window to `[0, count)`;
   - if `k` is outside `[0, count)`, the driver is invalid (the C overruns its allocation).

   Repeat until a learn round records nothing. If that has not happened after 4 rounds,
   the driver is invalid ("the C's untraced accesses do not converge").
3. **Phase C: confirm.** Run `bd_c` strictly in `tail` and in `head`. Both must exit 0, and
   both streams must be byte-identical to phase M. This proves the final windows cover
   every C access, traced or not. The detail records how many allocations were widened.
4. **Phase R: judge** (verify only). Run `bd_rs` strictly in `tail` and in `head`. Each run
   must exit 0 with both streams byte-identical to phase C in the same layout.
   - An `RH-FAULT` line makes the check red. The harness writes the detail from its own
     measurement: which call and symbol, which parameter the allocation was passed as
     (when it was passed directly), the allocation's source line, `count` × `elem_size`,
     the element touched, and the C's window. For example: "the Rust touched element 0 of
     `scfcod` in call 1 of read_scalefactors (boundary-driver.c line 43; 5 × 1 bytes); the
     C touches none of it in that call".
   - An `RH-DIVERGED` line makes the check red: the output differs.
   - Any other crash, exit or output difference makes the check red, with the standard
     wording (`candidate run failed …` / `outputs differ …`).

**Where the check sits in `verify`.** It is named `boundary-driver` and comes last, after
`sanitizers`, and runs **only when every earlier check passed**. Recorded red turns
therefore keep their evidence and their class byte-for-byte (§R-3 of the replay design).

- **No boundary driver** (no `units/<id>/boundary-driver.c`): `passed: true`, detail
  `not configured for this unit`.
- **Boundary driver present, but C-side phases M/L/C fail:** harness error (the driver's
  fault, never candidate evidence). This follows the driver-shape precedent. Verify of such
  a unit stops until the boundary driver is regenerated.
- **Unsupported toolchain** (the probe build does not compile, or the canary is off): also
  a harness error, never a silent pass. Revisit with the Linux sandbox.

**Verdict inputs.**
- `VerdictInputs.boundary_driver` is the digest of `boundary-driver.c`. It is `#[serde(default,
  skip_serializing_if = "String::is_empty")]`, so every verdict without one stays
  byte-identical.
- When the check ran, the toolchain gains the entry `boundary: sancov+guard-pages`.

### B.5 Harness-parsed files (strict, capped)

- **`$TMPDIR/ruharness-guard.out`** (written by the runtime inside the confined run; read
  back by a new `Confinement` method before the temp dir is removed; regular file only, no
  symlink, ≤ 4 MiB). The format is line-based ASCII:
  - `ruharness-guard 1`
  - `probe <0|1>`
  - `alloc <a> <line> <count> <elem> <lo> <hi>`
  - `call <n> <sym>`
  - `arg <n> <param> null|foreign|<a>:<k>`
  - `touch <n> <a>`
  - `end`

  The harness checks every field: ids dense and in order, `lo ≤ hi ≤ count`,
  `count × elem` within the limits, `sym`/`param` within the harness's own interface
  parse. No target text ever appears in the file; names are resolved by the harness.
  Anything malformed is a harness error (the file is written by harness code; a
  hostile unit could scribble on it, which can only produce an error or a weaker check,
  never a false red, because phase C re-proves every window).
- **The window table** (`bd/windows-<layout>.txt`, written by the harness) has the same
  line discipline: `ruharness-windows 1 <n>` then `alloc <a> <count> <elem> <lo> <hi>`
  and `touch <n> <a>`.

### B.6 The wrapper: generated from the interface lines

- **Validation of each interface line (target-derived).** tree-sitter must parse it as
  exactly one declaration whose declarator is a function declarator named exactly the
  plan symbol. It may contain no `{`, `;`, `#` or newline, and every parameter must be
  named and non-variadic. Otherwise `gen-driver --boundary` refuses the unit (unsupported
  signature). This reuses harness-scan's grammar; no new crate.
- **Parameter classification.** A *data pointer* is a parameter whose declarator contains a
  pointer or array declarator, and is not a function pointer. Typedef'd pointer types are
  not recognized; they are passed through and disclosed as unchecked.
- **Generated text:**

  ```c
  #include <the unit's headers, as the unit's .c files include them>
  #include "ruharness_guard_internal.h"
  #define SYM ruharness_call_SYM
  <interface line>
  #undef SYM
  { ruharness_enter(i); ruharness_arg(0, p0); … ; __typeof__(SYM(args)) r = SYM(args);
    ruharness_exit(); return r; }        /* void: no r */
  ```

  Symbol names are checked against `^[A-Za-z_][A-Za-z0-9_]*$` before they reach a `-D`
  argument or the file.

### B.7 The boundary driver and its validation (a new stage)

- **Stage `boundary-driver`.** `harness gen-driver <UNIT> --boundary [--provider] [--model]
  [--promote] [--retry] [--attempt ID]`. The trajectory engine is the same as for
  `gen-driver`. The attempts, traces and ids are all new:
  - attempts go under `units/<id>/boundary-attempts/<b-id>/`, traces under
    `units/<id>/boundary-traces/`;
  - the id is `b-` + 12 hex of blake3(`boundary-driver` ‖ NUL ‖ unit ‖ NUL ‖ unit_source ‖
    NUL ‖ driver ‖ NUL ‖ provider_kind ‖ NUL ‖ model ‖ NUL ‖ first request_key);
  - `stage: "boundary-driver"`, and `driver` is the digest of the unit's validated `driver.c`
    (the attempt is bound to it, because the prompt contains it).

  Existing stages, ids and records are untouched.
- **Preconditions:**
  - the unit has a fresh green driver validation;
  - every interface line passes B.6;
  - at least one data-pointer parameter exists ("nothing to guard").
- **Prompt:** its own system prompt and `[BOUNDARY CONTRACT]`.
  - **Sections:** `[UNIT]`, `[ABI CONTRACT]`, `[POINTER PARAMETERS]` (the harness's B.6
    classification, as nonce-fenced JSON lines), `[C SOURCE]`, `[VALIDATED DRIVER]` (the
    unit's `driver.c` as an untrusted nonce-fenced JSON string: "start from its cases"),
    `[GUARD API]` (the header text).
  - **Rules:**
    - every buffer and struct passed to a unit symbol, or reachable from one through a
      pointer field, comes from `rh_in` with `elem_size` = `sizeof` the pointee;
    - allocate at least what the C contract needs — the harness shrinks every allocation to
      what the C touches;
    - use malloc only for memory the unit frees or reallocates;
    - make fresh allocations for every call;
    - cover calls where the C touches part of an argument or none of it (zero counts, early
      returns, flags that skip optional arguments, short reads);
    - never read argv, the environment or files;
    - print every observation (the driver contract's output rules).
- **Emission.** The file `boundary-driver.c` (a new `FileSpec`); a promoted green candidate
  becomes `units/<id>/boundary-driver.c`.
- **Validation** (`validate_boundary_driver`, C only, `boundary-validation.json`, schema
  `ruharness-boundary-validation` v1). It stops at the first failure:
  1. **`boundary-build`:** the strict warning set on the driver TU (with the header, the
     `-D` renames and the wrapper linked), and a link with the unit's C at `-O0`.
  2. **`boundary-shape`:**
     - the object defines only `main`;
     - its undefined symbols ⊆ unit symbols ∪ the driver libc allowlist ∪ {`rh_in_at`};
     - the lint variant passes (it allows exactly `#include "ruharness_guard.h"` besides the
       unit's headers);
     - `rh_in` is called at least once.
  3. **`determinism`:** 3 runs in `full` mode; both streams identical; exit 0; the size and
     time bounds of driver validation.
  4. **`sanitizers`:** the ASan+UBSan C build in `full` mode is clean.
  5. **`footprint`:** phases M, L and C of B.4. The detail gives the number of calls and
     allocations, the elements the C touches out of the elements allocated, and how many
     allocations were widened.
  6. **`coverage`:** from phase M's argument records:
     - no call passes a foreign pointer to a data-pointer parameter;
     - every data-pointer parameter of every symbol that has one receives an allocation at
       least once.

  The record is green iff all checks pass. There is no mutation gate (the driver's job is
  extent, and behavior is already gated by `driver.c`) and no `-O0`-vs-`-O2` gate (every
  boundary build is `-O0`).
- **Promotion** mirrors `driver.c`:
  - the file is written atomically;
  - it is re-validated in place;
  - `boundary-validation.json` is stored only if that run is green;
  - a human-written `boundary-driver.c` (no record) is never replaced;
  - replacing a generated one needs `--promote`.

### B.8 Ledger, bench, replay, capabilities

- **Bench.**
  - `CaseInputs` gains `boundary_driver` and `boundary_validation` digests (serde default,
    omitted when empty). Every other case stays byte-identical. An opted-in case shows up
    as "inputs changed", which deliberately forces a re-score.
  - `CasePipeline` gains an informational `boundary`: `validated | stale | failed |
    missing`.
  - A case with a boundary driver counts as Verified only when the boundary validation is
    fresh and green, and the verdict's `boundary_driver` equals the file's digest.
  - The recheck re-validates the boundary driver.
  - There is no `environment` entry. Following the stderr-fix precedent, a changed verdict
    surfaces as an exit-10 PROBLEM rather than being masked as incomparable.
- **Replay.**
  - `bench check --replay` gains the stage `boundary-driver`, judged by
    `validate_boundary_driver`.
  - `superseded.jsonl` `stage` gains `boundary-driver`.
  - `harness state status` staleness includes the digest.
- **Capabilities hardening** (closes a hole that guard pages would otherwise open). The
  candidate's own archive members may not reference `mmap munmap mprotect madvise
  mach_vm_protect vm_protect mach_vm_allocate mach_vm_deallocate mach_vm_map vm_allocate
  vm_deallocate sigaltstack`. These join the `os` class, whose C-side allowance works as
  for the other classes.
- **Opting in, and the migration.** A unit opts in by having a promoted boundary driver.
  - Its recorded green migrate attempts are then re-judged by the new check. For
    read_scalefactors this is a tightening (green → red), recorded in `superseded.jsonl`
    after the unit is re-migrated and the new crate promoted. The scored artifact itself
    can never be superseded.
  - Recorded red turns are unchanged, because the check runs only when all earlier checks
    pass.

### B.9 What the check does not prove (disclosed)

- **Slice creation without access.** A slice longer than the C's window is UB in Rust even
  when it is never read, and nothing on stable can see it. The translator hint (B.10)
  targets it.
- **Memory outside `rh_in`.** Memory the driver did not allocate with `rh_in` is not
  checked, nor is anything reached only through typedef'd pointer parameters.
- **Widened allocations.** Where the C touches an allocation through untraced code
  (memcpy, struct copies), that allocation's window is its whole extent. This is counted
  and disclosed per check.
- **Driver coverage.** An input the boundary driver never exercises is not tested, the same
  limit every driver has.
- **Unsupported programs.** Threads, a unit that installs its own SIGSEGV/SIGBUS handler,
  and variadic or unnamed-parameter interfaces are refused or unsupported.
- **Platform.** macOS arm64 with clang only, until the Linux sandbox. The runtime already
  handles SIGSEGV and 4 KiB pages.

### B.10 Translator hint (a separate, later commit; an ordinary prompt edit)

For units whose interface has a data-pointer parameter only (the conditional-section
precedent of `[STDOUT]`, so no other unit's prompt changes), the translate prompt gains a
pinned `[POINTERS]` section. It explicitly amends STRUCTURE's "ffi.rs holds only …
pointer-to-slice conversion" rule:

- treat every pointer argument as pointing to exactly as much memory as the C accesses on
  that call, since callers may size buffers that tightly;
- never build a slice longer than the C provably accesses on that call (a length computed
  from a field — a bit limit, a capacity — is not the caller's allocation);
- convert a pointer to a reference or slice only on the paths where the C dereferences it;
- when how far the C reads depends on the data, pass the logic function a closure
  (`impl Fn(usize) -> T`) that does the raw read at exactly the index the C reads. This
  keeps ONE call into logic, and logic.rs stays safe.

The repair explanation for a red `boundary-driver` check says the same. The hint lands in
its own commit with its fixture diff, after the check, and each commit gets its own
`bench check --replay` gate.

### B.11 Rollout and calibration

There is no prior art to calibrate the false-red rate against (DECISIONS.md, spike Q5).

- **First:** read_scalefactors. Its boundary driver must turn its verdict red. Then
  re-migrate it through the audited hand-off (a new trial under the hinted prompt),
  supersede the old green attempt, and re-baseline.
- **Proposed calibration set:** all 14 verified pointer-taking units of the released-hidden
  split (read_scalefactors included). Each red is diagnosed by hand, as a real extent
  divergence or a false red. A false red is a design finding, fixed in the rule, not
  excused.
- **The full rollout** (56 verified pointer units) is a follow-up, decided on the
  calibration numbers and recorded with its trigger.

### B.12 Not doing

- Model-sized guarded buffers (the draft): the model cannot know N, and here the harness
  measures it.
- Rust-side sancov: it misses memcpy and relies on LLVM-internal flags.
- libc interceptors for precision: phase L makes them unnecessary for soundness; revisit if
  widening proves common.
- Interposing `malloc`: it is not the caller's memory.
- Per-call windows placed separately per call: an allocation has one address.
- An `environment` entry.
- A new crate.

### B.13 Revisit when

- Widening proves frequent (then add interceptors for `mem*`/`str*`).
- The Linux sandbox lands (runtime selection, gcc lacks trace-loads).
- LLVM's speculative loads (`llvm.speculative.load`, PR #179642) reach Apple clang or
  rustc. The runtime's guard would then see legal over-reads.
- Calibration finds a false-red class.

### B.R — Adversarial design review (4 lenses, 20 verified findings) — AUTHORITATIVE where it amends §B

Lenses: security, measurement validity, contracts/replay/feasibility, simplicity/scope. Every
blocker and serious finding was handed to an independent verifier who tried to refute it against
the code or by experiment; the verdicts below are the verifiers', not the reviewers'. Verdicts:
security **unsound as written** (sound-with-fixes after B.R-1..3); measurement, contracts and
simplicity **sound-with-fixes**. Where B.R and §B.0–B.13 disagree, B.R wins.

**B.R-1 The Rust side must fail closed; a denylist is not a soundness argument (SEC-1 serious,
SEC-2 blocker — both reproduced under the real run-sandbox profile).** Phase R's whole judgment
rests on a guard fault reaching the runtime's handler. A candidate can prevent that from inside the
process: intercept the fault before it becomes a signal (a Mach exception port), replace the
runtime's handler through an installer the `capabilities` classes do not list, or resume past
the fault with `sigsetjmp`/`siglongjmp`. Lexical denylists fail open; the kernel-trap surface
proves they can never be complete. Resolution: after every unit call in phases C and R, and once
at exit, the runtime (harness-owned C, which may use anything) verifies (a) `sigaction(SIGSEGV
| SIGBUS, NULL, &cur)` still names its own handler with its own flags; (b) `thread_get_
exception_ports` / `task_get_exception_ports` for `EXC_MASK_BAD_ACCESS` equal what `init`
recorded; (c) a private `PROT_NONE` canary page, read under `sigsetjmp`, still reaches its own
handler. Any deviation prints `RH-TAMPER <what>` and `_exit(97)` — red. The `mem` class (B.R-8)
remains as policy and defense in depth, never as the soundness claim (SEC-6: the classes are
"a policy, not an enforcement boundary" — `sysctl(KERN_PROCARGS2)` reads the environment and
`openx_np` reads files without touching any listed name).

**B.R-2 Derive the check mechanically from the validated `driver.c`; cut the model-written
boundary-driver stage (S1 — confirmed serious; the simplicity AND measurement lenses each
rebuilt this independently and both caught `read_scalefactors`).** RECOMMENDED; awaiting the
user's decision (recorded in DECISIONS.md). What is cut: B.7 and B.8's stage, prompts,
fixtures, attempts/traces/ids, promotion, `boundary-validation.json`, bench digests and
pipeline field, the replay stage, the superseded stage; from B.3: `rh_in`, the registry, lazy
re-opening, `RH-OVERRUN`. What replaces it: the generated wrapper (B.6) runs the unit's
already-validated, mutation-gated `driver.c` UNMODIFIED under `-D<sym>=ruharness_call_<sym>`.
Measure build: the driver TU is ASan-instrumented (the existing `sanitizers` check already
proves driver+C are ASan-clean) and each direct data-pointer argument's enclosing object is
located with `__asan_locate_address` — exact for stack, heap and global (verified on this
machine, also under `sandbox-exec`); any other kind (unit-owned or uninstrumented memory,
`heap-invalid`) passes through unshadowed and is counted. Tight runs: per call, each located
object is copied into a fresh reservation with the C's per-call window flush against a guard
page (tail, then head), the call runs, every pointer-sized word in the copied objects and a
pointer-typed return value that points into a shadow is relocated back to the original object
(out-params such as `hex2bin`'s `hex_end_p` and pointer returns otherwise dangle — reproduced),
changed objects are copied back, and spent reservations stay `PROT_NONE` (no address reuse within
a run). Per-call windows are exact by construction — S3, M3, M4 and M5's foreign-pointer rule,
and C1, C3, C8, C9, C12 are subsumed; zero tokens; no new ledger surface; the check becomes a
pure function of `driver.c`, the unit and the harness (toolchain entry `boundary:
sancov+guard-pages rt=<8 hex of runtime+headers+wrapper template>`). Lost, disclosed: memory
reached only through pointer fields (B.R-11).

**B.R-3 Honest threat model (SEC-3 minor; SEC-4, SEC-6, SEC-7 refuted).** The check proves extent
containment for a candidate that is not test-aware, plus B.R-1's tamper detection. The unit's C is
the reference by construction: it runs inside the measuring process and can weaken its own check
(widen windows by touching everything, forge the record, fork a writer) exactly as it can fail its
own vectors under §A — no transport change closes that, so none is adopted (B.5's parenthetical
becomes "a hostile unit can weaken its own check, never cause a false red"). Windows are not
secret: they share the candidate's address space. `count × elem` is already overflow-checked at
both ends (SEC-7: not a problem).

**B.R-4 Measure at `-O1`; enforce at `-O0` (S2, M2 serious).** At `-O0` every struct copy lowers
to an untraced `memcpy`, so phase L widens most windows on struct-array units (convex_clip: 75
widened) and element granularity collapses to object granularity. The instrumented unit C in the
measure build is compiled at `-O1 -ffp-contract=off`; every other boundary build stays `-O0`; fall
back to `-O0` measurement only when the `-O1` measure run's streams differ from the plain run's.
Widened objects are NAMED in the detail (call, symbol, parameter, driver line), never just
counted; B.13's interceptor trigger gets a number: widening above 25 % of any data-pointer
parameter's bytes across the calibration set.

**B.R-5 C-side phase failures are red checks, never harness errors (C2 serious).** B.4 misread the
driver-shape precedent: in `verify` a red `driver-shape` is a FAILED CHECK that ends the run; only
the migrate judge turns it into an `Err`. An `Err` from `verify` would leave the old green verdict
on disk and abort `bench score`/`check` for all 100 cases. Resolution: an M/L/C failure (and an
unsupported toolchain) is a failed `boundary-driver` check with the fixed lead-in `boundary driver
invalid (C side): <harness reason>`; `verify` demotes; bench reports one PROBLEM.
`migrate::is_c_side` recognizes the lead-in (never candidate evidence); `MigrateStage::judge`
turns it into an `Err` exactly as for driver-shape.

**B.R-6 Class and evidence (C5, M11 minor).** RH-FAULT, RH-DIVERGED and output differences are
`oracle`; any other candidate crash or timeout is `crash-timeout`; harness-written details never
contain "run failed" or "timed out" (unit test on each detail form). Phase R runs tail, then head,
and reports the first failing layout only. The evidence names a category — below the C's window,
above it, or "in a call where the C does not touch it" — plus the window, never the raw
first-faulting element (it depends on page size and libc's `memcpy` order). The runtime writes the
fault to its own out file, which the harness trusts over stderr (a candidate can print a fake
marker); a leading newline precedes the stderr line. ASCII only: `5 x 1 bytes`.

**B.R-7 Gates that can observe the claims (C7 serious).** Step 0, before any B code: `bench score
--write` at HEAD and commit it — expected diff exactly the six pre-existing `pipeline.
migrate_outcome` drifts from the R-5 promoted-attempt rule, no class, vector, inputs or totals
change; every later `scores.json` diff is then attributable. The `boundary-driver` check and its
toolchain entry are OMITTED when no boundary check ran (a stated exception to the whole-program
"not configured" precedent) so that "every other verdict stays byte-identical" is true. A
per-commit gate script with exact expected outputs replaces prose; the conformance total is
saturated (0/198) and proves nothing for B; fixture diffs are the review surface for prompt edits.

**B.R-8 Capabilities (C4 serious).** The memory APIs form their own `mem` class, never implied by
fs/process/net (the code; the §B.8 text said `os`, which `fopen` would have admitted). Signal-
disposition APIs (`sigaction signal sigaltstack sigprocmask pthread_sigmask sigset sigvec
bsd_signal`) split out of `process` into a `signal` class. A boundary-checked unit's candidate is
never granted `mem` or `signal`, whatever its C uses; candidate references to `rh_*`,
`ruharness_*` or `__sanitizer_cov_*` are always rejected. No recorded verdict or turn changes: all
189 recorded candidate and promoted crates were rebuilt and none references any of these names;
no unit's facts call them (the no-change proof, recorded here because replay only re-checks
attempts bound to the current inputs).

**B.R-9 Typedef'd pointer parameters (SEC-5 minor).** Enforcement is by address, so a typedef'd
pointer's object is guarded once it is shadowed; what B.6 loses is only the classification. Let
the compiler classify, not tree-sitter over raw headers (macros and conditional includes hide the
type): for every parameter not syntactically classified, a `-fsyntax-only` probe with
`_Static_assert(__builtin_classify_type(p) != 5, "")`; a failing probe is a typedef'd data pointer,
with `elem` from `sizeof *p` where the pointee is complete, else 1 and "granularity unchecked".

**B.R-10 Positive controls and calibration (M6, M7 serious; S4 minor).** B.1-4's "one-change fix
passed" came from the whole-run prototype; under the per-call rule that fix is RED at `bs` (M7,
reproduced): it still reads `bs->pos`/`limit` in calls where the C never touches `bs`. The four
`read_scalefactors` variants become committed oracle regression fixtures: verified (red at
`scfcod`), scfcod-lazy (red at `bs`), fully lazy (green in both layouts), over-read of `buf` (red).
Calibration is stratified, not hidden-only: the 14 released-hidden pointer units plus every
public organic unit whose verified `ffi.rs` builds a fixed-length or field-derived slice
(`hdr_compare`, `hdr_bitrate`, `read_side_info`, `dequantize_granule`, `bin2hex`, `hex2bin`,
`md5_digest`); splits reported separately, neither called a test set. Before calibrating, decide
and record the rule's stance on an eager read of a buffer whose size is fixed by contract but
which the C skips on some path (`hdr_compare`, structurally `read_scalefactors`). Units whose only
data-pointer parameters are fully widened or reach the unit only through libc (`printLine`) are
reported "vacuous" and not counted as boundary-checked. Per parameter, phase M yields a power
figure (calls where the object is untouched or partially touched); zero power is disclosed.

**B.R-11 Nested pointer fields (M1 serious).** Memory reached only through a pointer field
(`bs->buf`) is outside the rule and stays disclosed. Cheap partial detector kept: the wrapper
passes `__builtin_frame_address(0)` to `enter`; in measure mode any traced in-call unit access
into the driver's stack range is recorded as `foreign <n> stack` (first call only, no
addresses), and the passed objects are scanned for pointer-sized words into that range; the
detail reports "reached through a pointer field: unchecked". Full nested shadowing is a
revisit trigger (B.13).

**B.R-12 Minor findings adopted.** Learn = one run per layout plus the strict confirm; no round
cap, no "did not converge" error (S6). On divergence `_exit(97)` at once; the probe is linked only
into the measure build (S8). A reentrant unit call is `RH-ERROR`, listed unsupported (M9). Every
instrumented unit object must reference `__sanitizer_cov_*` (`nm`), and `no_sanitize` attributes
or `#pragma clang attribute` in an opted-in unit's sources are refused (M12). B.9 states that a
reference or `&[T; N]` created at the boundary may be read early by the optimizer on any path, so
such reds are real (M13, reproduced with stable rustc). Reservations get large `PROT_NONE` gaps
(≥ max(1 MiB, size)) and a fault is attributed to an object only inside its range plus gap (M14).
A syscall on a closed page returns `EFAULT` instead of faulting: passed and touched objects are
re-opened on exit, and units that pass caller buffers to `read`/`write`/`recv`-family calls are
refused (M8). SCHEMAS.md gets a §B contract checklist mirroring §A's (C11). Bench/pipeline fields
(C6) are moot under B.R-2.

**B.R-13 Translator hint deferred (S5 minor).** The check ships with a repair explanation that
names the call, parameter and window and explicitly authorizes `ffi.rs` to test the C's own
conditions before converting a pointer, or to pass an accessor closure; `read_scalefactors` is
re-migrated on the UNHINTED prompt; the proactive `[POINTERS]` section (B.10) is added only if
calibration re-migrations show repair failing within budget.

**Refuted, recorded.** C1 (bind attempts to the boundary driver; "nothing can excuse" a judge
change) — the `--retry`/`.rN` sample plus `superseded.jsonl` transition already covers a judge-
input change, as the stderr fix did. SEC-4 (a tightness gate) — no party gains from widening;
the C is the reference. SEC-6 (env instead of a file for the table) — neither channel is secret.
SEC-7 (overflow) — checked at both ends. SEC-3's transport/group-kill proposals — neither stops a
unit forging from inside the process (B.R-3).

**What the review changed in the size of the work.** As written, ≈ 4,000–4,500 lines across five
crates plus ≈ 12 prompt fixtures and a hand-off round per calibrated unit. Under B.R-2 (mechanical):
no harness-llm stage, no fixtures, no bench/replay/supersession surface; the runtime, the wrapper
generator, the verify integration, the fixtures of B.R-10 and the classifier probe of B.R-9 —
roughly half, and zero tokens to calibrate.
