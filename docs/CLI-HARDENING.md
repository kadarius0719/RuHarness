# CLI hardening — the milestone before the review cockpit

Status: IMPLEMENTED (2026-09-24; §R holds the resolutions of the 15 confirmed design-review
findings and the 11 confirmed code-review findings — the text below is the design as built). Sources and rejected
alternatives: DECISIONS.md "TUI track: §15 research spike" and "CLI hardening: design review".

The review cockpit (`harness-tui`) and the Claude Code bridge (`harness-mcp`) — both built
since — are *clients* of the `harness` CLI: they read the ledger and spawn `harness` for every write.
Four things must be true of the CLI before a second process can drive it safely:

1. two writers can never mutate one ledger at once (**§1 writer lock**);
2. a green attempt can be recorded without being promoted, and promoted later by an
   explicit act (**§2 `migrate --no-promote` + `harness promote`**);
3. cancelling the CLI cancels everything it started, including sandboxed process groups,
   and never turns the cancellation into model evidence (**§3 cancellation**);
4. a client can follow a run without parsing human prose (**§4 `--json` events**).

Everything here is append-only against the CLI contract (docs/SCHEMAS.md "CLI contract"):
new flags, one new subcommand, one new config key, one gitignored file. Human stdout stays a
non-contract; the events stream becomes one.

## 0. Ground rules carried over

- `#![forbid(unsafe_code)]` in every crate. Process-group kills stay as they are
  (`/bin/kill -- -<pgid>`, exec.rs); the lock is std's `File::try_lock` (stable since Rust
  1.89 — the workspace `rust-version` moves from 1.85 to 1.89); signals go through
  `signal-hook` (the one new crate: +2 unique, `signal-hook-registry` and `errno`; std has no
  signal API and the registry's raw entry point is `unsafe`).
- Every child the harness runs is spawned by `exec::scrubbed_command` + `run_with_timeout`
  (the only `Command::new` sites outside tests) and leads its own process group. §3 builds
  on that single choke point rather than adding a second spawn path.
- The ledger is the truth; events are a *courtesy* for a live consumer. Anything an event
  says must also be recoverable from the ledger afterwards — so event fields are the
  ledger's own values, verbatim (the Turn's `result`, the verdict's checks), never a
  re-encoding.
- Target-owned files are hostile input (SCHEMAS.md "Trust boundaries"); the new in-place
  write of §1 gets the same symlink discipline as every other write.

## 1. The writer lock

**File.** `<target>/migration/.lock` — created on first use, never deleted, gitignored
(zopfli and the tractor case patterns get a line each). An advisory `flock(2)` exclusive
lock held on the open file description (`File::try_lock`); the kernel drops it when the
holder exits or dies, so a crash never leaves a stale lock — the defect of the existing
`create_new` `BenchLock` (harness-oracle/src/bench.rs), which moves to the same mechanism
in this milestone (`.bench/LOCK` becomes a permanent gitignored file; its remove-on-drop and
its "remove it yourself" message go away).

**Open protocol (SEC-1).** The harness never truncates through a link it did not verify:
`symlink_metadata(path)` first — `NotFound` → `create_new(true)` (O_EXCL never follows a
symlink; on `AlreadyExists` fall through once); a regular file with `nlink == 1` → open
read+write without `create`/`truncate`, then `fstat` and require the same `(dev, ino)`,
regular, `nlink == 1`; anything else (symlink, directory, hard link) → `Error::Invariant`
"…is not a regular file; refusing to lock the ledger". `migration/` itself must be a real
directory (not a symlink). Only after `try_lock` succeeds: `set_len(0)`, write the holder
line, flush. An un-locked open can never destroy content.

**Holder record.** One JSON line `{"pid":N,"command":"migrate u-lib","started":"<RFC 3339>"}`
written under the lock; on a clean release (`Drop`) the file is truncated to empty BEFORE the
`File` closes (still exclusive, so no reader sees a torn line). Wall-clock is allowed: the file
is gitignored, outside every hashed set and non-canonical. Diagnostics only — never consulted
to decide staleness.

**Readers never lock (CONC-M1).** `WriterLock::holder(&Ledger) -> Result<Option<Holder>, Error>`
READS the line (≤ 4 KiB, first line, `command` rendered printable and ≤ 80 bytes): empty or
absent → `None`; a parsed line → `Some` — a live holder, or one that died without cleanup
(every signal death leaves one). `unit_report` checks liveness itself (`kill -0`, the same
unsafe-free probe exec.rs uses; a failed probe reads as alive) so a dead holder's line is
never mistaken for a writer at work, and every client gets that through the one function.
flock has no query operation, so a try-lock "probe" would itself take the exclusive lock for
microseconds and knock over a real writer's fail-fast acquisition; the design has no probe.

**Who takes it.** Every subcommand that writes under `migration/` or the target tree:
`scan`, `plan`, `verify`, `detect`, `observe`, `review`, `migrate`, `gen-driver`,
`promote`, `sync-runtime` — right after `TargetContext::load`, before promotion recovery
(recovery is a write), held to exit. `harness state status` and the cockpit's reads take
nothing. **Bench (CONC-M2):** `bench score` and `bench check` are writers of every selected
case ledger (building a unit crate or a candidate writes `units/<id>/<crate>/target/` and
`Cargo.lock`; `--replay` writes `.replay-<id>/`), so they acquire the lock of EVERY selected
case that has a ledger up front — after `load_verified`, before `Scorer::prepare` and before
`replay_all` — into one `Vec<WriterLock>` held until the command returns; contention is
reported before any build, and `check`'s two passes (`replay_all`, then `compute`) run under
one continuous hold, so the crate `--replay` judges against is the crate that is scored. A
suite run therefore excludes `migrate` on its selected cases for its duration — the honest
trade; cases without a ledger take nothing (no `.lock` is created in an uninitialised case).

**What a lock-free reader can see (CONC-H1).** Single files are atomic (`write_atomic`), but
every verdict-bearing write is a non-atomic PAIR: `verify` stores `oracle-latest.json` then
flips `plan.toml` status; a promotion stores the verdicts then the status then the attempt's
`promoted` flag; driver promotion replaces `driver.c` then stores its validation. A reader's
own plan-then-verdict snapshot is a wider window than the writer's. The in-flight signal for
readers is therefore `holder()`, whose `command` names the unit — NOT marker directories.
`harness state status` (human and `--json`) makes its contradiction/stale detection
two-phase: when the rule fires for a unit, re-load the plan and that unit's verdict once and
recompute; if it still fires, consult `holder()` — `Some` → report `write-in-flight` with the
holder (an additive value of the open `state` set, never `contradiction`); `None` →
`contradiction`/`stale` as today. The cockpit's in-process reader applies the same rule.

**Contention.** Non-blocking. A second writer fails immediately with exit 1:
`ledger is locked by another harness command (pid 4242, `migrate u-lib`, since
2026-09-24T09:14:02Z); wait for it or stop it`. In the `WouldBlock` path the holder line may
be a new holder's not-yet-written record (microseconds): sleep 2 ms and re-READ once (never
re-lock); an empty or unparseable line is worded "ledger is locked by another harness command
(holder record not yet written); retry". No `--wait`: the cockpit and the MCP server retry
on their own schedule. Revisit when a client needs queueing.

**API.** `harness_core::ledger::WriterLock { file, path }` — `acquire(&Ledger, command) ->
Result<WriterLock, Error>` (owns the `File`; never `try_clone`s it out; `.lock` is never
written through `write_atomic`, whose rename would swap the inode from under the flock),
`holder(&Ledger)`. `Error::Locked { holder: Option<Holder> }`. The test pins the rule std makes
load-bearing: two `acquire`s on one path in one process — the second is `Locked` (flock
contends per open file description, unlike fcntl); drop the first, the second succeeds;
`holder()` is `None` after drop; a reader looping `holder()` while another thread
acquires/drops 1000× never makes an acquisition fail.

**Not covered.** NFS (flock is unreliable there; the ledger is local by design). Two
harness *builds* of different versions on one ledger (schema versions guard the files).

## 2. `migrate --no-promote`, `harness promote`, and a crash-proof promotion

**Today** (main.rs `cmd_migrate`, SCHEMAS.md "Promotion protocol"): a green live attempt is
promoted at once unless the unit is already verified (then `--promote` forces it).

**The protocol, made idempotent (crash-1).** ONE marker spans the whole protocol:
`units/<id>/.promote-<attempt>/`. Steps: (1) the attempt record is already stored; (2) stage
the candidate's closed file list into the marker dir and check its digest; (3) two renames
(`<crate>` → `.<crate>.prev` when a crate exists, staged → `<crate>`) — the marker dir stays
(empty) ; (4) verify IN PLACE; a red or an error rolls back (remove the crate, restore
`.prev`, remove the marker); (5) the green tail, every step idempotent, in this order: store
`oracle-latest.json` → `oracle-last-green.json` → `oracle-latest.md` → `plan.toml` status
`verified` → attempt `promoted: true` → remove `.prev` → remove the marker LAST.

**Recovery, by evidence (`recover_promotion`, replaces `recover_interrupted_promotion`),**
runs in every writing command right after the writer lock (`verify`, `migrate`, `promote`;
`gen-driver` keeps its own path). For each `.promote-<id>/` in the unit dir (and a bare
`.<crate>.prev` with no marker, the legacy case): load `attempts/<id>/attempt.json` (absent →
rollback); `swapped` := the crate on disk exists and `crate_content_hash(crate) ==
candidate_digest`; not swapped (killed before or between the renames) → if the crate is
absent and `.prev` exists, rename it back; remove the marker (it may still hold the staged
copy). Swapped → `verified_in_place` := `oracle-latest.json` is green and its `rust_crate`
digest equals the crate's file-set hash; true → finish the tail (idempotent) and remove
`.prev`/marker; false → roll back exactly as step (4). A leftover marker for another id at
promotion time is a bug, not staging debris — recovery has already resolved every marker.
A first promotion (no `.prev`, nothing scaffolds `units/<id>/<crate>/`) is covered by the
marker; today it is not (its unverified crate would be left at the promoted path).

**`promote_attempt`.** The protocol lives in `fn promote_attempt(ctx, ledger, plan_path,
unit, record, attempt_dir, candidate) -> Result<Promotion>` with two callers:

- `harness migrate <UNIT> [--no-promote | --promote]`: on green, promote unless told not to.
  Precedence: `--promote` > `--no-promote` > `[llm.migrate] promote_on_green` (harness.toml,
  additive `Option<bool>`, default true) > default. `--promote` also forces replacing an
  already-verified unit, as today. With `promote_on_green = false` the only promotion path
  is `harness promote` (or an explicit `--promote`): "Accept = an explicit act" made sticky
  across the several invocations one `external` attempt takes (CONTRACT-4) — the flag alone
  is per-invocation and the resume that reaches green decides. The user sets the key by hand
  (harness.toml is committed, target-owned input; clients never write it); both first-party
  clients pass `--no-promote` on every `migrate` spawn regardless. The awaiting exit prints
  the exact resume command including the flags that were passed, and the `awaiting` event
  carries it as `resume`. The human line and the `attempt` event carry the promotion REASON
  (`promoted: default | --promote`; `not promoted: --no-promote | promote_on_green=false |
  already verified | replay`), courtesy only — the ledger says `promoted: true/false`.
- `harness promote <UNIT> <ATTEMPT> [--replace] [--target DIR] [--allow-unsandboxed]`:
  requires the sandbox like `migrate` (it runs model-derived code). Refusals, in this order,
  before any filesystem write (exit 1; `error.kind` `stale` for the binding/preconditions,
  `harness` otherwise):
  1. `<ATTEMPT>` is a clean path segment (the rule replay pinning uses), so an MCP-supplied
     id never becomes a path traversal;
  2. `attempts/<ATTEMPT>/attempt.json` loads with `record.id == <ATTEMPT>` and
     `record.unit == <UNIT>` (`harness_core::attempts::load_pinned`, moved out of
     trajectory.rs so replay pinning, `promote` and the cockpit share it);
  3. `record.stage` is `None` (migrate records omit `stage`; drivers keep their own path —
     a cockpit Accept on a driver attempt is a follow-up), `outcome == "green"`, the last
     turn's `result == "green"`, `candidate_digest` non-empty, `promoted == false` unless
     `--replace`;
  4. the migrate preconditions, re-established because the attempt may be older than the
     tree (`fn migrate_preconditions`, extracted from `cmd_migrate` and shared): the unit's
     plan `source_hash` matches the tree, and R6 — a unit with driver-generation history has
     a `validated` driver (`bench::driver_state`), else "run `harness gen-driver <UNIT>`";
  5. binding (CONTRACT-1/SEC-2): `(record.unit_source, record.driver) == current_binding(ctx,
     facts, unit)` — the include-closure file-set hash and the driver file hash, derived
     exactly as `run_migration` records them and as `bench` recognises provenance (R-5);
     else "attempt … is bound to superseded inputs (its <unit source|driver> is not the
     current one); re-run `harness migrate <UNIT> --no-promote` to record an attempt against
     the current tree";
  6. `candidate/` exists and `crate_content_hash == candidate_digest`;
  7. the unit is not already `verified`/`merged`, or `--replace` was passed.
  Then `promote_attempt`: exit 0 verified, exit 10 rolled back on a red in-place verdict,
  exit 1 on a harness error (rolled back too). Its guarantee is "verified in place, bound to
  the current inputs"; provenance beyond the digest (a consistent hand edit of `candidate/`
  AND `candidate_digest`) is replay's job (`bench check --replay`), as today.

**Attempt id form.** Full id, positional, explicit — no "latest green" default.

**Writer table.** `units/<id>/<crate>/**`, `oracle-latest*.json`, `plan.toml` status —
`migrate` (promotion) and `promote`; `units/<id>/<crate>/target/**`, `Cargo.lock`,
`units/<id>/.replay-*/` — `bench score`, `bench check` (builds; scratch).

## 3. Cancellation

**Today.** Each child leads its own process group; a timeout or output overflow kills the
group (exec.rs). Nothing handles SIGINT/SIGTERM in the CLI: Ctrl-C reaches only `harness`
(the children are not in the terminal's foreground group), which dies by the signal, and
the sandboxed group runs on until its own timeout. A cockpit that SIGTERMs its `harness`
child has the same problem.

**Mechanism — cancellation is a state the spawn choke point observes (sig-1).** In
`harness_oracle::exec`: `static CANCELLED: AtomicBool` and `static LIVE: Mutex<BTreeSet<u32>>`.
`run_with_timeout` takes `LIVE` BEFORE `spawn()`: if `CANCELLED` is set it returns
`Err(Error::Interrupted)` without spawning; otherwise it spawns, inserts the pgid and drops
the guard (spawns serialise by ~1 ms — irrelevant next to the children). When `try_wait`
returns, and in the timeout/overflow branch, if `CANCELLED` is set the group is killed,
reaped, removed from `LIVE`, and the result is `Err(Error::Interrupted)` — never a
`ChildEnd`. So a SIGKILLed child is never judged: today `built_with_env` folds every
non-success status into `RunFailure` ("always evidence, never a harness error"), and a kill
reaped by the 50 ms poll before the process is dead would be journaled as a `crash-timeout`
turn, close the attempt `red` (immutable), and later make `bench check --replay` report
`Diverged` — a harness-bug finding manufactured by the user's Ctrl-C. `built_with_env`
therefore takes the shape `tool_outcome` already has, `Result<Result<RunOutput, RunFailure>,
Error>` (outer `Err` = could not run / interrupted; inner `Err` = evidence), and its
consumers (`lib.rs` run checks, sanitizers, whole-program runs, `validate.rs`, `confine.rs`)
`?` the outer.

`pub fn kill_live_process_groups() -> usize`: sets `CANCELLED`, takes `LIVE` and keeps it
(the caller exits while holding it, so a spawner arriving later blocks and dies with the
process — both the spawn-to-insert window and the spawn-after-the-loop window are closed,
not narrowed), SIGKILLs each group via the existing `kill_process_group`, returns the count.

**The handler (sig-2).** `harness-cli` installs, first thing in `main`,
`signal_hook::iterator::Signals` for `SIGINT`, `SIGTERM`, and — only when stdout or stderr
is a terminal — `SIGHUP` (a closed terminal or tmux pane; `nohup` sets SIGHUP to `SIG_IGN`
and redirects the output, and installing a handler would silently override that inherited
disposition, which std cannot read without `unsafe`; a detached run keeps its own). On the
first signal: `kill_live_process_groups()`, then the courtesy output — the stderr line and
the events-mode `result` — from a helper thread with a 250 ms budget (the main thread may be
parked in a write on a full stdout pipe holding the stdout mutex, and a closed stderr would
make `eprintln!` panic; neither may delay dying), then
`signal_hook::low_level::emulate_default_handler(sig)` — reset to `SIG_DFL` and re-raise — so
the process dies BY the signal: bash/zsh abort
a `for u in …; do harness migrate $u; done` loop exactly as they do today (a normal exit 130
would make an interactive shell continue to the next unit and spend a model call — verified
on this machine's bash 3.2 and zsh 5.9), and a parent sees `ExitStatus::signal() == Some(N)`.
Fallback if the re-raise fails: `process::exit(128 + sig)`. A second signal during the group
kill lands in signal-hook's handler and is ignored.

**Why not cooperative cancellation** (a flag polled by the turn loop): the blocking points
are an HTTP call (`ureq`) and `run_with_timeout`'s poll loop; the first cannot be interrupted
without an async client, and the ledger is crash-consistent — `attempt.json` is journaled
after every completed turn, `write_atomic` leaves at worst a `.tmp-<pid>` file, promotion
recovers by evidence (§2), the writer lock is released by the kernel. Dying is safe once the
children are dead and no child's death is evidence. Revisit when the LLM client is async.

**Contract.** On SIGINT/SIGTERM/SIGHUP the harness kills every live sandboxed process group
and then terminates BY that signal (`code() == None`; shells report 130/143/129). Not new
exit codes. The `result` event's `exit` is the shell-visible code (128+N) with an additive
`"signal":"SIGINT"` field.

**Tests.** exec.rs: a sleeping child through `run_with_timeout` on a thread,
`kill_live_process_groups()` from another, the run ends `Err(Interrupted)` and the group is
gone; a spawn after cancellation returns `Interrupted` without spawning. harness-cli e2e (a
`spawn` variant of the runner, since `.output().code()` reads signal death as `-1`): `harness
verify` on a fixture whose driver sleeps, `/bin/kill -INT <pid>` once the sleep exists,
`status.signal() == Some(2)`, no surviving group.

## 4. `--json` events mode

**Surface.** A global flag before the subcommand: `harness --json <cmd> …`. With it, stdout
carries newline-delimited JSON only (one compact object per line — the cargo / Claude Code
`stream-json` convention); human logs and errors go to stderr. Without it, nothing changes.

**Envelope.** First line `{"k":"header","schema":"ruharness-events","schema_version":1,
"command":"migrate","args":["u-lib"],"pid":4242,"harness":"<crate version>"}`; last line
`{"k":"result","exit":N}` (plus `"signal"` on signal death, best effort). `k` is an open
enum: consumers skip unknown kinds and ignore unknown fields (SCHEMAS.md "Global rules").
No timestamps. Additive changes never bump the version.

**Kinds (v1).**

| `k` | fields | emitted by |
|---|---|---|
| `message` | `text` | every line `out()` prints today (nothing is lost) |
| `facts` | `files`, `stale` | `state status`, before the units |
| `unit` | the `UnitReport` (EVENTS-3): `id`, `status`, `source_fresh`, `verdict: {state: present\|missing\|unreadable, green, stale: [source\|rust-crate\|driver]}`, `contradiction`, `write_in_flight: {pid, command}` (CONC-H1), `attempts: [{id, provider_kind, outcome, bound}]` | `state status`, one per unit |
| `turn-start` | `unit`, `attempt`, `index` (1-based), `kind`, `request_key` | migrate, gen-driver, replay — before the call |
| `turn-end` | `unit`, `attempt`, `index`, the Turn verbatim: `kind`, `result` (the ledger's closed set `green \| format \| check \| build \| oracle \| crash-timeout \| truncated \| blocked`, treated as open), `request_key`, `response_hash`, `input_tokens`, `output_tokens` | right after the Turn is pushed |
| `attempt` | `unit`, `id`, `outcome`, `provider`, `model`, `promoted`, `promotion` (reason) | end of a run, verified recorded attempts |
| `check` | `unit`, `name`, `passed`, `detail` | verify, promote, migrate's final verdict — one per check |
| `verdict` | `unit`, `green`, `path` | only after a verdict is stored at `path` (verify; migrate's final judged turn, `attempt-verdict.json`; a green promotion) — never for a rolled-back promotion, whose verdict is not stored (its `check` lines are) |
| `promote` | `unit`, `attempt`, `result` (`verified` / `rolled-back`) | migrate, promote |
| `awaiting` | `attempt` (null for triage), `path`, `resume` (the exact re-run command) | the `external` hand-off |
| `error` | `message`, `kind` (`locked` / `stale` / `awaiting` / `interrupted` / `harness`) | any command, before `result` |

**Typed errors (EVENTS-2).** The kinds come from `harness_core::Error` variants, not prose
matching: `Locked { holder }`, `Stale { subject, hint }` (`#[error("{subject} is stale:
{hint}")]`, replacing the six `bail!` sites and the new `promote` refusals), `Awaiting { path,
attempt: Option<String> }` (`#[error("awaiting response: {path}")]` — byte-identical to
today's prose; raised at the adapter with `attempt: None`, filled in by `Run::call_failed`
where the record is in scope, so migrate and gen-driver both carry the id; triage stays
`None`), `Interrupted`. The four `contains("awaiting response")` matchers become typed
matches; one `fn error_kind(&anyhow::Error) -> &'static str` in the CLI maps them.

**Wiring.** harness-cli gets a process-global `Reporter` (`OnceLock`): `out()` becomes
`report::line(text)` (human → stdout as now; json → `message`), plus `report::event(&impl
Serialize)`. Per-turn events need a hook where turns happen: `MigrateParams` gains
`progress: &'a dyn harness_llm::Progress` (`turn_start(unit, attempt, index, kind,
request_key)`, `turn_end(unit, attempt, index, &Turn)` called right after
`record.turns.push`; a `NoProgress` default; manual `Debug`). Verification runs (`replay`)
emit the same events for the reproduced turns. EPIPE stays ignored like today. `state
status` computes `UnitReport` (`harness_core::unit_report`, lifted from `cmd_status`) and
renders BOTH the human line (unchanged bytes) and the event from it, so the cockpit and
`harness-mcp` call the same function instead of re-implementing four hash comparisons.

**Test.** Golden NDJSON for `harness --json state status` on a fixture with a unit in each
`verdict.state`, one with a non-empty `stale` list, one `source_fresh: false`, one
contradiction, one `write_in_flight` (hold the lock, store a fresh green verdict without
flipping status); and for a scripted red-then-green migrate (turn-start/turn-end/attempt/
promote) asserting `turn-end.result` equals `record.turns[i].result` byte for byte. Every
line parsed back through `serde_json`, so a stray human line on stdout fails the test.

## 5. Order of work and gates

1. `rust-version = "1.89"`; `Error::{Locked, Stale, Awaiting, Interrupted}`; `WriterLock`
   (open protocol, holder read, tests); `BenchLock` migration; gitignore lines.
2. exec.rs cancellation (`CANCELLED`/`LIVE`, `Interrupted`, `built_with_env` shape) + tests.
3. `promote_attempt` extraction with the new tail order; `recover_promotion`;
   `migrate_preconditions`, `current_binding`, `attempts::load_pinned`; `--no-promote` /
   `promote_on_green`; `harness promote` with the refusal list; e2e tests (a staged
   `external` response gives a green attempt without a model; first-promotion crash
   recovery; each refusal).
4. `signal-hook` handler + the spawn-variant e2e.
5. `Reporter`, `Progress` hook, typed errors at their sources, `UnitReport`, events, golden
   tests; bench `lock_cases`; SCHEMAS.md (CLI contract, `promote`, `promote_on_green`, the
   `ruharness-events` schema, writer table, promotion protocol) and README.

Gates as always: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --workspace`, `bench check --suite targets/tractor --replay --jobs 6` unchanged.

## R. Design review — 15 confirmed findings and their resolutions

Four lenses (concurrency, crash consistency & signals, contract & consumers, dependencies &
security), each finding attacked by an independent verifier against the code; 15 confirmed,
0 refuted. Section references are to the revised text above.

| id | finding (short) | resolution |
|---|---|---|
| CONC-H1 | lock-free readers hit real non-atomic verdict+status pairs (verify, promotion, driver promotion) and would report `contradiction`; marker dirs are not an in-flight signal | §1 "What a lock-free reader can see": `holder()` is the signal; two-phase detection with `write-in-flight`; the false marker rule is dropped |
| CONC-M1 | a try-lock `holder()` probe takes the exclusive lock for microseconds and makes a real writer fail-fast naming a dead pid | §1 "Readers never lock": holder is a pure read; Drop truncates before close; acquire re-reads once on WouldBlock |
| CONC-M2 | per-case locks inside bench workers surface contention only after the whole suite ran; `check`'s two passes could straddle a promotion | §1 "Bench": all selected cases locked up front, one hold across both passes; writer-table rows for build scratch |
| sig-1 | group kill and exit are not atomic: a SIGKILLed child reaped by the poll is journaled as evidence; spawn-to-insert and spawn-after-loop windows | §3: `CANCELLED` state observed at the spawn choke point, `Interrupted` never a `ChildEnd`, handler holds `LIVE` through exit, `built_with_env` outer/inner result |
| crash-1 | recovery keyed on `.prev` misses a FIRST promotion and every window after `.prev` removal; W0 leaves markers forever | §2: one marker for the whole protocol, idempotent tail order, `recover_promotion` by evidence |
| sig-2 | `process::exit(130)` is a normal exit: interactive bash/zsh continue a loop over units after Ctrl-C; parents see no signal; SIGHUP unhandled | §3: re-raise via `emulate_default_handler`, SIGHUP registered, contract says "dies by the signal" |
| CONTRACT-1 | `promote` never binds the record to the current inputs or the unit; path-unsafe id; `stage` check wrong for migrate records | §2 refusals 1, 2, 3, 5; `load_pinned`, `current_binding` |
| CONTRACT-2 | R6 driver-freshness gate missing from `promote`; driver re-generated between record and promote breaks provenance | §2 refusals 4 and 5 (`migrate_preconditions`); `verify`'s missing R6 recorded as a follow-up question |
| CONTRACT-4 | `--no-promote` is per-invocation; an `external` resume without the flag promotes what the cockpit was holding | §2: `promote_on_green`, precedence, resume command echoed, promotion reason, clients always pass the flag |
| EVENTS-1 | `turn-end.result` invented `red`; `check` field has no source; turns lacked `unit` | §4: the Turn verbatim, `check` dropped, `unit` on turn events |
| EVENTS-2 | `awaiting`/`stale` are prose matched by substring; attempt id not in the CLI's hands | §4 typed errors; `call_failed` fills the id |
| EVENTS-3 | the `unit` event collapsed an orthogonal report into one string, frozen on ship | §4 `UnitReport` in harness-core rendering both surfaces; `facts` event; golden covers every state |
| DEP-1 | fd-lock superseded by std `File::try_lock` (1.89); its guard borrows the lock so the owned RAII type is unbuildable; lockfile cost understated (+5) | §0/§1: std lock, MSRV 1.89, fd-lock dropped; signal-hook stands (std has no signal API) |
| SEC-1 | the holder write follows symlinks into target-owned space (`migration/.lock -> plan.toml`) | §1 "Open protocol"; parent must be a real dir; bounded printable holder echo |
| SEC-2 | (as CONTRACT-1/2) plus: promote's guarantee is "verified in place", provenance beyond the digest is replay's | §2 refusal list and the stated guarantee |

**Code review (2026-09-24, four lenses, 11 confirmed / 0 refuted), resolved in the fix pass:**
SIG-H1/SIG-1 — the handler's courtesy output could panic (closed stderr) or block (full
`--json` stdout pipe) before the re-raise, leaving an unkillable process: helper thread +
250 ms budget, `writeln!` never `eprintln!`. SIG-M1 — `Signals::new` overrides an inherited
`SIG_IGN`, so `nohup harness … &` died on hangup: SIGHUP registered only when the output is a
terminal. EVT-H1 — `run_triage` re-stringified the typed `Awaiting`, so `observe` lost its
`awaiting` event and hint: the variant is carried through the fold. LOCK-READ-1/STAT-1 — a
dead holder's line (every signal death) turned real contradictions into `write in flight`:
liveness probe in `unit_report`. PROMO-M1/CONS-2 — a rolled-back promotion emitted a
`verdict` line naming a file it never wrote: `check` lines only. CONS-1 — `migrate` emitted
no `check`/`verdict` for its final judged turn (the cockpit's `--no-promote` mode): the
judge's stored verdict is carried out of the trajectory and reported with its
`attempt-verdict.json` path. SIG-M2 — the cancellation e2e observed nothing (a warm verify's
children exit on their own): a spinning driver, the `drv_c` child identified by name, alive
asserted before the kill. LOCK-TEST-1 — the promised status golden and the hard-link refusal
were untested: contradiction, write-in-flight (lock held in-process), stale list, hard link,
stalled stdout, ignored SIGHUP and the typed `observe` hand-off are pinned.

**Follow-ups recorded, not in this milestone:** `verify` lacks the R6 gate (contract-visible;
decide separately); a cockpit Accept on a driver attempt (drivers' own promotion path);
queueing on contention; async client for cooperative cancellation.

## 6. Open questions carried into implementation

- Q4 `check` event granularity: one per oracle check, detail uncapped (the boundary
  check's detail is a few hundred bytes; the events stream is not a canonical file).
- Q5 model-derived build stderr is not streamed; the ledger has it.
