# The cockpit as a friendly wrapper — design

Status: DESIGN, reviewed (2026-09-24). **Build A is built (2026-09-25)**; its code review is
in DECISIONS.md. Build B (the mouse) is next.

This design implements docs/TUI-DESIGN.md §9, "Suggested order" 1: the user's direction,
recorded in DECISIONS.md "Direction change (user)". It went through two review rounds:
- **Adversarial design review.** Four lenses: usability, safety & provenance, engine reuse,
  scope. 46 findings; the verifiers confirmed 43, partly confirmed 3 and refuted none (§R).
- **Check of the revision.** 13 more findings, 3 of them high (§R2). A second, scoped check
  confirmed all 13 resolutions against the code and found one wording slip, fixed.

Every resolution is already in the text below.

Sources:
- the §15 spike (DECISIONS.md "Cockpit wrapper: §15 spike");
- docs/TUI-DESIGN.md §2–§6, the engine this builds on — every rule there holds unless this
  document says otherwise;
- docs/MCP-DESIGN.md §0/§4: what chat may record, and the read preflight;
- docs/SCHEMAS.md "The events stream".

## 0. What it is, and is not

The same cockpit, made for people who do not use vim-style keys. It has one navigator, one
display and one activity panel.

**The navigator** is a tree of the target's C files. Under each file are its functions. Under
each unit are its crate and its attempts. The user moves through it with the arrow keys; the
mouse comes in Build B.

**The display** is the View on the right. It shows whatever is selected.

**`Enter`** opens a short menu of what can be done with the selection in its current state. A
chosen action goes through four steps:
1. It is confirmed in words.
2. It runs behind the scenes as the same spawned `harness --json …` command as today.
3. It reports in plain language in the activity panel, which is always visible.
4. Its exact command is one keypress away.

**What stays** from TUI-DESIGN §2–§6:
- The ledger is the truth. The cockpit never writes it.
- Every write is a spawned `harness --json` command, and its argv is shown before it runs.
- Nothing runs without an armed confirmation. Typed-ahead, pasted and auto-repeated input
  never confirm.
- One command at a time.
- The ledger is re-read after the child is reaped, on `g`, and on the 2 s tick while a command
  runs or a hand-off is outstanding. There is no idle watcher.
- Cancel sends SIGINT to the child's process group.
- **A hand edit is never lost.**
- The cockpit's own signal path.
- The provenance labels keep their meaning.
- Every untrusted string passes the display filter.

**Two builds, each reviewed on its own:**
- **Build A** is the keyboard-complete navigator. It starts by fixing the bugs the review found
  in the cockpit as shipped.
- **Build B** is the mouse.

**Not in this milestone:**
- The chat pane. §10 reserves its screen space and nothing more.
- Any model-driven action the cockpit does not have today: a fresh migration, driver
  generation, triage.
- A findings view, an idle watcher, a find filter, and a guided "record an outside edit" flow.
- New CLI subcommands, changes to the CLI's contract, and new crates.

## 1. The screen

The mock below is 120 columns wide:

```
┌ Files ───────────────────────┐┌ lib.c · migrated (u-lib, pipeline) ──────────────────────────────────────────────────┐
│▾ read_scalefactors_lib  ✓1/1 ││ read_scalefactors   C  src/lib.c:17  ⇄  Rust  src/ffi.rs:11  → logic::read_scalefactor│
│  ▾ test_case/                ││17 void read_scalefactors(bs_t *bs, uint› │ 11 pub unsafe extern "C" fn read_scalefa› │
│    ▾ include/                ││18                        int bands, flo› │ 12     bs: *mut BsT,                      │
│        · lib.h         header││…                                         │ …                                         │
│    ▾ src/                    ││                                                                                      │
│      ▸ ✓ lib.c       migrated││ Checks  ✓ same exports  ✓ allowed calls  ✓ driver shape  ✓ the Rust builds           │
│  ▾ Units (1)                 ││         ✓ same outputs as C  ✓ whole program  ✓ sanitizers  ✓ boundary calls         │
│    ▸ ✓ u-lib         migrated││ Crate from attempt a-13c9 (pipeline) · verdict green, fresh                          │
└──────────────────────────────┘└──────────────────────────────────────────────────────────────────────────────────────┘
 Ready. Last: Re-check u-lib — GREEN, all 8 checks passed (41 s).                                           [Details c]
 plan changed 1 unit — review `git diff migration/plan.toml` (c for the lines)
 ↑↓ move  ←→ fold/open  Enter actions  Tab pane  Esc back  c details  g re-read  ? help  q quit
```

**Files** is the pane on the left.
- It is 32 columns wide at 120 terminal columns or more, and 24 at 80–119.
- A long name ends in `…`.
- A row shows its state word at the right edge when the word fits. The selected row always
  shows it, and the View's title repeats it.

**View** is the pane on the right.
- The C and the Rust sit side by side when the View is at least 78 columns wide. Below that
  they stack, C first (today's narrow renderer).
- A code line that is cut ends with a dim `›`.
- `←` and `→` scroll it horizontally.

**Activity** is two rows, visible at every width.
- Row 1 is the status: a spinner, the action, the current step, the elapsed time,
  `[Cancel x]` and `[Details c]`. When idle it reads "Ready. Last: …", followed by
  `[Try again t]` after a `locked` refusal.
- Row 2 holds notices (§6.2).
- `c` shows the details — today's run panel — over the lower half of the screen, or over the
  whole screen below 80 columns.

**The hint bar** is the last row. It describes the focused pane, and a notice never replaces it.

**Below 80 columns** one pane shows at a time, Files or View; `Tab` switches between them.
Activity and the hint bar stay. `--layout split|stacked` keeps its meaning for the pairs.

**Chat.** The right side is reserved at 150 columns or more (§10). Nothing is drawn there.

**Focus.** The focused pane has a bold border and a reversed title.

## 2. The file tree

### 2.1 What is listed

The tree lists the files the scanner reads. Both use one walk, which moves into harness-core
as a language-neutral function:

`walk::confined(dir, exts, limits) -> Walk { files, skipped, errors, truncated }`

How it walks:
- Each entry is canonicalized only to check containment and cycles.
- An entry that resolves outside `dir` is skipped, and so is a directory already walked. This
  is the scanner's rule today.
- The paths returned have the same form as the scanner's today: joined under `dir`, not
  canonical.
- Non-regular files (FIFOs, devices) go to `skipped` and are never returned.
- Errors are returned as data.

**harness-scan** calls it with its own extensions (`*.c` and `*.h`, which stay in the frontend)
and no limits.
- Walk errors stay fatal, as today.
- Skipped files are reported as a message. This is new: a FIFO named `a.c` no longer hangs a
  scan.
- The scanner's tests stay in harness-scan, including its symlink test.
- facts.jsonl is proven byte-identical on zopfli, on the read_scalefactors case, and on a
  synthetic copy that holds a symlink inside `source_dir`.

**The tree** calls it with limits: 20 000 files and depth 32. The limits bound the listing, not
a full count. Errors, skipped entries and truncation show as rows, for example "… more files
not listed (limit 20 000)".

The walk reads directory entries and `lstat` only, never file contents. It runs outside
`Snapshot::load`, on the worker thread that loads snapshots (§6.3), so harness-mcp's preflight
contract is unchanged.

A file the facts record but the walk did not return is checked with `lstat`:
- absent → *missing*;
- present, but beyond the limit → *not listed*.

A directory is listed only when it holds a listed file. File names are untrusted: they pass
the display filter.

### 2.2 Nodes and selection

The selection is one of:

`Project | Dir(path) | File(path) | Function(file, symbol) | Unit(id) | Crate(id) |
Attempt(id, attempt)`

It is keyed by path and id, never by row index.

The tree, from the top:
- The project is the root. It starts expanded and selected.
- Under it: directories, then files. Under a file: its functions, taken from the facts' symbol
  records in span order.
- After the source tree: a group **Units (n)**. Under each unit: its **crate** node, then its
  attempts in the rail's defined order (TUI-DESIGN §2).

A reload keeps the selection. If the selected node vanished, the selection moves to its parent.

A jump ("Open unit u-…", a link in the project summary) pushes the current selection onto a back
stack. `Esc` in Files (or `Backspace`) goes back.

The tree is flattened into rows only when the snapshot, the expansion or the layout changes.
Only visible rows are rendered.

### 2.3 States

`harness_tui::files::build(&Snapshot, &Walk)` computes the states. It is pure and tested with
fixtures. Its inputs are explicit, and no file is hashed twice:
- the walk;
- the snapshot, with two additive fields:
  - `FactsState.stale_paths`: files whose hash differs. `Snapshot::load` already computes this.
  - `UnitView.crate_digest`: the unit crate's content hash. It is computed today and dropped.

Every state has a glyph AND a word. Colour is never the only signal.

**File states.** The first matching rule wins.

| # | word | glyph | rule |
|---|---|---|---|
| 1 | missing | `?` | a facts record whose file is absent |
| 2 | changed since scan | `!` | in `stale_paths` (headers too) |
| 3 | not scanned yet | `+` | listed, with no facts record (or no facts at all) |
| 4 | header | `·` | a scanned `.h` |
| 5 | no exported functions | `–` | a scanned `.c` with no public symbol. The planner never plans such a file, so no plan is offered |
| 6 | not in the plan | `○` | a scanned `.c` with a public symbol that no non-blocked unit's `files` holds, or there is no plan |

A file owned by a unit takes its owner's state. The owner is the non-blocked unit whose `files`
hold the file (plan pass 2 keeps a blocked unit's old `files`).

**Unit states.** Also first match wins.

| # | word | glyph | rule |
|---|---|---|---|
| 7 | needs attention | `⚠` | a cause the user can fix, each named in the View with its next step: a contradiction, an interrupted promotion ("the next command recovers it"), a stale verdict ("Re-check"), or **changed outside the harness** — the crate matches neither a recorded attempt's candidate nor what the oracle last judged ("restore it, or record it with `harness override`; see Help") |
| 8 | failing | `✗` | the current verdict is red. This includes a verified unit that `verify` demoted to `in-progress` |
| 9 | migrated | `✓` | `verified` or `merged`, with a green, fresh verdict and provenance pipeline, steered or human (tagged `steered` / `by hand`) |
| 10 | verified, origin not recorded | `✓?` | `verified` or `merged`, green and fresh, with provenance None or Ambiguous. Nothing to fix: the crate's code was judged, but no attempt bound to the current inputs produced it. zopfli's u001 is an example: the M0 layout, and the source moved on. Not counted in the migrated share |
| 11 | tried | `◐` | `pending`, with at least one attempt (red, green but not accepted, or unfinished) |
| 12 | planned | `◇` | `pending`, with no attempt |
| — | the unit's status word | | any other combination: the fallback, e.g. a verified unit whose verdict is missing |

Where the other states show:
- A unit row shows its own state.
- `⊘ blocked` shows on unit rows only.
- A function named in its unit's `symbols` takes the unit's state. Any other function is
  **internal**: dim, with no glyph.

**Rollups.** A directory and the project show non-zero counts of the action states and the
migrated share, for example `✗2 ⚠1 ✓3/11 (+1 ✓?)`.

**Busy** is not a file state. A live holder of the writer lock is shown at project level as
"busy: `<holder command>`", and every spawning item is greyed with it. The holder could be the
CLI, another cockpit, or an act from chat. It comes from harness-core's `status::live_holder`,
which becomes public. The cockpit never re-implements the pid check and never guesses a unit
from the holder's text.

**Hand-offs.** An attempt this cockpit posed a hand-off for shows "waiting for your answer" on
its row. Other runs' hand-offs appear as unfinished attempts (state 11).

**Hand-rolled, not `tui-tree-widget`.** The crate is vetted and viable, but a flat row list is
what the cockpit already renders and tests. Revisit if the tree passes about 400 lines or needs
drag or multi-select.

## 3. The View: what the selection shows

| selection | the View shows |
|---|---|
| project | the summary (below) |
| directory | its files and their states |
| file owned by a unit | the pairs of its public functions (the unit screen filtered to this file's symbols), the unit's checks and provenance, and its internal functions by name |
| file owned by no unit, or a header | its C source, display-filtered and highlighted; at most 1 MiB, the rest cut with a note |
| public function in a unit | its C beside its Rust (today's pair view, one pair) and its unit's checks |
| internal function | its C only, noting "compared through the unit's exported functions" when its file is in a unit |
| unit | today's unit screen |
| crate | the unit crate's pairs (today's `Shown::Crate`) |
| attempt | the attempt's crate as pairs (today's `Shown::Attempt`), its turns, checks, provider and model, and its note |

**The project summary** shows:
- the facts: N files, and how many changed, are new or are missing;
- the units by state;
- the live holder of the writer lock, if any;
- the last command's outcome;
- lists of files that need action (failing, attention, changed, new), each a jump link;
- a **Next step** line, stated as a fact. The first match wins:
  1. nothing scanned → "Nothing is scanned yet — choose Scan the project";
  2. changed, new or missing files → "N files changed since the scan — Scan the project
     again";
  3. no plan → "No plan yet — Refresh the plan";
  4. a unit whose C changed since planning → "u-…'s C changed since it was planned — scan,
     refresh the plan, then review its diff";
  5. otherwise, no line.

  It never suggests model work.

**Check names** are shown in words. The raw name stays in the details:

| raw | in words |
|---|---|
| `symbol-set` | same exports |
| `capabilities` | allowed calls only |
| `driver-shape` | driver shape |
| `rust-build` | the Rust builds |
| `differential-driver` | same outputs as C |
| `whole-program:*` (by prefix) | whole program |
| `sanitizers` | sanitizers |
| `boundary` | boundary calls |
| anything else | the raw name |

Failed checks come first. The verdict overlay (`v`) and the diff overlay (`d`) stay, and `Esc`
closes them.

## 4. `Enter`: the action menu

### 4.1 How it behaves

`Enter` on a tree row opens that row's menu. Focus starts on the recommended item:
- **Open** (focus moves into the View) on a file, function, crate or attempt;
- the Next step's action on the project, if there is one; otherwise Open;
- **Open / Fold** on a directory.

The menu opens with a fresh check of the lock holder: one file, plus `live_holder`'s pid check.

Items that do not apply are not listed. Items that apply but cannot run now are greyed, with
their reason; `Enter` on a greyed item shows the full reason in the menu footer. Model-backed
items sit under a separator, "Uses a model — can take minutes". Accelerators are shown beside
their items.

Every command is project-wide or unit-wide. **No tree path ever enters an argv**, and the words
say so ("Scan the project"). File and function menus carry their owning unit's items, labelled
with the unit.

### 4.2 Items by node

Each argv follows `harness --json`, and `--target=<root>` is always attached.

| node · state | items |
|---|---|
| any | **Open** · **Re-read the project** (`g`) |
| project; a file in states 1–3 | **Scan the project** — `scan` |
| project; a file in state 6; a unit whose C changed | **Refresh the plan** — `plan` |
| project, file | **Find hazards (run the detectors)** — `detect` |
| owned file or function | **Open unit u-…** (a jump) and the unit's items below, labelled "… u-…" |
| unit, crate, owned file or function | **Show the checks** (`v`) · **Re-check with the oracle** — `verify <unit>` [`--allow-unsandboxed`] · **Accept a-… into u-…** when exactly one attempt is green, bound and not promoted; otherwise **Choose an attempt to accept…** (jumps to the unit's attempts) |
| crate, attempt | **Hand edit** (`e`) — TUI-DESIGN §4 |
| attempt | **Accept** (`a`), worded "Replace u-…'s verified crate with a-…" when `--replace` applies · **Compare with the promoted attempt** (`d`, when TUI-DESIGN §3 enables it). Then, under the model separator: **Modify with a note** (`m`), **Retry** (`r`), **Resume** (`R`) |
| project; the kept edit's unit, crate and attempts | **Continue my kept hand edit (u-…)** (`E`) · **Discard my kept hand edit** (opens the override dialog focused on Keep) |
| project; a unit that is planned, tried or failing | greyed **Migrate — ask in chat**: "model work happens in chat, which is not built yet; see Help (`?`) for today's route" |
| any, while a command runs | **Cancel the running command** (`x`) |

### 4.3 The cockpit's own gates

These gates are the cockpit's, not the CLI's. The CLI accepts any provider and does not check
that a crate is recorded code, so the cockpit checks both — **again at confirm time, on a fresh
read**.

**Busy.**
- While this cockpit runs a command: "a command is running (one at a time)".
- While another process holds the lock: "busy: `<holder command>` (checked just now)".

**Refresh the plan, Find hazards.**
- No facts: "nothing is scanned yet — scan first".
- Stale, new or missing files: "N files changed or new since the scan — scan first".

**Re-check.** Offered when the unit has a crate whose code the harness knows:
- its content hash equals a recorded attempt's candidate digest (any binding), **or**
- its file-set hash equals the stored verdict's `rust_crate` input, i.e. code the oracle has
  already judged (this includes zopfli's u001).

Otherwise it is greyed:
- "the crate differs from every recorded attempt and from what the oracle last judged —
  restore it, or record it with `harness override` (see Help)";
- when the unit's source is stale: "the C changed since the unit was planned — scan, refresh
  the plan and review its diff first".

Verifying code nobody recorded or judged stays impossible (MCP-DESIGN §R CONTRACT-2).

**Providers.** The cockpit takes a `--provider <name>` list (repeatable; default `external`
only), as harness-mcp does.
- **Modify** always passes `--provider=<the first listed>`. The target's `harness.toml` never
  chooses the provider.
- **Retry** runs only when the record's provider is on the list. Otherwise it is greyed:
  "provider `<p>` is not allowed — start with `--provider <p>`".
- Both dialogs name the provider, and the model from the target's migrate routing, in words.
  Retry's are marked "from the attempt record".

**Retry refusals**, matching harness-mcp:
- Retry is not offered on an unseeded attempt whose provider is `external`. Its retry would
  pose a blind hand-off, and only the audited protocol may answer those.
- Retry is refused on a half-seeded record: one with only one of `seeded_from` and
  `steer_note`.

**Resume** applies only to hand-offs this cockpit posed. After the Retry refusal those are all
steer attempts.

**Modify** remembers the last note typed for each attempt — cancelled, declined or refused —
and pre-fills it.

**Try again** (`t`), after a `locked` refusal or a failed start, re-opens the same dialog with
the same argv. The dialog arms like any other.

**Plan approval** is unchanged: the plan is a file the user reviews and commits. "Record the
crate as it is" is not offered (§R2 CHK-3); a guided flow that shows the diff and asks "did you
write this?" is for later (§15).

## 5. Confirming

The Re-check dialog, 76 columns wide:

```
┌ Re-check u-lib with the oracle? ─────────────────────────────────────────┐
│ Builds u-lib's Rust and runs it against the C in the sandbox.            │
│ Writes units/u-lib/oracle-latest.json and .md (and oracle-last-green.json│
│ when green) and u-lib's status in plan.toml: green → verified; red → a   │
│ verified unit becomes "in progress" (a merged one keeps its status).     │
│ Changes no code. The crate is unchanged since the cockpit last showed it.│
│                                                                          │
│ Command: harness --json verify u-lib --target=/…/read_scalefactors_lib   │
│                                                                          │
│        [ Cancel  Esc ]   [ Run  y ]      ready: → then Enter, or y       │
└──────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Wording

The title is a question naming the object. The body names every file the act writes and what
changes:

| act | what the body says it writes and changes |
|---|---|
| Scan | rewrites facts.jsonl; a changed C file makes verdicts stale |
| Refresh the plan | rewrites plan.toml: re-approves the changed sources of every unit whose C changed, verified units included; adds and removes units; blocks units whose files left. Review `git diff migration/plan.toml` afterwards |
| Find hazards | rewrites the observer findings |
| Re-check | as in the dialog above |
| Accept | replaces the unit crate with the attempt's candidate and verifies it in place; writes the oracle files and the unit's status in plan.toml, and marks the attempt promoted; the replace wording when it applies |
| Modify, Retry | a model call, naming the provider and model; records a new attempt; can take minutes |
| Hand edit | records a human attempt judged by the oracle; never promotes |

`--allow-unsandboxed`, when present, is named in words: "runs code WITHOUT the sandbox". No
duration is promised.

Then comes the exact argv. It is always shown whole — wrapped, never cut — and a long one must
be scrolled to its end.

### 5.2 Buttons

| dialog | buttons, with their keys |
|---|---|
| act | `[Cancel Esc]` `[Run y]` |
| override (a hand edit) | `[Keep for later Esc]` `[Record y]` `[Discard D]` |
| quit, while a command runs | `[Stay Esc]` `[Quit, let it finish q]` `[Stop it and quit x]` |
| cancel (`x`, `[Cancel]`) | `[Keep running Esc]` `[Stop it x]` |

Focus starts on the first button, the safe one, in every dialog.

### 5.3 Arming

A dialog **arms** once three things hold:
1. It is drawn whole, with its argv seen to the end.
2. At least **300 ms** have passed since the last input event was *read*.
3. `event::poll(0)` finds nothing pending at that moment (today's check, kept).

Arming is a latch: once armed, the dialog stays armed.

Before the dialog arms:
- **Scroll keys** scroll and restart the 300 ms, because the argv must be seen to its end.
- **`Esc` and `n`** cancel at any time.
- **Every other key is dropped**, never queued, and restarts the 300 ms. That includes
  `Enter`, `y`, the letter keys and the focus moves.

After it arms, the non-safe buttons can be reached in two ways:
- a button's letter key acts;
- a focus move (`←`, `→`, `Tab`) followed by `Enter` activates the focused button.

`Enter` on the safe button takes the safe choice.

So a held `Enter` is harmless. Auto-repeat arrives as `Press`, because no keyboard-enhancement
flags are enabled. Before arming, the repeats are dropped and keep restarting the wait. After
arming, `Enter` sits on the safe button, and reaching another one takes a separate move key.

**The state is visible:**
- Run (Record, Quit, Stop) is dim with "reading…" until armed.
- "↓ more below — scroll to the end" shows while the argv is unseen.
- A dropped key shows "Too soon — wait for ready" in the dialog.
- Once armed, the dialog shows "ready: → then Enter, or y" (with the dialog's own key).

`q`, `Q` and `Ctrl-C` open the quit dialog while a command runs. With no command running, `q`
and `Ctrl-C` quit at once. No key cancels or quits without arming. The quit prompt no longer
takes an unarmed `y`.

The logic lives in `app`. `main` passes it the clock and the pending-input flag, so it can be
tested.

## 6. The activity panel

### 6.1 Narration

Row 1 narrates the running command:
- a spinner, the action, the **current step**, and the elapsed time since the run started (the
  start is stored in `RunPanel`);
- afterwards, "Ready. Last: …".

`c` toggles the details: today's run panel verbatim — the argv, every event line, the stderr
tail, the exit.

A stateful `Narrator` is fed the run's argv and every event:

| input | narrated as |
|---|---|
| start of `verify` / `promote` | "Running the oracle… (the checks are reported when it finishes)" |
| `turn-start` | "Turn N: asking the model", plus " (`<model>`)" when the argv carries `--model=`, plus " for a translation", " to repair it" or " to apply your note" (other kinds: the raw word) |
| `turn-end` | "Turn N: " plus the result in words — `green` passed · `format` the reply was not in the expected form · `check` a check failed · `build` the Rust did not build · `oracle` the outputs differ from C · `crash-timeout` it crashed or timed out · `truncated` the reply was cut off · `blocked` the safety scan refused it · anything else: the raw word |
| `check` | "Checked: same outputs as C — passed" / "— FAILED" |
| `verdict` | "Verdict: GREEN — all N checks passed" / "RED — K of N checks failed: <words>", counted from this run's `check` events |
| `attempt` | "Recorded attempt a-…: <outcome>" |
| `promote` | "Promoted a-… into u-…: verified" / "rolled back — the crate is unchanged" |
| `awaiting`, on a steer attempt | "Paused: waiting for the answer to the hand-off (request next to <path>). Write the answer, then choose Resume." |
| `awaiting`, on an unseeded attempt | a guard; the cockpit's own acts cannot pose one: "Paused: a BLIND hand-off — only the audited protocol (targets/tractor/handoff-tools) may answer it; an answer written by hand is recorded as pipeline output." |
| `error` | `locked` → "Another command is changing this project (<holder>). Nothing was done." and `[Try again t]` · `stale` → "Out of date: <message>" · `awaiting` → folded into the pause · `interrupted` → "Stopped." · anything else → "The harness reported: <message>" |
| `message` | the line itself (scan, plan and detect speak only through these) |
| exit | 0 → "Done" · 10 → "Finished — red" · 1 after `awaiting` → "Paused" · 1 otherwise → "Refused: <last message>" · a signal → "Stopped (<signal>)" |

Every narrated value is filtered for display. Browsing stays free while a command runs; only
spawning is blocked.

### 6.2 Notices

Row 2 holds the notices. They are the cockpit's one-line messages: refused accelerators, note
errors, "hand edit kept", and about 28 like them.
- A notice clears on the next key or after 8 s.
- After `plan`, one summary line stays until the next command starts: "plan changed N units —
  review `git diff migration/plan.toml` (c for the lines)". A transient notice covers it for a
  while, and then it returns.

### 6.3 Freshness and loading

**Loads run on a worker thread.** One load runs at a time, and the latest request wins. The
worker runs harness-mcp's read preflight, which moves into the library, and then
`Snapshot::load` and the walk. The UI keeps answering keys meanwhile, including `x`, `q` and
`Ctrl-C`; the Files title says "reading…".
- A failed load keeps the last snapshot and shows "unreadable: <reason>" once.
- At start, an unreadable target is refused in words.

The rule that the ledger is re-read only after a child is reaped still holds: the reap is what
sends the load request.

**The pairs cache key** gains:
- the shown unit's `source_fresh`;
- the `stale_paths` among its files;
- `crate_digest`.

After a reload, an outside C edit shows "facts predate <file> — run `harness scan`"
(TUI-DESIGN §2). An outside Rust edit shows the new code.

**Unchanged since shown.** The rendered pairs remember the `crate_digest` they were built from.
When the Re-check dialog is confirmed, the cockpit recomputes the digest. If it differs, the
dialog refuses: "the crate changed on disk since the cockpit showed it — press g and look
again". The View shows each shim and its logic function, not every line of the crate, so the
claim is exactly this — "unchanged since shown" — and no more.

## 7. Mouse (Build B)

**Modes.** Only button tracking with SGR coordinates (`CSI ?1000h`, `CSI ?1006h`), enabled
through a small crossterm `Command` of our own. crossterm's `EnableMouseCapture` also sets
`?1003h` (every movement) and would flood the loop.
- It is disabled with `DisableMouseCapture`, which resets all five modes, through the terminal
  guard (§11) on every path: quit, error, the panic hook, the signal path, and `suspend` before
  the editor.
- It is re-enabled in `resume` unless the cockpit is dying.
- Mouse events outside the frame are ignored.
- Terminals without SGR support are documented as `--no-mouse` cases.

**Hit testing** uses the last frame. The view records each clickable region as it draws: a
tree row's node, a pane, a menu item, a button, a hint-bar entry, the dialog. That API exists
from Build A.

**Gestures:**
- **Left click** focuses the pane and selects the row.
- **Double click** means `Enter`: two presses on the same row within 400 ms, timed by the
  cockpit. A pair is discarded when the loop iteration between them took longer than the
  window.
- **The wheel** scrolls an open menu or dialog; otherwise it scrolls the pane under the pointer.
- **Hint-bar entries and buttons** do what their key does. Quit opens the quit dialog.
- **A click outside a menu** closes it. A click outside a dialog does nothing.
- **A click on a dialog button** acts only once the dialog is armed. It is a deliberate press
  at a position, so no prior move is needed.

**Selecting text** belongs to the terminal. A drag — a press and a release on different cells —
shows "To select text, hold Option (Terminal, iTerm2) or Shift (most others)". Help has a
"Mouse on/off" item, and `--no-mouse` starts with the mouse off. tmux needs `set -g mouse on`
and is best effort.

## 8. Keys

| key | does |
|---|---|
| `↑` `↓` | move in the focused pane (tree: a row; View and details: scroll) |
| `←` `→` | in the tree: `←` folds, or on a leaf goes to the parent; `→` unfolds, or on a leaf moves into the View. In the View: horizontal scroll, and `←` at column 0 returns to Files |
| `Enter` | tree: the action menu · menu: choose · dialog: the focused button (§5.3) |
| `Esc` | close a menu, dialog, overlay or the details; View: back to Files; Files: back along the back stack |
| `Backspace` | back along the back stack |
| `Tab` `Shift-Tab` | next / previous pane (below 80 columns: Files ⇄ View) |
| `PgUp` `PgDn` `Home` `End` | page, or go to the ends, in the focused pane |
| `?` `F1` | help: every key, the states' legend, the mouse notes, today's routes for model work |
| `c` | activity details |
| `g` | re-read the project |
| `t` | try again, after a refusal |
| `q` | quit (a dialog while a command runs) |

**Accelerators.** `a m e E r R x d v` act on the **selection** and open the same dialogs as the
menu. `D` exists only inside the override dialog. `Q` is `q`. `j` and `k` alias the arrows,
`]f` and `[f` go to the next or previous pair in the View, and `J` and `K` go to the next or
previous unit node. Nothing requires any of them.

**The hint bar** lists the focused pane's keys in priority order. When the width runs out it
drops whole entries, never cutting one. `? help` and `q quit` are always last and never dropped.

Letters typed into a note input are text.

## 9. Empty states and first run

The project is selected at start. Its View leads with the Next step: "Nothing is scanned yet —
press Enter and choose Scan the project". The menu opens focused on that item.

Help opens with three lines:
- move with the arrows (in Build B, or the mouse);
- `Enter` for what you can do;
- `?` for this screen.

A target without `harness.toml` is refused at start, in words: "<dir> is not a harness target
(no harness.toml); start with `--target <target dir>`". This replaces today's raw io error.

## 10. The chat pane (later)

At 150 columns or more, the right side is reserved. This design decides none of the following;
they are the chat spike's questions (TUI-DESIGN §9):
- how chat's acts reach the activity panel and the cockpit's confirmation;
- how a chat-requested act is labelled (the requester label, MCP-DESIGN §7);
- where chat sits in the focus order.

Until then, the greyed "Migrate — ask in chat" items point to Help, which gives today's routes:
- `harness migrate <unit>` — the blind hand-off, or a live provider — for a fresh translation;
- harness-mcp, in a separate Claude Code session, for steer attempts (README);
- `harness override <unit> <dir>` to record an outside edit as a hand edit.

## 11. Engine reuse: what changes in the code

| part | change |
|---|---|
| `model` (library) | two additive fields, `FactsState.stale_paths` and `UnitView.crate_digest`; otherwise unchanged |
| `harness_tui::files` (library, new) | `build(&Snapshot, &Walk)`: the states, the rollups, file → units and file → symbols, and the "known code" test for Re-check. Pure |
| `harness_tui::preflight` (library, moved) | harness-mcp's read preflight (MCP-DESIGN §4; `policy::preflight` uses nothing beyond std), one implementation. harness-mcp calls it as before; the cockpit's worker calls it before each load |
| `pairs`, `display`, `events`, `spawn` | unchanged |
| harness-core | `walk::confined` (new); `status::live_holder` made public; nothing else |
| harness-scan | calls `walk::confined`; its tests stay; facts proven byte-identical (§2.1) |
| harness-cli | nothing. `verify`'s R6 gate is decided separately (§15) |
| harness-mcp | calls the moved preflight; no change in behaviour |
| `app` | see below |
| `view` | the new layout: tree rows, the menu, the dialogs, the two activity rows, the hint bar, the hit record, horizontal scroll with cut marks. The pair renderer, highlighting and the verdict and diff overlays are kept |
| `main` | the worker thread for loads, and the terminal guard (below) |
| crates | none new |

**`app`:**
- `Selection` replaces `unit`, `rail` and `shown`. Those are derived from it, so the acts read
  the same values as before.
- `Focus` becomes Files, View or Activity.
- `Mode` gains `Menu`. `Confirm` gains buttons, focus and the arming latch. Quit and cancel
  become confirm variants.
- `Act` gains `Scan`, `Plan`, `Detect` and `Verify`.
- Today's argv builders stay. Added: Modify's `--provider`, and Retry's refusals and allowlist.
- The hand-edit lifecycle stays. `E` and Discard become menu items and a button.
- Also new: the `Narrator`, the per-attempt note memory, `[Try again]`, the back stack, the
  confirm-time re-checks (§4.3), and the clock and pending-input flag passed in from `main`.

**The terminal guard** (`main`). The restore paths never block:
1. The signal path and the panic hook first set an atomic `dying` flag.
2. They restore the terminal unconditionally; the restore is idempotent.
3. The signal path then waits at most 200 ms (`try_lock` in a loop) for an enable that is in
   flight — init, resume, or the mouse in Build B.
4. It restores once more, then dies.

Enables take the guard's mutex and do nothing once `dying` is set. The panic hook never takes
the mutex, so a panic inside `resume` cannot deadlock.

A changed hand edit joins the kept list that the signal path prints **before** staging and
resume.

## 12. Safety and provenance: what reviewers must be able to check

1. **Confirmation.** No act, quit or cancel happens without an armed confirmation (§5). The
   keyboard, paste, auto-repeat and the mouse all go through the same latch. Focus starts on the
   safe button. A non-safe button needs its letter, or a move plus `Enter`, after arming.
2. **Untrusted strings.** Every string from the target passes the display filter on screen. The
   only target-derived values that reach an argv are:
   - unit and attempt ids;
   - for Retry and Resume, the record's provider, model and note.

   The provider must be on the cockpit's `--provider` list. Modify's provider comes from that
   list, never from the target. The dialogs name the provider and model in words. No tree path
   enters an argv.
3. **Model work.** It is reachable only through Modify, Retry and Resume:
   - Retry never runs for an unseeded `external` attempt, nor for a half-seeded record;
   - Resume runs only for this cockpit's own hand-offs, which are all seeded.

   No fresh migration is offered.
4. **Provenance.** The ledger records no provenance; it is computed from the attempts.
   - Re-check runs only on code the harness knows: a recorded candidate, or what the oracle
     last judged.
   - Unrecorded code shows `⚠ changed outside the harness`.
   - Verified code with no attributable attempt shows `✓? origin not recorded`, which is not
     counted in the migrated share.
   - These gates are the cockpit's own, and they are re-evaluated at confirm time on a fresh
     read.
5. **The terminal** is restored on every path through the non-blocking guard, and this is
   tested under a pty (§13).
6. **Reads are bounded.**
   - The preflight runs before every load, off the UI thread.
   - The walk reads entries and `lstat` only, within its limits.
   - There is no idle watcher.
7. **Known gaps outside this milestone** (§15):
   - the crate hash does not cover files outside `src/`;
   - harness-detect's walk follows symlinks out of `source_dir`;
   - `verify` has no R6 gate.

## 13. Tests

**Walk** (core):
- the limits;
- errors returned as data;
- a symlink out of the directory; a cycle;
- a FIFO and a device, both skipped;
- the path form matches the scanner's.

**Scanner:** facts.jsonl is byte-identical on zopfli, on read_scalefactors, and on a synthetic
copy with an internal symlink. The existing symlink test still passes.

**Files:**
- zopfli: u001 shows `✓?` and allows Re-check through the verdict's `rust_crate`; 10 units are
  planned; a 3-file unit; headers.
- the tractor case: `✓`.
- synthetic copies:
  - a new file, an edited `.c`, an edited header, a deleted file;
  - a static-only `.c`, an unowned `.c`;
  - no facts, no plan;
  - a red verdict with `in-progress` status;
  - a pending unit with red attempts;
  - a blocked unit whose file is also owned by a new unit;
  - a merged unit with provenance None;
  - a crate edited outside the harness;
  - more than 20 000 files;
  - a control character in a file name.

  There is one test per rule, plus these conflicts: a header edit together with a stale
  verdict; a verified unit with provenance None.

**Menus:** table-driven over each `Selection` and state — the items, their argv, and the greyed
reasons. Properties:
- no tree path appears in any argv;
- a greyed item never spawns;
- Retry is absent for an unseeded `external` attempt and refused for a half-seeded record;
- Retry is greyed for a provider not on the list;
- Modify always carries `--provider=`;
- Re-check is greyed on unknown code;
- the busy state is re-read when the menu opens and at confirm.

**Confirm and arming** (with an injected clock):
- a key before arming is dropped and restarts the wait;
- pending input blocks arming;
- the latch holds once armed;
- one press, then 600 ms of silence, then repeats every 30 ms: nothing runs;
- after arming, `Enter` on the safe button takes the safe choice;
- after arming, a move followed by `Enter`, or the letter, runs;
- `D`, `q` and `x` inside their dialogs need arming;
- `Q` and `Ctrl-C` never act directly;
- goldens of a dialog before and after arming.

**Narrator:** the recorded fixtures, plus new ones recorded from the CLI's end-to-end runs:
verify (green and red), plan, detect, a `stale` refusal, awaiting.

**Freshness:**
- an outside C edit followed by a reload shows the stale label;
- an outside Rust edit refreshes the pairs;
- a crate edited between display and confirmation is refused;
- a slow fake load keeps the keys answered;
- a failed reload keeps the last snapshot.

**Terminal guard:**
- a panic during `resume` does not hang;
- a SIGTERM while an enable is blocked still restores and dies.

**Goldens (6):**
- 120 columns wide;
- 80 columns;
- a tree showing every glyph;
- a menu with a greyed item and its footer reason;
- the activity panel while running;
- the details.

**pty tests** (`script`):
- rewrite signals.rs for the new keys: it currently waits for "units", types `\t j \r e` and
  expects "run this?";
- a keyboard end-to-end run: Enter → Scan the project → wait for ready → `→` → Enter → "Done",
  and the facts are rewritten;
- a SIGTERM during `resume` after an edit: the shell is restored and the kept edit is announced;
- Build B: mouse-off sequences after SIGHUP, and around the editor.

**Mutation-checked rules**, named up front:
- the arming latch drops early input;
- a held `Enter` never runs anything;
- no tree path in an argv;
- greyed items never spawn;
- no unseeded `external` Retry, and no half-seeded Retry;
- Modify's provider comes from the list;
- Re-check is greyed on unknown code;
- the crate must be unchanged since shown;
- no enable after dying;
- the kept edit is announced before resume;
- (Build B) the mouse is off on every exit path.

## 14. Order of work

**Build A**

0. Fix the bugs in the cockpit as shipped. Each fix starts from a failing regression test:
   - `Q` and the quit prompt get arming (SAFE-11);
   - the non-blocking terminal guard, with the kept edit recorded before resume (SAFE-10,
     CHK-5);
   - the Retry refusals and the provider list, with Modify's `--provider` (SAFE-3, SAFE-12,
     CHK-1, CHK-13);
   - the preflight moves into the library, and the cockpit's loads move to a worker thread
     (SAFE-6, CHK-10).

   Commit.
1. The dialogs, arming and buttons (clock injected), and the menus' argv table. Test and
   mutation-check them.
2. In harness-core: `walk::confined` and the scanner's switch to it (facts byte-identical),
   `status::live_holder`, and the two model fields. Then `files` and its tests.
3. The navigator:
   - `Selection`, the tree, and the View by selection;
   - the activity rows and the `Narrator`, notices, the hint bar, help, and the empty states;
   - the hit-record API;
   - goldens, the rewrite of signals.rs, and the keyboard pty end-to-end test.
4. Update README and TUI-DESIGN §3. Then an adversarial code review, a fix pass, a verification
   of the fix pass, mutation checks, the DECISIONS handoff, commit and push.

**Build B**

1. Mouse modes through the guard, starting with their pty restore test.
2. Gestures, and the help toggle.
3. Review, fixes, verification, push.

## 15. Later, and decided separately

**Decided separately**, each in its own DECISIONS entry:
- `verify`'s R6 gate (its remedy, `gen-driver`, is model work);
- a crate-shape gate for `verify` and `promote`, or a crate hash over the whole directory
  (SAFE-5; suggested as its own task);
- switching harness-detect's walk to `walk::confined` (suggested as its own task).

**With the chat pane:**
- an idle watcher, which must re-read off the UI thread and fingerprint every file the snapshot
  reads;
- how chat's acts reach the activity panel.

**Later:**
- a findings view, using `observer::finding_state`, `load_annotations` and `affected_units`,
  with `facts_records_hash` moved into core;
- a guided "record an outside edit" flow that shows the diff and asks "did you write this?",
  then stages the closed file list;
- a find filter;
- a wide-View zoom;
- a "show all files" toggle;
- NO_COLOR;
- `harness review` from a file;
- right-click;
- scrollbar drag (`?1002h`).

## R. Design review — 46 findings (43 confirmed, 3 partly, 0 refuted) and their resolutions

Four lenses, run independently. A separate verifier checked every finding against the code.

**Safety & provenance**

| id | sev | finding (short) | resolution |
|---|---|---|---|
| SAFE-1 | high | Enter on a Run button focused by default: a held Enter confirms after the OS repeat delay | the safe button has default focus; after arming only a move plus Enter, or the letter, confirms (§5.3) |
| SAFE-2 | med | arming by "time since input arrived" drops today's `poll(0)` check | three conditions and a latch (§5.3) |
| SAFE-3 | high | Retry of an unseeded `external` attempt poses a blind hand-off; an answer is scored as pipeline output (today too) | not offered; Resume only for the cockpit's own seeded hand-offs; a guard narration (§4.3, §6.1); Build A step 0 |
| SAFE-4 | high | `verify` from a menu verifies unlabelled edits; §12.4 claimed the verdict records provenance | Re-check only on known code; `⚠ changed outside the harness`; `✓?` for unattributed code (§2.3, §4.3, §12) |
| SAFE-5 | med | the crate hash skips files outside `src/` — existing | decided separately; a task is suggested (§15) |
| SAFE-6 | high | the cockpit's reads are unbounded — existing | the preflight moves into the library and runs before every load, on a worker (§6.3); Build A step 0 |
| SAFE-7 | med | the View could show old Rust while Re-check verifies new code | `crate_digest` in the pairs key; the confirm-time check (§6.3) |
| SAFE-8 | med | dialogs understated their writes; hints skipped "review the diff" | a per-act write list; the plan summary notice (§5.1, §6.2) |
| SAFE-9 | med | a walk with caps truncates the scanner or lies; FIFOs hang | the walk returns data; the scanner stays fail-fast and skips non-regular files (§2.1) |
| SAFE-10 | med | a signal right after the editor races `resume` — existing | the non-blocking guard; the kept edit recorded first (§11) |
| SAFE-11 | low | `Q` and the quit prompt act unarmed — existing | armed dialogs (§5) |
| SAFE-12 | low | Retry takes its provider from the committed record — existing | the provider list; words in the dialog (§4.3) |

**Engine reuse**

| id | sev | finding (short) | resolution |
|---|---|---|---|
| ENG-1 | high | `files` cannot be pure over an unchanged `Snapshot` | explicit inputs; additive model fields (§2.3, §11) |
| ENG-2 | high | the state table misread `in-progress`, had no precedence, and "plan predates facts" could not be computed | ordered rules with a fallback; the Next step restated (§2.3, §3) |
| ENG-3 | high | the pairs cache showed old C after an outside edit | the key gains freshness and the digest (§6.3) |
| ENG-4 | high | "one implementation" could not serve both scanner and tree; tests could not move | a language-neutral walk; extensions and tests stay in the frontend (§2.1) |
| ENG-5 | med | per-unit write-in-flight and hand-off states not derivable | busy at project level via `live_holder`; the cockpit's own hand-offs only (§2.3) |
| ENG-6 | med | the idle watcher missed new files and blocked the UI thread | no watcher; loads on a worker; double-click discarded across stalls (§6.3, §7) |
| ENG-7 | med | narration did not match the events | a stateful narrator; the table corrected (§6.1) |
| ENG-8 | med | acts depended on a rail the tree replaces; the crate node unreachable | `Selection`; accelerators act on it; the crate node (§2.2, §4, §8) |
| ENG-9 | med | `E` and `D` did not fit | the kept-edit items; a three-button override dialog (§4.2, §5.2) |
| ENG-10 | med | timing logic untestable; signals.rs broken; fixtures missing | the clock injected; the rewrite and fixtures listed (§5.3, §13) |
| ENG-11 | med | static functions had no pair; static-only files never plan | internal functions; state 5 (§2.3, §3) |
| ENG-12 | low | a findings view ignored triage and reviews; §12.6 overclaimed | findings view cut (§15); §12.6 made true |

**Usability**

| id | sev | finding (short) | resolution |
|---|---|---|---|
| USE-1 | high | the tab layout below 110 columns hid progress | one layout down to 80 columns; Activity always visible (§1) |
| USE-2 | high | "Show" was a no-op and nothing opened a file | Open; recommended focus; a C view for unowned files (§3, §4.1) |
| USE-3 | high | dropped presses gave no feedback | visible arming states (§5.3) |
| USE-4 | med | `Q` quit and cancelled without confirming; Ctrl-C quit | armed quit and cancel dialogs (§5.2) |
| USE-5 | med | refusals meant retyping; kept edits reachable only by key | busy greying; `t`; the note memory; kept-edit items (§4) |
| USE-6 | med | notices had no home | Activity row 2 (§6.2) |
| USE-7 | med | file menus lacked unit actions; no way back | unit items on file menus; the back stack (§4.2, §2.2) |
| USE-8 | med | failures not visible at a glance | action counts; words on rows; lists (§2.3, §3) |
| USE-9 | med | states hid who must act | busy separated; ⚠ only for causes the user can fix (§2.3) |
| USE-10 | med | code cut without a marker; stuck ←/→ | horizontal scroll and `›`; `→` into the View, `←` back (§1, §8) |
| USE-11 | med | double-click cancelled dialogs; the wheel could not reach a dialog | outside clicks never cancel a dialog; the wheel scrolls the dialog; the drag hint (§7) |
| USE-12 | med | misleading menu items | greyed reasons; state 5; replace wording; the model separator (§4) |

**Scope**

| id | sev | finding (short) | resolution |
|---|---|---|---|
| SCOPE-1 | high (partly) | one build and one review | Build A and Build B (§0, §14) |
| SCOPE-2 | med (partly) | §10 pre-decided the chat architecture | reserved space only (§10) |
| SCOPE-3 | med | the order put risk last | reordered (§14) |
| SCOPE-4 | med | the walk move contradicted itself; the detector's walk diverges | the walk returns data; the detector's walk decided separately (§2.1, §15) |
| SCOPE-5 | med | the R6 gate on verify is a deferred CLI contract change | decided separately (§15) |
| SCOPE-6 | med (partly) | findings in the file View | cut; the file View shows the file's pairs (§3) |
| SCOPE-7 | med | the idle watcher was not asked for | deferred (§15) |
| SCOPE-8 | med | the second (tabbed) layout | one layout down to 80 columns (§1) |
| SCOPE-9 | med | goldens heavy, behaviour tests thin | 6 goldens; named mutation-checked rules (§13) |
| SCOPE-10 | low/med | extras; Next step suggested a refused migrate | cut; Next step restated from facts (§3, §15) |

## R2. Check of the revision — 13 findings, all resolved

One checker went over the §R resolutions against the code and the direction. Its findings,
three of them high:

| id | sev | finding (short) | resolution |
|---|---|---|---|
| CHK-1 | high | Modify has no `--provider`, so the target's `harness.toml` picked it (today too) | Modify passes `--provider` from the cockpit's list; the dialogs name the provider and model; Retry's greyed reason names the flag (§4.3) |
| CHK-2 | high | rule 7 put ⚠ on states nothing clears; zopfli's u001 (M0 layout) could never be re-checked | Re-check also on code the oracle judged (the verdict's `rust_crate`); `✓? origin not recorded` is its own non-action state; ⚠ only for fixable causes (§2.3, §4.3) |
| CHK-3 | high | "Record the crate as it is" staged two files only, hit dead ends (identical attempt, R6, plan staleness), needed an unsaid `--replace`, and relabelled unknown authorship as human | cut; Help names `harness override`; a guided flow is for later (§4.3, §15) |
| CHK-4 | med | the confirm-time check had no reference digest; "nothing verified unseen" overclaimed | `UnitView.crate_digest` in the pairs key; the digest the pairs were built from; "unchanged since shown" (§6.3) |
| CHK-5 | med | the terminal guard could deadlock (a panic in `resume`; a blocked write) | restores never block: `dying` first, unconditional restore, a bounded `try_lock` wait (§11) |
| CHK-6 | med | keyboard confirmation under-specified (per-key quiet, Enter on Cancel, other buttons, quit choices) | a latch; the safe button first; letters and move plus Enter after arming; keys per button (§5.2, §5.3) |
| CHK-7 | med | busy greying went stale | the holder re-read at menu open and at confirm (§4.1, §4.3) |
| CHK-8 | med | `[Try again]` had no key; the plan notice was persistent and multi-line in one row | `t`; a one-line plan summary, lines in `c`; notice precedence (§6.2, §8) |
| CHK-9 | med | the diff and Accept were hard to reach from a file; Esc in Files did nothing; "Show C beside Rust" was a no-op | Compare on attempt menus; Accept on file and unit menus; Esc goes back; the no-op removed (§4.2, §8) |
| CHK-10 | med | the preflight on the UI thread with a server's budget | loads on a worker; the last snapshot kept on failure; refusal in words at start (§6.3) |
| CHK-11 | low | holes in the state table (merged with no provenance, ⊘ on files, a doubly owned file) | a fallback row; ⊘ on unit rows only; the non-blocked owner wins (§2.3) |
| CHK-12 | low | the layout rules and mocks contradicted each other | words on rows when they fit; mocks redrawn at 120 and 76 columns (§1, §5) |
| CHK-13 | low | "the CLI re-checks everything" was false for these gates; a half-seeded record was retried unseeded | the gates declared as the cockpit's own, re-checked at confirm; a half-seeded Retry refused (§4.3) |

## R3. Code review of Build A — 51 findings, verified, resolved (2026-09-25)

Four lenses (safety & provenance, process & terminal, engine & state, usability & design
conformance) over c39c9b6, then two independent verifiers that read the reviewed commit
only: 51 findings, 49 distinct — 33 confirmed, 13 partly, 3 real but as this design asked,
0 refuted (overlaps merged below). Every fix has a regression test that fails
without it, and the rules are mutation-checked. Where a resolution changes this design, the
text above is not rewritten; this table governs.

| finding (ids) | resolution |
|---|---|
| dialog focus wrapped: one `←` from the safe button reached Discard, Run, Stop (USE-1, SAFE-6) | focus clamps: `←` stays on the safe button; a destructive button is reached only by moving right to it |
| Try again rebuilt the command without its unit, attempt and note: Re-check and Retry retries always refused, dialogs said "the unit" (USE-2, ENG-1, SAFE-4) | the run keeps the confirmed command whole; Try again offers it; the "Last:" line says "another command is changing this project (…) — nothing was done" |
| confirm-time gates read the last snapshot's crate and did unbounded reads on the UI thread (SAFE-1, SAFE-2, PROC-6) | confirm runs the preflight first, re-reads the plan (refuses when it names another crate), then hashes; the migrate model is read by the loader, not the dialog |
| "unchanged since shown" compared against a digest a background load replaced (SAFE-3, ENG-8) | the digest shown is captured into the command when the dialog opens |
| Resume: no gate at confirm; a blind hand-off could be tracked (SAFE-4, SAFE-12) | Resume re-checks at confirm (in progress, seeded, this cockpit's, response present); an `awaiting` of a run without `--steer` is never tracked |
| a stale tick read applied after a reap pruned the new hand-off (PROC-1) | `app::fold_loaded`: a read superseded by a later reaped or asked-for read is dropped, its reasons carried over |
| restores could block on the stdout lock of a stalled terminal; a helper-thread panic froze the process; drawing outside the guard; a window after the editor where a signal named no edit; the mirror updated after deletion (PROC-2…5, PROC-11) | cooked mode first, then bounded writes on a helper (as harness-cli does); a helper-thread panic interrupts the child and exits 101; `park()` is bounded; frames draw under the guard; the edit joins the kept list before the editor starts; the mirror is updated before any deletion |
| a verified unit whose C `plan` re-approved stayed "C changed — scan, refresh" (ENG-2) | SourceChanged only while the plan is stale; a verdict stale on its source is StaleVerdict ("Re-check it") |
| Next step stuck on blocked units and on a plan with no units; missing files counted twice (ENG-3, ENG-6) | blocked units skipped; "No plan yet" only without a plan file; the stale paths already hold the missing files |
| Re-check offered where the View shows C source, then always refused (ENG-4) | greyed: "open the unit (or its crate) first — the cockpit re-checks only code it shows" |
| Accept from a unit, crate or file promoted a candidate the View never showed (ENG-5; this design's §4.2 listed it) | **changed**: the unit-level item is "Accept a-… into u-… (opens it first)": it selects the attempt, whose code the View shows; Accept is confirmed there |
| Accept and Hand edit offered on a crate changed outside the harness (SAFE-8, SAFE-9) | both greyed until the crate is restored or recorded with `harness override` |
| Discard offered while the override recording that edit ran (SAFE-7); the lock read failing open (SAFE-10) | Discard greyed while a command runs; an unreadable lock file is busy |
| source_dir unchecked: the tree could list files outside the target (SAFE-11) | a source_dir that leaves the target, by path or link, makes the read refuse |
| invisible format characters passed the filter; 4-hex short ids in dialogs; ids unchecked in argv (SAFE-13) | U+200B–D, U+2060–64, U+FEFF, U+00AD, U+2028/9, the tag block shown as `?`; full ids in dialog titles; ids must be plain path segments |
| a dialog could arm on the frame that first showed it after a stall; tiny terminals (SAFE-5) | the quiet time also runs from the first full draw; a dialog never arms on a terminal too small to show it (it says so) |
| a deleted verdict called an M0 crate "changed outside" (ENG-11) | a new cause: "its verdict is missing, so the harness cannot tell whose code the crate is"; Re-check stays greyed (unknown code is never verified) |
| owned headers showed "header"; a function of a changed file showed the file's word (ENG-12) | an owned file takes its owner's state, headers too; an in-unit function takes its unit's state |
| duplicate definitions (`#if`/`#else`) made identical rows and stuck `↓` (ENG-7) | one row per name |
| the model's fallback caught every error (ENG-10) | only a missing input binds nothing; harness-mcp now reports such a target instead of refusing it (MCP-DESIGN §4) |
| `v` missing on attempts (ENG-9, USE-5); the hint bar ignored menus and dialogs (USE-3); functions opened at line 1 (USE-4); the fallback glyph was the header's (USE-6); ⊘ on a file in a golden (USE-7); names squeezed to `…` (USE-8); sandbox words contradicted (USE-9); no Open on the project (USE-10); `E` only on some nodes, Discard titled "Record" (USE-11); "Too soon" hid "scroll" (USE-12); Enter in the View jumped to a link on first run (USE-13); unbounded sideways scroll, Space paging the tree, Ctrl-C ignored in overlays (USE-14) | Show the checks on attempts; the hint bar lists the open overlay's keys (`? help`/`q quit` only in the panes); a function opens at its line; the fallback has no glyph; the golden fixed; the name keeps half the row and the C heading its line number; sandbox words follow the flag; Open on every node; `E` from anywhere, the discard dialog says what it is; "scroll to the end" first; Enter jumps only to a chosen link; sideways scroll stops at the widest line; no Space in the tree; Ctrl-C closes overlays |
| an empty quarter of the screen from 150 columns (USE-15; this design's §1/§10) | **changed**: the View takes the width until the chat exists |
| pty tests: UTF-8 split across reads, restore order unchecked, cooked mode unchecked, reused pids killed, temp dirs leaked; the loader test timing-dependent (PROC-7…10) | bytes decoded across reads; the restores asserted LAST; the shell records `stty -a` after a TERM (icanon, echo); only live pids reaped; temp dirs in drop guards; the loader test waits on the first read |
