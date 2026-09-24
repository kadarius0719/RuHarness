# Oracle hardening after M4 — designs

Status: **A implemented** (design review 3 lenses; code review 2 lenses; §A.R + §A.2 +
§A.3 authoritative; normative contract in docs/SCHEMAS.md). **B designed and adversarially
reviewed** (§B, 2026-09-23; §B.R is AUTHORITATIVE where it amends §B; B.R-2 DECIDED by the user:
mechanical baseline now, a model-written additive driver only as a calibration-triggered revisit).
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
   elements). Under per-call windows (item 5) a Rust that only makes `scfcod` lazy is
   still red — at `bs`, which it still dereferences in calls where the C never touches it
   (§B.R-10, reproduced); a fully lazy Rust passes with identical output. The same results
   hold under a run-like `sandbox-exec` profile.
5. **Windows must be per call.** The driver's own read-back after a call (it prints
   `bs->pos`) makes `bs` "touched" over the whole run, masking the eager `&mut *bs`.
   Windows are measured and enforced **per unit call**, not over the whole run — under the
   decided mechanism, fresh shadows per call give this by construction.
6. **Nothing else works on stable** (re-checked): no Miri or `-Zsanitizer` on stable
   1.94–1.98; `ub_checks` in `from_raw_parts` checks alignment, null and size only; there
   is no libFuzzer runtime in Apple clang. Stable rustc can trace loads
   (`-C passes=sancov-module`), but it misses memcpy and depends on LLVM-internal flags, so
   it is not used; the Rust side is judged by guard pages only.

> **Revision note (2026-09-23, after the review and the user's decision):** B.2–B.13 below are
> the DECIDED mechanical mechanism (§B.R-2). The original model-written boundary-driver stage
> (B.3 `rh_in`, B.7/B.8) is in git history at 67a8cea and remains the §B.13 revisit trigger.

### B.2 Rule (normative)

For a unit whose plan entry opts in (`[unit.oracle] boundary = true`), and for every call the
unit's validated `driver.c` makes to a unit symbol: during that call, the Rust may touch (load or
store) only the objects the C touches during the same call, and within each object only the
elements inside the C's window for that (call, object). Everything else about the run must match
the C — both streams and the exit status — in both layouts.

- **"Object"** is the driver-owned memory an argument points into, as ASan locates it in the
  measure build: the enclosing stack variable, heap block or global, exactly. A pointer ASan
  cannot place (unit-owned or uninstrumented memory, a string literal, NULL) is passed through
  unshadowed and counted ("unshadowed").
- **"Element"** means `sizeof *p` bytes for the parameter `p` the object arrives through (1 byte
  for `void *` and incomplete pointees: "granularity unchecked"). Granularity is per element, so
  reading a whole struct element is never a violation once the C reads any byte of it.
- **"Window"** is the element-rounded hull of the C's traced accesses to that object during that
  call, replaced by the whole object when a learned untraced access falls outside it (then the
  check proves object-level containment for that (call, object), not element-level; such
  objects are NAMED in the detail).
- Accesses by the driver outside unit calls are always allowed. Memory reached only through a
  pointer field of an object is outside the rule (B.9; a partial detector reports it).
- A Rust access outside the rule faults and the check is red; nothing is excused. A reference or
  `&[T; N]` created at the boundary may be read early by the optimizer on any path — such reds
  are real (the C accepts the pointer without reading it).

### B.3 The runtime (harness-owned C, `include_str!`; §B.R-1 integrity built in)

`ruharness_guard.c` is compiled into every boundary build (with `-DRUHARNESS_MEASURE` in the
measure build, where it references `__asan_locate_address`). The driver never sees it: the
driver TU is compiled unmodified with `-D<sym>=ruharness_call_<sym>` for every unit symbol, so
every call goes through the generated wrapper (B.6), which is the runtime's only client.

| `RUHARNESS_GUARD` | Build | Behavior |
|---|---|---|
| `measure` | `bd_measure` | arguments pass through; the sancov callbacks record per-(call, object) byte hulls; the record is written to `$TMPDIR/ruharness-guard.out` at exit |
| `learn-tail` / `learn-head` | `bd_c` | windows enforced; an in-call fault inside a shadow is recorded, the whole object opened, execution continues |
| `tail` / `head` | `bd_c`, `bd_rs` | windows enforced; an in-call fault is recorded and the run ends with exit status 97 |

`RUHARNESS_GUARD_WINDOWS` names the harness-written window table (a listed input; not secret —
B.9).

- **`ruharness_enter(sym, frame)`** starts call *n*. Measure: records `call n sym`. Tight/learn:
  the table's call *n* must name `sym`, else `RH-DIVERGED call=n` and `_exit(97)`.
- **`ruharness_arg(param, p, elem)`** returns the pointer the unit is given. Measure: locates
  `p` (`__asan_locate_address`: kind ∈ {stack, heap, global}, `0 ≤ off ≤ size`, `size > 0`),
  dedupes objects within the call by base address, records `arg n param obj:j:off_bytes` (or
  `null` / `pass`), returns `p`. Tight/learn: the table's classification (null / pass / obj) must
  match, else `RH-DIVERGED arg=n:param`; for an object, a fresh reservation `[gap][pages][gap]`
  (`PROT_NONE`; `gap = max(1 MiB, size)`, page-rounded — B.R-12) is mapped, the object's bytes
  copied in with the window's anchor (its end in tail layouts, its start in head layouts)
  exactly on a page boundary — a page-aligned address ± a multiple of `elem` keeps alignment —
  and only the window's pages opened; the shadow pointer `base' + off` is returned.
- **`ruharness_ret(p)`** relocates a pointer-typed return value that points into a shadow
  (`[base', base'+size]`) back to the original object.
- **`ruharness_exit()`** ends the call: (1) integrity (below); (2) relocation — every
  pointer-sized word at an 8-byte offset from the OBJECT start (whatever the shadow's
  alignment) whose bytes the call changed and whose value points into any shadow of this call
  is rewritten to the original object (out-params such as `hex2bin`'s `hex_end_p`; relocation
  is by value — disclosed in B.9); (3) copy-back of every byte the call changed (against a
  snapshot taken at copy-in, so string literals, `const` globals and writes through an
  unshadowed alias are left alone); (4) the reservations stay mapped `PROT_NONE` for the rest
  of the run — no address is ever reused.
- **Integrity (B.R-1), on every `exit` and once at normal end:** (1) **signal accounting** —
  `getrusage(RUSAGE_SELF).ru_nsignals` is read at `enter`; every signal delivered during the
  call must have been serviced by the runtime's own handler (a candidate that installs its own
  handler through an unlisted route, survives its fault and restores everything before
  returning still leaves the delivery count behind — reproduced: such a candidate is
  `RH-TAMPER signal`); (2) `sigaction(SIGSEGV | SIGBUS, NULL, &cur)` must still name the
  runtime's handler with its flags; (3) `task_get_exception_ports` and
  `thread_get_exception_ports(EXC_MASK_BAD_ACCESS)` must equal what `init` recorded; (4) a
  private `PROT_NONE` canary page, read under `sigsetjmp`, must reach the handler. Any
  deviation: `RH-TAMPER <signal|handler|exception-port|canary>` and `_exit(97)`; (5) every
  reservation of the call is walked with `mach_vm_region`: its closed pages must still be
  `PROT_NONE` and the mapping must still be the private anonymous one it was created as (an
  alias made with `vm_remap` changes the share mode or reference count) — `RH-TAMPER
  protection`. This one is defense in depth, not the soundness claim (RT-1, refuted to
  minor by its verifier): every page-reprotecting route is already denied to an opted-in
  unit's candidate by the `mem`/`signal` classes before phase R runs; reproduced anyway — a
  candidate that opens a closed page and leaves it open is red. What (1)–(5) cannot see: a Mach exception port (no signal is generated) or a page
  re-protection that is installed AND undone inside the call — those routes exist only
  through the `mach_msg` family and `mprotect`/`vm_protect`, which the `signal` and `mem`
  classes deny (B.8); the residual is disclosed in B.9. A process that ends inside a unit
  call is `RH-EXITED` (exit 97) and a run that makes fewer calls than the table is
  `RH-DIVERGED` — the candidate's doing, never a harness fault.
- **Fault handler** (`SA_SIGINFO | SA_ONSTACK`, static alternate stack): a fault inside a
  reservation's range plus gap during a call → tight: `RH-FAULT call=n object=j byte=b` on stderr
  (after a leading newline), `fault n j b` in the out file, `_exit(97)`; learn: `learn n j b`,
  open the whole object, return — unless the byte lies outside the object, which opening
  cannot satisfy: terminal in every mode (RT-3). A fault in a reservation of an EARLIER call
  is `stale n m j b` (a retained pointer). Outside every reservation, or outside a call: restore the default
  disposition and return — the process dies by its own signal, exactly as without the runtime.
  Never prints an address.
- **Measure-mode tracing (`__sanitizer_cov_{load,store}{1,2,4,8,16}`):** inside a call, an
  access inside a located object updates its (call, object) hull; an access in
  `[frame, stack top)` outside every object is recorded once as `foreign n stack` (B.R-11); the
  probe's store outside calls counts the canary (`probe 1`).
- **Limits:** 65 536 calls, 16 objects per call, 64 recorded arguments per call, 16 MiB per
  object, 65 536 reservations per run (they are never unmapped); a limit is `RH-ERROR
  <reason>` and `_exit(96)` — in a C-side run a red C-side check ("not applicable"); in the
  Rust run (where the C ran clean under the same table) `candidate run failed: the guard
  runtime stopped the run (…)`, never a harness error (RT-4). A reentrant unit call is
  `RH-ERROR`.

### B.4 The check, phase by phase

Builds go into a fresh `build/<unit>/bd/`; the runtime, its header, the probe and the generated
wrapper are written there and `-I<bd>` comes first. Every C compile passes `-ffp-contract=off`.

| Build | Driver TU | Unit C | Runtime | Link |
|---|---|---|---|---|
| `bd_measure` | `-O0 -fsanitize=address`, renames | `-O1`, sancov (`-fsanitize-coverage=edge,trace-loads,trace-stores`) | `-DRUHARNESS_MEASURE`, probe instrumented | `-fsanitize=address` |
| `bd_c` | `-O0`, renames | `-O0` | plain | plain |
| `bd_rs` | `-O0`, renames | — (staticlib) | plain | plain |

0. **Plain reference.** `driver.c` + unit C at `-O0`, run once: the expected streams.
1. **Phase M: measure.** `bd_measure` in `measure` mode must exit 0 with streams equal to the
   plain run's (else the unit is re-measured at `-O0`; if that differs too: red, C side). The out
   file (B.5) is parsed strictly; `probe 1` required; every instrumented unit object must
   reference `__sanitizer_cov_*` (`nm`). Windows = element-rounded hulls per (call, object).
2. **Phase L: learn.** One `learn-tail` and one `learn-head` run of `bd_c` (exit 0 required).
   Each learned `(n, j, byte)` outside the window widens that (call, object) to the whole object
   and marks it widened. No repetition: for a deterministic C a second round finds nothing.
3. **Phase C: confirm.** `bd_c` strictly in `tail`, then `head`: exit 0 and streams identical to
   the plain run. This proves the windows cover every C access, traced or not. Failure: red,
   `boundary driver invalid (C side): …` (B.R-5).
4. **Phase R: judge.** `bd_rs` strictly in `tail`, then `head`, stopping at the first failing
   layout. Each run must exit 0 with streams identical to the plain run. `fault n j b` → red with
   the harness's own detail: the call, symbol and parameter, the object's size in elements, the
   category (below the C's window / above it / the C does not touch it in that call) and the
   window; a widened object is named as such. `RH-DIVERGED` / `RH-TAMPER` → red ("the Rust
   changed the driver's control flow" / "the guard was tampered with"); any other exit or output
   difference → the standard wording.

**Where it sits in `verify`:** named `boundary`, last, after `sanitizers`, and only when every
earlier check passed. Only for an opted-in unit; otherwise NO check entry and NO toolchain entry
(a stated exception to the whole-program "not configured" precedent, so every other verdict stays
byte-identical). When it ran, `inputs.toolchain` gains `boundary: sancov+guard-pages
rt=<8 hex of blake3(runtime ‖ headers ‖ probe ‖ wrapper template)>`, placed before `observable`.
Preconditions, checked before any build (each a red C-side detail, closed set): a fresh green
driver validation; every interface line parses (B.6) and names a plan symbol; ≥ 1 data-pointer
parameter; no `signal`/`mem` class in the unit's own C; no `no_sanitize` attribute or
`#pragma clang attribute` in the unit's sources; macOS + clang (until the Linux sandbox).

**Class and evidence (B.R-6):** RH-FAULT, RH-DIVERGED, RH-TAMPER and output differences are
`oracle`; any other candidate crash or timeout is `crash-timeout`; harness details never contain
"run failed" or "timed out". The repair explanation names the call, parameter and window and
authorizes `ffi.rs` to test the C's own conditions before converting a pointer, or to pass an
accessor closure into `logic` (B.R-13).

### B.5 Harness-parsed files (strict, capped, numbers only)

`$TMPDIR/ruharness-guard.out` (written by the runtime, read back before the temp dir is removed:
regular file, no symlink, ≤ 16 MiB). Measure:
`ruharness-guard 1 measure` · `probe <0|1>` · `call <n> <sym>` · `arg <n> <param>
null|pass|obj:<j>:<off>` · `obj <n> <j> <size> <elem> <lo> <hi>` (byte hull; `0 0` = untouched)
· `foreign <n>` · `end`. Learn: header, `learn <n> <j> <byte>`…, `end`. Tight: header, then
exactly one of `end` · `fault <n> <j> <byte>` · `stale <n> <m> <j> <byte>` · `exited <n>` ·
`tamper <signal|handler|exception-port|canary|protection>` · `diverged call|arg <…>` ·
`error <reason>`. The window table the harness writes is strictly sequential: `ruharness-windows
1 <calls> <layout>` · per call `call <n> <sym> <nobj> <nargs>` · `obj <j> <size> <elem> <lo>
<hi>` × nobj (elements) · `arg <param> <kind> <j> <off>` × nargs (kind 0 null, 1 pass, 2 object;
`off` in bytes) · `end`. Ids dense and in order; every bound
checked; names resolved from the harness's own interface parse. A malformed file is a red C-side
check. (A hostile unit can weaken its own check, never cause a false red — B.9.)

### B.6 The wrapper, generated from the interface lines

Each plan `interface` line must parse (`harness_scan::parse_interface`) as one function
declaration of a plan symbol with every parameter named and no attribute, preprocessor, brace,
semicolon or variadic syntax — else the unit is not applicable (red C-side detail).
Classification: a parameter with a pointer or array declarator that is not a function pointer is
a data pointer; a parameter not classified syntactically is probed with the compiler
(`-fsyntax-only`, the unit's headers): `_Static_assert(__builtin_classify_type(p) != 5, "")`
failing ⇒ a typedef'd data pointer; then `(void)sizeof(char[sizeof *p])` compiling (under
`-Werror=pointer-arith`) ⇒ `elem = sizeof *p`, else 1 ("granularity unchecked").

```c
#include "<every header of the unit's include closure, absolute path>"
#include "ruharness_guard_internal.h"
<interface line>;                                            /* per symbol: its prototype */
static __typeof__(<sym>) *const ruharness_real_<sym> = <sym>; /* bound where no parameter can shadow it */
<interface line, function renamed ruharness_call_<sym>>
{
    ruharness_enter(<i>, __builtin_frame_address(0));
    __typeof__(<p>) rh_a<k> = (__typeof__(<p>))ruharness_arg(<k>, (const void *)<p>, sizeof *<p> /* or 1 */);
    …
    __typeof__(<call>) rh_ret_ = ruharness_real_<sym>(<rh_a<k> or plain args>);   /* void: no rh_ret_ */
    rh_ret_ = (__typeof__(rh_ret_))ruharness_ret((void *)rh_ret_);             /* pointer returns only */
    ruharness_exit();
    return rh_ret_;
}
```

Symbol and parameter names are plain C identifiers by construction (the parse refuses others),
so nothing else reaches a `-D` argument or the file; a header path with `"` or `\` is refused.
The prototype comes from the interface line because the corpus's helper symbols are often
declared in no header (the driver declares them itself — 14 of 100 cases); the file-scope
binding exists because a parameter may be named like its function (`crc16`'s last one is).
Locals are named by parameter index so no parameter name can collide with them. The renamed
line is the interface line with its declarator identifier replaced (byte range from the parse),
never re-emitted from parts. The compiler classifies what the parse cannot: a baseline probe
(the prototype with an empty body) must compile, else the headers do not declare the line's
types and the unit is not applicable.

### B.7 Ledger, bench, replay

Nothing new is recorded per unit: the check is a function of `driver.c`, the unit source, the
crate and the harness (its `rt=` digest). Opt-in is the plan key `[unit.oracle] boundary = true`
(kind-owned, validated). `harness verify` runs it inside the ordinary verdict; a red demotes as
any red does. **Calibration** (B.R-10): `harness bench boundary --suite DIR [--case NAME]…` runs
phases 0–R for every verified unit that has a data-pointer parameter, writes NOTHING (no
verdicts, no scores), and prints per unit: green / red (category, call, parameter) / not
applicable (reason) / vacuous (every data-pointer object fully widened or unshadowed), plus
widened objects, unshadowed arguments and a power figure per parameter (calls where the C leaves
its object untouched or partially touched). The stratified calibration set and the eager-fixed-
size-read stance are decided and recorded before the run. Turning the check on for a unit is a
reviewed plan edit; a recorded green attempt that then goes red is handled by the existing
judge-change transition (`migrate --retry` → `.rN`, or a new trial plus `superseded.jsonl` —
B.R "refuted C1").

Bench: `bench check`'s recheck simply re-verifies (the check is part of `verify`); an opted-in
unit whose verdict goes red is a PROBLEM (exit 10), never an abort. No `environment` entry, no
schema change to `scores.json`.

### B.8 Capabilities (B.R-8)

`mem` class (memory mapping/protection and Mach VM entry points), never implied by
fs/process/net; `signal` class — fault interception: `sigaction signal sigaltstack sigprocmask
pthread_sigmask sigset sigvec bsd_signal __sigaction`, the exception-port setters
(`task_set_exception_ports thread_set_exception_ports task_swap_exception_ports
thread_swap_exception_ports`), raw Mach messaging (`mach_msg mach_msg_overwrite mach_msg2
mach_msg_trap mach_msg2_trap`), port and thread creation (`mach_port_allocate
mach_port_insert_right mach_port_construct thread_create thread_create_running
thread_set_state thread_resume`) and the `_kernelrpc_*` traps — split out of `process`.
Verified against the 191 recorded candidate crates: none references any of these names. An opted-in unit's candidate is never granted `mem`
or `signal`; references to `rh_*`, `ruharness_*` or `__sanitizer_cov_*` are always rejected.
These are policy and defense in depth; the soundness claim is B.R-1's integrity check.

### B.9 What the check does not prove (disclosed)

- **Slice creation without access** is invisible on stable Rust; the boundary is judged by
  accesses (including the optimizer's early reads of boundary references).
- **Memory reached only through a pointer field** of an object (`bs->buf`): unchecked; the
  measure-mode driver-stack detector reports "reached through a pointer field: unchecked" when
  the C reads the driver's stack through one. Revisit trigger: a model-written additive driver
  that shadows nested buffers (the original B.7/B.8, git history 67a8cea), IF calibration shows
  this gap matters in practice (user decision 2026-09-23).
- **Unshadowed arguments** (string literals, unit-owned memory, pointers ASan cannot place) and
  **widened objects** (reached through untraced code: `memcpy`, struct copies, libc): only
  object-level containment; both named per check. A unit whose data-pointer objects are all
  unshadowed or widened is "vacuous" and not counted as boundary-checked.
- **Threat model.** The unit's C is the reference by construction: it runs inside the measuring
  process and can weaken its own check (touch everything, forge the record, fork a writer)
  exactly as it can fail its own vectors under §A; no transport change closes that. Against a
  test-aware candidate the guarantee is B.R-1's tamper detection (signal accounting, handler,
  exception ports, canary, page protections), not the capability classes; windows are not
  secret. Residual: a candidate that reaches an exception-port setter or `mprotect` through a
  route no class lists, and undoes it before returning, is not detected (reproduced for
  `mprotect`: leave-open is `RH-TAMPER protection`, restore-before-return is green) — the
  known routes (`mach_msg`, the traps, `mprotect`/`vm_protect`) are denied lexically, and
  inline assembly is banned.
- **Relocation is by value:** a pointer-sized datum the call wrote that happens to equal a
  shadow address is relocated (mmap addresses under ASLR make this negligible for organic
  code; a test-aware candidate gains nothing it could not get by storing a real pointer).
- **ASan's object attribution:** a pointer within 64 bytes before a global may be attributed
  to that neighbouring global; such an argument is passed through unshadowed (counted, never a
  false red). Syscalls on closed pages return `EFAULT` rather than faulting: units
  whose C passes caller buffers to `read`/`write`/`recv`-family calls are not applicable.
- **Unsupported:** threads, a unit installing its own SIGSEGV/SIGBUS handler, reentrant unit
  calls (callbacks into unit symbols), variadic or unnamed-parameter interfaces, non-macOS
  (until the Linux sandbox: SIGSEGV, 4 KiB pages, gcc without sancov).

### B.10 Translator hint — deferred (B.R-13)

The check ships with the repair explanation of B.4 only. `read_scalefactors` is re-migrated on
the unhinted prompt. A proactive `[POINTERS]` section is added — as its own commit with its
fixture diff — only if calibration re-migrations show repair failing within budget.

### B.11 Rollout and calibration (B.R-10)

Step 0: `bench score --write` at HEAD before any B oracle code (the six pre-existing
`pipeline.migrate_outcome` drifts). Then: the four `read_scalefactors` Rust variants as oracle
regression fixtures (verified → red at `scfcod`; scfcod-lazy → red at `bs`; fully lazy → green;
`buf` over-read → red). Then `bench boundary` over the stratified set (14 hidden pointer units +
the public organic units whose verified `ffi.rs` builds a fixed-length or field-derived slice),
splits reported separately; every red diagnosed by hand — a false red is a rule finding, fixed in
the rule, never excused. Then opt in `read_scalefactors`, re-migrate it through the audited
hand-off, supersede the old green attempt, re-baseline. The remaining pointer units are opted in
on the calibration numbers, recorded with their trigger.

### B.12 Not doing

The model-written boundary driver (revisit trigger in B.9). Rust-side sancov. libc interceptors
(widening is named; trigger in B.13). Interposing `malloc`. Keeping windows secret. A tightness
gate. An `environment` entry. A new crate.

### B.13 Revisit when

- Calibration shows the nested-pointer-field gap matters (→ the additive model-written driver).
- Widening exceeds 25 % of any data-pointer parameter's bytes across the calibration set (→
  `mem*`/`str*` interceptors in the measure build).
- The Linux sandbox lands (signal, page size, gcc).
- LLVM's speculative loads (`llvm.speculative.load`) reach Apple clang or rustc.

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
reproduced): it still reads `bs->pos`/`limit` in calls where the C never touches `bs`. The
`read_scalefactors` variants' outcomes under the decided mechanism (calibration, 2026-09-23):
verified → red at `bs` in call 1 (it dereferences `bs` before touching `scfcod`), scfcod-lazy →
red at `bs`, fully lazy → green in both layouts, the `buf` over-read → green with the
pointer-field note (the disclosed limit). The committed positive controls are the synthetic
unit of `crates/harness-oracle/tests/boundary.rs`, which reproduces each outcome class; the
eager-read stance was recorded in DECISIONS.md ("Design B calibration") after the first run,
not before — the reds it governs (`hdr_compare`, `wcscat`) stand as the rule says.
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
