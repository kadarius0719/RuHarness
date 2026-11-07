# The run log: a file that keeps the evening, and a view that shows it

Status: built 2026-11-07 (what the build changed from the text below is in §B), after the
review in §R the same day. Builds on
COCKPIT-WRAPPER-DESIGN (§6 the activity panel, §9.3 one list, one card, one line) and
CHAT-PANE-DESIGN (§1.3 the stream, §5 the screen). Governs the log file, the Log view, the
chat's heartbeat and the Details panel's plain pause once built; SCHEMAS.md gets the file's row.

## 0. What it is, and is not

The person asked for it on 2026-10-30, watching the chat think for ten minutes with "(0 s)" on
the status row and a Details panel that read "error: awaiting response / result: exit 1" for a
pause that was normal: *another interface where a user can intentionally get a running log of
tasks and processes*, and something more verbose than Details for debugging.

What it is:

- **A log file** the cockpit appends to as things happen: every command it ran with its full
  output, every chat reply with its time and cost, every line the chat pane showed, what the
  cockpit told the model, every dialog's answer, every notice. One event per line, timestamped.
  It is for whoever debugs — the person, a session reading it, a bug report — and may be as
  verbose as it likes. Today nothing reaches a file: a run's lines live in memory under the one
  current command and are gone when the next runs; the chat's transcript dies with the chat.
- **A Log view** inside the cockpit, opened on purpose with one key and closed with Esc, full
  width over the panes, following the end as things happen. Plain words by default, the raw
  lines underneath on a toggle. The screen's calm stays: nothing new appears unless the key is
  pressed.
- **A heartbeat** while the chat thinks: how long, and when Claude Code was last heard from.
  The one thing the screen itself was missing.
- **A plain pause** at the top of Details when the run paused for the chat.

What it is not: a transcript of the model's replies (the chat pane has them while the chat
lives; their substance is in the ledger's traces); a replacement for the ledger's records
(attempts, traces, verdicts are the evidence, the log is the diary); a second status row; a
file the harness CLI reads (`tail -f` reads it; the view is the cockpit's); a mirror of the
screen (§2: the log says a little more than the screen, never less).

## 1. The file

**Where.** `<target>/migration/build/cockpit.log` — the target root's ledger, in the folder
every ledger's `.gitignore` names (`build/`, matched at any depth; written by the first map,
never over an existing one) and where SCHEMAS already puts "logs with wall-clock time". On a
mapped project the build folders are the tools' own, so the root `migration/build/` is usually
the cockpit's to create. This is **the one exception to "the cockpit never writes the ledger"**
(COCKPIT-WRAPPER-DESIGN §0): a diary in an ignored folder, append-only, never read back, that
no harness command reads either. DECISIONS records the exception. A mapped tool opened in the
cockpit logs to the target root's file, not the tool's ledger: the Project view's acts (Scan,
Refresh the plan, Accept) have no tool yet. Adopting a program deletes `migration/build/`
(`delete_build_folders`) and the cockpit adopts mid-session: the writer closes its file before
the adoption and opens it again after, with a header line; nothing is written into a deleted
file.

**Never through a link.** The target folder is untrusted. The folder is created and the file
opened with `O_NOFOLLOW` and `O_APPEND`, owner-only (0600; the folder 0700 when the cockpit
makes it); a `migration/`, `build/` or `cockpit.log` that is a symbolic link, or a file with
more than one hard link, is refused — the log is not written, and the Log view's footer says
why (the rule `.lock` and `write_gitignore` follow). The check is made at every open (start,
and after an adoption).

**Shape.** One JSON object per line, written with the same escaping as the CLI's `--json`
stream (`json_safe`: DEL, the C1 controls, the bidi and invisible format characters as
`\uXXXX`, so `tail -f` shows nothing a terminal acts on; the helper moves to harness-core and
both use it). The CLI's conventions: `k` is the kind; a header line opens each session:

```
{"k":"header","schema":"ruharness-cockpit-log","schema_version":1,"t":"2026-11-07T17:12:33.418Z","pid":13792,"harness":"0.1.0","target":"/Users/b/lz4-map"}
{"k":"dialog","t":"2026-11-07T17:12:51.002Z","pid":13792,"piece":"u-lz4","words":"Make u-lz4's test program — ask in chat"}
{"k":"command","t":"2026-11-07T17:12:51.130Z","pid":13792,"piece":"u-lz4","words":"Make u-lz4's test program — started (allowed by you)","raw":"harness --json gen-driver u-lz4 --target=/Users/b/lz4-map --tool=t-lz4cli --provider=external --model=claude-opus-5-5 --requester=chat"}
{"k":"command","t":"2026-11-07T17:12:51.400Z","pid":13792,"piece":"u-lz4","words":"turn 1 generate → …","raw":"{\"k\":\"turn-start\",\"unit\":\"u-lz4\",\"attempt\":\"d-8378804af048\",\"index\":1,\"kind\":\"generate\"}"}
{"k":"command","t":"2026-11-07T17:12:51.610Z","pid":13792,"piece":"u-lz4","words":"paused: waiting for the chat's answer to turn 1 (0 s)","raw":"exit 1"}
{"k":"chat","t":"2026-11-07T17:12:51.700Z","pid":13792,"piece":"u-lz4","words":"told the chat: Make u-lz4's test program paused — turn 1 waits for your answer in …/bec63844.response.json"}
{"k":"chat","t":"2026-11-07T17:13:04.220Z","pid":13792,"piece":"u-lz4","words":"read the request of d-8378804a… (page 1)","raw":"harness_read_request {\"attempt\":\"d-8378804af048\",\"page\":1}"}
{"k":"chat","t":"2026-11-07T17:13:04.900Z","pid":13792,"piece":"u-lz4","words":"read the request of d-8378804a… (page 1) — done"}
{"k":"chat","t":"2026-11-07T17:19:40.900Z","pid":13792,"piece":"u-lz4","words":"Claude Code says: the reply hit the output limit; the chat goes on from where it was cut"}
{"k":"chat","t":"2026-11-07T17:24:11.031Z","pid":13792,"piece":"u-lz4","words":"the chat's reply ended — 439.0 s · $1.774 of plan usage"}
```

Fields: `k` (`header`, `cockpit`, `command`, `chat`, `dialog`, `notice`; an open set), `t`
(UTC, RFC 3339, milliseconds), `pid` (two cockpits on one target interleave; the pid tells them
apart), `piece` (the unit id when the event has one, absent otherwise), `words` (the plain
sentence), `raw` (the command line, the output line, the event record, the tool call —
absent when the words are the whole of it). Fields are additive. A `raw` longer than 4 000
bytes is cut with ` … (N more bytes)`; the words never are.

**Writing.** A small writer thread owns the file (the screen's thread never waits on a disk:
COCKPIT-WRAPPER-DESIGN §6.3's reason for the ledger's worker); the app sends it lines through
a channel and keeps its own copy for the view. A write that fails (disk full, the folder
read-only) is said once in the notice row ("the log could not be written: <why>; the Log view
still shows this session") and the file is left alone from then on. A target with no
`migration/` yet has no log until the first Map makes it: the events until then are kept in
memory and written once the folder exists. The time on each line is the writer's wall clock
(`SystemTime`); the view shows it local, through `localtime_r` (libc is already a workspace
dependency; harness-tui gains it, no new crate).

**Rotation.** None. A busy evening is tens of kilobytes; a month of evenings is a few
megabytes. §10 holds the size rule if it is ever needed.

## 2. What is logged

The rule: **everything the cockpit says, does or decides, in the words it uses.** Most rows
are a line the screen already shows; a few are decisions the screen has no line for (the
permissions, what the chat was told, who allowed a run), and the log is where they get one.
Nothing in a row is invented: each quotes a sentence the code already builds or a value the
ledger or the runtime gave.

| event | `k` | words | raw | piece |
|---|---|---|---|---|
| a session starts | `header` | — (the header's fields: pid, harness version, target) | — | — |
| the cockpit ends by `q` | `cockpit` | "cockpit ended" (a signal or a panic leaves no row: a session without one ended that way) | — | — |
| a dialog answered | `dialog` | "<title> — <button's label>" ("Make u-lz4's test program — ask in chat", "Migrate u-lz4 — y", "— cancelled", "— discarded", "— quit, the command runs on"); the Cancel dialog closed by the run's own end: "— the command ended" | — | `Pending::unit` |
| a command spawned | `command` | "<label> — started (allowed by you)" or "(allowed by the chat's continuation permission)" | the command line (`shell_line(argv)`) | `Pending::unit` |
| a command's line | `command` | the narrator's step when the line changed it, else the line's words (a `message` event's text, a stderr line) | the original line (stdout as printed, stderr) — `ChildMsg::Event` gains the raw line | `Pending::unit` |
| a command ended | `command` | `RunPanel::exit` as Details shows it, with the duration | "exit N" / "signal <name>" | `Pending::unit` |
| what the chat was told | `chat` | "told the chat: <the deny message the cockpit sent>" (`chat_owed`'s message to the model, which differs from the pane's line) | — | the act's |
| a permission granted / ended | `chat` | "the chat may continue <act> without asking (<attempt>)" / "the chat's permission to continue ended: <why>" (`end_permits`/`end_permit`'s reason) | — | the act's |
| a chat process starts / ends | `chat` | "chat started (<model>, <sign-in words>)" — the endpoint with any user name and password removed — / "chat ended" or "chat ended: <why>" | — | — |
| a reply starts / ends | `chat` | "the chat's reply started" (the model begins after the person's message, or after the cockpit hands an outcome back: `released()`) / "the chat's reply ended — <time> · <cost>" (the `— 382.7 s · $1.159 of plan usage` line) — no "turn n": the chat's turns are not the attempt's | — | the running act's piece |
| every line the chat pane shows | `chat` | the line, as the transcript gets it: a cockpit line, a request's line, an act's line and each change to it, a read's call ("read the request of d-… (page 1)") and separately its result ("… — done" / "— failed"), "stopped", a nudge, a foreign message with its 300 characters, the person's own message ("you: …") | a tool call's name and input, cut at the limit | the running act's piece |
| Claude Code's own error output | `chat` | the stderr line (today kept only in `stderr_tail`) | — | — |
| the model's own text | — | not logged (§0) | — | — |
| a notice | `notice` | the notice row's text, when it changes (compared once per loop pass against the last logged, so the 88 places that set one need no edit) | — | the running act's piece, else none |
| the log could not be written | `notice` | the notice's words (memory only, by definition) | — | — |

Secrets never reach the file: the chat's environment is not an event; `sign_in_words` is
scrubbed of credentials in a URL; a command line holds paths and ids; a tool call's input is
the request the chat made of the harness (attempt ids, page numbers, ledger paths), cut at the
limit. Tool *results* — the `[C SOURCE]` the chat reads, a request's pages — are never logged.
Every value quoted is the ledger's or the runtime's and is untrusted: the file escapes it
(§1), the view renders it through the display filter like every other string
(COCKPIT-WRAPPER-DESIGN §12).

**The piece.** A dialog's and a command's piece is `Pending::unit`. A chat row's is the piece
of the act the chat is running or answering (the running `Pending`'s unit, or the last one the
chat was told about), not the tree's selection (`App::about` describes the selection, which the
person moves freely while the chat works). Events of no piece (a Scan, a Refresh, the
cockpit's own) carry none.

## 3. The Log view

**Opening.** `L` from the tree or the View; `Ctrl-L` from the chat (letters are text there,
CHAT-PANE-DESIGN §5.4). Both are listed in the help (`?`) and in the key row of the screen
they work on. Nothing else opens it: no button on the status row (one line, §9.3), no menu
item (the menu is acts). `Esc` or `L` closes it and the focus is where it was.

**Where.** `Mode::Log`, an overlay over the whole panes area (the tree, the View and the chat
pane together), like Details but full height; the status rows stay underneath. A terminal too
narrow (under 40 columns) shows one row — "make the window wider to read the log — Esc
closes" — the dialog's small-terminal rule.

**Keys it keeps and keys it passes.** The Log handles `↑↓` (one row), `PageUp`/`PageDown` (a
page), `Home` (the top), `End` (the newest, follow again), `r` (raw), `Esc`/`L` (close), and
the mouse wheel. It passes through only what must keep working while it is open: `x` and
`Ctrl-X` (cancel the running command — the status row underneath shows what runs), `Ctrl-C`
and `q` (quit, through the quit dialog as usual). Every other key is swallowed: `r` is raw,
never Retry; `m`, `a`, `t` do nothing until Esc. A click outside the overlay does nothing (the
Details rule: the panes outside keep answering clicks only when they are visible, and here
they are not).

**The chat's requests while the Log is open.** The chat's requests are read behind the
overlay; a request that arrives while it is open puts one line in the notice row underneath,
"the chat asks — Esc to see", the same words the row uses when the chat lacks focus; the
overlay never answers a request.

**What it shows.** The session's events, oldest at the top, newest at the bottom, following
the end until the person scrolls up (then `↓ newest` on the last row; `End` follows again —
the chat transcript's rule). One event is one row:

```
17:12:51  u-lz4   Make u-lz4's test program — started (allowed by you)
17:12:51  u-lz4   turn 1 generate → …
17:12:51  u-lz4   paused: waiting for the chat's answer to turn 1
17:13:04  u-lz4   read the request of d-8378804a… (page 1)
17:13:04  u-lz4   read the request of d-8378804a… (page 1) — done
17:19:40  u-lz4   Claude Code says: the reply hit the output limit; the chat goes on
```

The time is local wall-clock, `HH:MM:SS`; the piece column is the width of the longest piece
id in the plan (blank for an event of none); the words wrap under their own column. Tones are
the screen's: a dialog's row in the label role, a command's exit in its Details colour (green
"exit 0", the notice colour for a pause, warn for a failure), a chat line in the tone it had in
the pane, a notice in the notice colour, the cockpit's own rows dim.

**Raw, on a toggle.** `r` shows, under every row that has one, its `raw` in the dim tone,
indented past the piece column and wrapped. Off by default; the state is the session's. This
is the "more verbose": the exact command, the exact output, the tool call. The title row says
"Log — this session · raw" when on.

**The footer.** The overlay's last row names the file: `migration/build/cockpit.log — copy
this file when reporting a problem`, in the dim tone; when the log is not written: `the log is
not being written (<why>) — this view shows this session only`.

**Memory.** The cockpit keeps the session's last 5 000 events for the view (older ones are in
the file); the title says "(the first N are in the file)" when it has dropped any.

## 4. The heartbeat

The one line the screen was missing. Two clocks the chat already has, shown in two places:

- `reply_started`: when the model began its current reply — the person's send, or the moment
  the cockpit hands a command's outcome back (`released()`). Today's `turn_started` is set at
  the send alone and so counts the dialog's wait and the command's own run: it is the wrong
  clock for "how long has it been thinking". `reply_started` is set at both and cleared at the
  reply's end.
- `last_out`: when Claude Code last printed anything (every stream line, the model's text as
  it streams included; the minute-long "nothing from the model" notice already uses it). This
  is the honest "is it alive": a model writing a long file is heard from every second even
  while no read or call happens.

**The chat pane's header**, while a reply runs: `Chat — thinking 4 min 12 s · fable-5-1`,
ticked every second like the activity row's spinner; `thinking… heard 3 s ago` is not needed
there — the pane shows the stream.

**The status row**, when the harness command paused on the chat: today it shows the finished
sentence `Paused: the chat answers turn 1 (0 s)` (`Narrator::last` at `on_child_exit`; "(0 s)"
is the command's own duration — true and the wrong clock). The app records that the pause is
the chat's (`chat_pause: Option<Instant>`, set with `run.pending.chat` at exit, cleared when
the chat's next command starts or the chat ends) and the row then says, clock first, short
enough for 80 columns under `fit_status`:

- a reply running: `Chat thinking 4 min 12 s · heard 3 s ago — answering turn 1 for u-lz4`
- no reply running, the run still paused: `Chat quiet 40 s — its reply ended; the command
  has not been asked again` (a stall, visible)
- the chat ended: `Chat ended — turn 1 waits; Ctrl-N for a new chat` (the way out)

`last` keeps its sentence for the moment the pause began, for the Details panel and the
idle row once the pause is over.

## 5. Details: the pause said first

The Details panel for a run that paused on a hand-off begins, before the command line, with
one row in the notice colour: `This pause is normal: the harness wrote its question to a file
and waits for the chat's answer.` (or, for the person's own hand-off, `… and waits for your
answer — <file>`). The raw lines follow as today, so nothing is lost; the panel's last line
(`paused: …`) stays. The lines "error: awaiting response" and "result: exit 1" are the
harness's own and stay as they are: they are true, and the lead row says what they mean.

## 6. Safety and provenance

- The log is append-only and never read by the cockpit or by any harness command: nothing in
  it can steer anything.
- It is never written through a link; the file and the folder are the owner's alone.
- It lives inside the target's ledger, in a folder git ignores, so a target's code (if a line
  ever quoted some) stays in the target's repo (AGENT-BRIEFING §12.1).
- Every quoted value is escaped in the file and rendered through the display filter in the
  view: a file name with an escape sequence in it is as harmless in `tail -f` as in the tree.
- The chat's environment is never logged; the sign-in words are scrubbed; a tool call's input
  is cut at 4 000 bytes; tool results are never logged; the model's own text is not logged.
- The writer thread cannot block the screen; a failed write is said once and never retried.
- A reviewer can check: every row's words are a sentence the code builds for the screen or a
  decision's reason the code already holds; §8's test opens a dialog, runs a command, drives
  the chat, and checks that every screen line is in the file in order (the file may hold
  more; never less).

## 7. Engine reuse: what changes in the code

New module `crates/harness-tui/src/runlog.rs`: `RunLog { events: VecDeque<Event> (5 000),
dropped: usize, writer: Option<Writer>, not_written: Option<String> }` with `fn open(target_root,
pid) -> RunLog` (the link checks, the header line, the thread), `fn push(&mut self, k, piece:
Option<&str>, words, raw: Option<&str>, tone)` (stamps, caps raw, queues the line, keeps the
event), `fn close(&mut self)` (before an adoption; `open` again after), `fn footer_words()`.
`Event { t: SystemTime, k, piece, words, raw, tone }`; tests give it a fixed clock.
`harness_core::text::json_safe` (moved from the CLI's `report.rs`, which calls it there).

Hooks:

- `App::new`: `open`; the `q` path: "cockpit ended" then `close`. `close_adopt_dialog`
  (app/adopt_offer.rs): `close` before, `open` after.
- `close_dialog(confirm, choice)` and the Cancel dialog's close in `on_child_exit`: the
  `dialog` row, from the dialog's title and the chosen button's label.
- `on_spawned` (with who allowed it: a dialog's `y`, or the continuation permission —
  `Pending` knows), `on_child_msg` (`ChildMsg::Event { event, line }` carries the raw line;
  the narrator's `step()` compared before and after), `on_child_exit`: the `command` rows.
- `chat_owed`: the "told the chat" row with the message sent; `end_permits`/`end_permit` and
  the grant: the permission rows.
- The transcript is the funnel for the pane: `Transcript::line`, `update_line` and `you` push
  `(tone, words, raw)` onto a queue `Transcript::pending_log`, which `App::chat_pump` drains
  into the log each pass with the running act's piece. A read's call and its result are two
  rows (the fold is the screen's, not the diary's). `Chat` gains `reply_started` (set in `send`
  and `released`, cleared at `In::Result`); `Out::Stderr` in `pump` becomes a `chat` row as
  well as the tail; `start()` and the end path push the process rows.
- The loop (`main.rs`, after `app.tick()` and after each event batch): the notice row's text
  compared with the last logged; a change is a `notice` row.
- View: `Mode::Log { scroll: usize, follow: bool, raw: bool }`; `view::log_rows(app, width)`;
  drawn where Details is drawn, full height; keys beside Details' with the pass-through list of
  §3; `L` and `Ctrl-L` in the key maps and the help; the chat header and `activity_row` read
  `reply_started`, `last_out`, `chat_pause`; `details_rows` prepends the pause row.

SCHEMAS.md: a row for `migration/build/cockpit.log` in the files table and a short section
"The cockpit log" with the fields and the exception. `docs/topics/cockpit.md`: a paragraph
"When something looks stuck" naming `L` and the file. TESTING-GUIDE: a step in the cockpit's
part opening the Log after a run.

## 8. Tests

- `runlog`: a push writes one line, valid JSON, escaped like the CLI's stream (a line with a
  C1 control and a bidi mark comes out as `\u` escapes), the header first with the schema and
  the pid; raw cut at the limit with the suffix; a linked `build/` and a linked `cockpit.log`
  refuse and say why; a write failure records `not_written` once and later pushes still keep
  the event; the memory cap drops the oldest and counts them; `close` then `open` writes a new
  header.
- The hooks: a view test that runs a scripted command (the cockpit driver's fixture), opens a
  dialog, answers it, and reads the file: the dialog's answer, "started" with the command line
  and "allowed by you", the lines with their raw, the exit — and every line Details shows is in
  the file in order.
- The chat: the synthetic-stream chat test extended: every transcript line is a row with the
  act's piece; a read's call and its result are two rows; the reply's start and end rows; a
  nudge row; "told the chat" with the message sent; a permission ended with its reason; a
  stderr line.
- The adoption: the file is closed before and reopened after, with a second header; no line
  lost.
- The view: goldens of the Log overlay (plain, raw, scrolled with the hint, the footer's two
  states, the narrow terminal's row); the keys (`r`, `Esc`, `L`, `End` follows again; `x`
  passes through; `m` does nothing); the notice row's "the chat asks — Esc to see" while open.
- The heartbeat: the chat header with a running reply (fixed clock, "thinking 4 min 12 s");
  the status row's three states; `reply_started` set at `released()`, not only at the send.
- Details: the pause row first for a paused run, absent otherwise.
- `cargo fmt`, clippy `-D warnings`, the guide's gates.

## 9. Order of work

1. `json_safe` to harness-core; `runlog.rs` and its tests; the header and `cockpit` rows; the
   file in SCHEMAS.
2. The `command` and `dialog` hooks (`ChildMsg` with the raw line; who allowed); the hooks
   test.
3. The transcript funnel, the chat process and reply rows, "told the chat", the permissions,
   stderr; the chat test.
4. The adoption's close and reopen.
5. The Log view, its keys, goldens; the help and key rows; the notice row while open.
6. The heartbeat; the Details pause row.
7. The topic page, the guide step, DECISIONS (the exception), NEXT-SESSION.

## 10. Open

- **This piece only.** A `p` toggle filtering the view to one piece: wanted when a long run
  on a big project makes one piece hard to follow; left out of the first build (§R, scope).
- **Size.** If a file ever matters: at 50 MB the cockpit renames it `cockpit.1.log` at start
  and begins afresh, keeping one old file. Not built until seen.
- **Earlier sessions in the view.** The view shows this session; `tail` shows the rest. A
  renderer reading the file back would be a separate, read-only parser, if wanted.
- **A `harness log` command** printing the file in words for a terminal. Cheap; wanted only if
  someone works from the CLI and the view.
- **The model's replies in the log.** Left out on purpose (§0); if the person wants the chat's
  words kept after the chat ends, a `reply` kind, cut at a limit, behind an opt-in.

## B. As built (2026-11-07)

The build followed §9's order; the code is `crates/harness-tui/src/runlog.rs`, the hooks in
`app.rs`, `app/asks.rs`, `app/adopt_offer.rs`, `chat/mod.rs`, `chat/transcript.rs`, the view in
`view.rs` and `view/chat_pane.rs`. Where it departs from the text above:

- **The local clock.** Every crate forbids unsafe code, and `localtime_r` needs it. The view
  asks the system once, `date +%z`, for the zone's offset (no new crate), and the title says
  "· times in UTC" when that fails. The file's `t` is UTC either way.
- **An adoption and the file before it.** Adopting deletes `migration/build/`, and the log
  written before it goes with the folder (the harness's rule, not the cockpit's). The file is
  closed before and opened again after with a new header; the adoption dialog's own row is
  logged after the reopen, so the file that remains begins with it.
- **The adoption check knows the file.** A `migration/` holding only hand-written files is
  "no results" to `adopt::check`, and the cockpit's `build/cockpit.log` would have made it
  look like results made elsewhere (the cockpit's own re-read then refused the folder:
  `tool_reread` caught it). `holds_no_results` now takes a `build/` holding the log alone as
  the diary it is; a `build/` with anything else stays a result.
- **A closed log holds its lines.** Between `close` and `reopen` (and after `q`) nothing
  opens the file: lines pushed meanwhile wait and are written after the reopen's header.
- **The exit row** reads `<exit as Details shows it> (<took>)` — `exit 0 (41 s)`, `paused:
  … (41 s)` — with the raw `exit N` / `signal <name>` beside it.
- **The reply's end** is one log row, `the chat's reply ended — 382.7 s · $1.159 of plan
  usage`, in place of the pane's `— …` line (the pane keeps its line; the log has the
  sentence). A reply's start is logged at the send, at the runtime's `init`, and at every
  `released()`.
- **The footer** when the file is not written reads `not written: <why> — this view shows
  this session only` (short enough for 120 columns with the reason).
- **The narrow terminal's row** wraps to two when even "make the window wider … Esc closes"
  does not fit on one.
- **The status row while the chat is told an outcome** (its request still held by the
  cockpit, the model waiting rather than thinking) says `Chat is told the outcome, then
  answers turn 1 for u-lz4` — a fourth state §4 did not list.
- **The chat pane's keys** list `Ctrl-L log`; the panes' key row lists `L log`, dropped first
  when the row is too narrow. The goldens changed by those entries alone.
- Tests as §8 lists them, 17 new: eight on `runlog`, the hooks, the adoption, the chat's
  funnel, the permissions and "told the chat", three goldens of the view and its keys, the
  heartbeat's four states, the notice row under the log, the Details pause row.

## R. Design review — nine findings, resolved (2026-11-07)

A reviewer (Opus, high effort) read the first draft against the code. In plain words, what
it found and what changed:

1. **The file could be written through a link, and raw JSON can drive a terminal.** The
   draft created the folder and appended; the project refuses links everywhere else, and
   serde_json leaves DEL, the C1 controls and the bidi characters raw. Now §1: `O_NOFOLLOW`,
   owner-only, hard links refused, `json_safe` shared from harness-core.
2. **The chat hooks as named would miss lines.** `Chat::say` has no event list and many
   pane lines never pass through it (`App::chat_line`, `chat_act_line`, the in-place act
   line); the folded reads line never "closes". Now §7: the transcript is the funnel; a read's
   call and result are two rows; the "file equals the screen" claim became "every screen line
   is in the file".
3. **Debugging needs rows the screen has no line for.** What the cockpit told the model
   (`chat_owed`'s deny message differs from the pane's), the permissions granted and ended
   with their reasons, who allowed a run, Claude Code's stderr, the raw event lines (`ChildMsg`
   dropped them; the kind is `turn-start`, not `turn_start`). All in §2.
4. **The heartbeat used the wrong clocks.** `turn_started` counts the dialog's wait and the
   command's run; the chat does not number turns; `last_out` already exists and is the honest
   liveness clock; `App::last` is a finished sentence with "(0 s)" baked in; the sentence was
   too long for 80 columns. Now §4: `reply_started` set at `released()`, `last_out` for "heard
   N s ago", `chat_pause` as a real field, the clock first.
5. **Where the file lives.** `build/` is ignored at any depth, but the root `migration/build/`
   is the cockpit's to create on a mapped project, and writing it breaks "the cockpit never
   writes the ledger"; adopting deletes the folder mid-session. Now §1: the exception stated
   and recorded; close before and reopen after an adoption.
6. **Keys and the overlay.** `L` and `Ctrl-L` are free. Details does not close on a click
   outside, and passes unhandled keys to the panes (`r` would be Retry). Covering the chat
   hides its requests. Now §3: the explicit keep/pass list; the notice row's "the chat asks —
   Esc to see"; the narrow-terminal row offers Esc.
7. **Hook names.** `Act::unit()` does not exist (`Pending::unit`); the reaping path is
   `on_child_exit`; dialogs open in seven places and the Cancel dialog closes outside
   `close_dialog`; `Choice` has no "asked in chat" (the button's label has); notices are set
   in 88 places; `App::about` is the selection, not the chat's piece. All corrected in §2 and
   §7; the "dialog opened" row dropped; notices logged by comparison once per loop pass.
8. **Conventions and plumbing.** `k` and a header line like the events stream; the pid on
   every line; a writer thread; `localtime_r` through libc (already a dependency); no signal
   row (the signal thread cannot reach the app); `sign_in_words` scrubbed of credentials.
9. **Scope.** The "this piece only" filter and the "dialog opened" rows went beyond the ask;
   both out of the first build (§10 keeps the filter).

Found sound: `on_spawned`, `on_child_msg`, `RunPanel::exit`, `close_dialog`, `notice()`,
`turn_started`, the reply's end in `In::Result`, `details_rows`, `paused_words` exist as
named; `build/` is ignored; sandboxed builds cannot touch the log; leaving out the model's
text and tool results; never reading the file back; the Details pause row.
