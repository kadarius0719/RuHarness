# Chat recordings

Claude Code 2.1.274 (`--model haiku`), run headless with the exact command line of
docs/CHAT-PANE-DESIGN.md §1.1 and `harness-mcp --cockpit`, by `record.py` — a mini cockpit
that allows the reads, runs each act with the harness CLI itself and answers the call
`deny` + the outcome. Recorded 2026-09-28 on a zopfli copy whose `u001-katajainen` was reset
to pending. Each line is `{"dir": "in"|"out"|…, "msg": …, "t": seconds}`: `in` is what the
cockpit wrote to the runtime's stdin, `out` what the runtime printed. Paths are rewritten
(`/tmp/rec`, `/repo`); thinking deltas beyond the first three, `system/thinking_tokens`
lines and tool-input deltas beyond the first three are dropped (the cockpit ignores them).

- `round.jsonl` — "Please migrate this unit.": status, unit, `harness_migrate` (awaiting),
  `harness_request`, `harness_answer` (green), the summary.
- `stop.jsonl` — the same request, interrupted while `harness_migrate` is held: the
  runtime's `control_cancel_request`, the rejection, its interrupt marker, `aborted_tools`;
  then a second message.
- `decline.jsonl` — the same request, `harness_migrate` answered `deny` "declined by the
  person": the model says so and ends the turn.
- `stream-stop.jsonl` — interrupted at the first delta: `aborted_streaming`, the marker; then
  a second message.

Seen in these and not in the spike's logs: every user message the cockpit sends comes back
with `"isReplay": true` and the same `uuid`; `command_lifecycle` (`queued`, `started`,
`cancelled`, `completed`, by `command_uuid`); `system/status`; `system/thinking_tokens`; the
runtime echoes each `control_response` it received; the interrupt's response carries
`still_queued`.

Re-record: `python3 record.py <scenario>` (a live model call through your own `claude`
login; `RECORD_DIR` keeps the logs), then trim as above.
