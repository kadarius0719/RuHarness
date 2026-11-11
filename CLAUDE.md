# RuHarness — read first

1. Read `docs/AGENT-BRIEFING.md` whole: the mission, the architecture, the engineering standards
   and the rules. It is the brief every session works under.
2. Then `docs/NEXT-SESSION.md`, the kickoff: where the last session stopped and the work, in
   order. It is the current plan; no other file holds one.
3. `DECISIONS.md`'s last entries hold what was decided and why (its index at the top finds older
   ones).

`docs/README.md` says what every document is for, and whether it is current or history.

Rules that are never skipped, whatever the task:
- No cloud API keys here: model calls go through the `external` hand-off. Answer hand-offs with
  plain Agent subagents, never Workflow agents. Set `--model` to the model that actually answers.
- Never download anything (a model, a C project, a crate) without asking first.
- Vet every crate before adding it (maintenance, advisories, weight). No bloat, no dead crates.
- Report in plain words: no review codes (M1, N4, …). The person wants to learn from what is said.
- Solo developer: commit to `main` and push at each milestone, no pull requests, never force-push.
- Before a large multi-agent workflow late in a session, check the plan's usage and say what it
  will roughly cost.
- Models (the person, 2026-10-07; sharpened 2026-11-11): the main session may run on Fable;
  everything it hands off runs on Opus — Agent subagents with `model: "opus"`, every Workflow
  `agent()` call with `{model: 'opus'}` (the default inherits the main session's model),
  hand-off answers with `--model claude-opus-5-5`. Ask the person before giving any handed-off
  work Fable. The division of labour: Fable orchestrates and reviews — it plans, cuts the work
  into small tasks with a complete brief each, hands them to Opus subagents, and reviews what
  comes back (the diff and the gates, not the report) before anything is committed. Fable
  keeps the design decisions, the triage of failures and the final check; it does a one-line
  change or a quick lookup itself when the hand-off would cost more than the work.
- Effort: a session cannot change its own; it stops and asks the person, in one plain line, at the
  switch points: medium by default; high when the work turns to design judgement (a design, a
  review's triage, a revision), and medium again when building resumes. The kickoff names a switch
  point when it has one. Handed-off agents get their effort set on each call (medium for
  mechanical work, high for checkers/judges).
