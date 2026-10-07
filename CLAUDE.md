# RuHarness — read first

1. Read `docs/AGENT-BRIEFING.md` whole: the mission, the architecture, the engineering standards
   and the rules. It is the brief every session works under.
2. Then `docs/NEXT-SESSION.md`: where the last session stopped and what to do next (it points at
   the current plan, today `docs/NEXT-WEEK-PLAN.md`, and `docs/FEATURES-PROGRESS.md` "NOW").
3. `DECISIONS.md`'s last entries hold what was decided and why.

Rules that are never skipped, whatever the task:
- No cloud API keys here: model calls go through the `external` hand-off. Answer hand-offs with
  plain Agent subagents, never Workflow agents. Set `--model` to the model that actually answers.
- Never download anything (a model, a C project, a crate) without asking first.
- Vet every crate before adding it (maintenance, advisories, weight). No bloat, no dead crates.
- Report in plain words: no review codes (M1, N4, …). The person wants to learn from what is said.
- Solo developer: commit to `main` and push at each milestone, no pull requests, never force-push.
- Before a large multi-agent workflow late in a session, check the plan's usage and say what it
  will roughly cost.
- Models (the person, 2026-10-07): the main session may run on Fable; everything it hands off runs
  on Opus — Agent subagents with `model: "opus"`, every Workflow `agent()` call with
  `{model: 'opus'}` (the default inherits the main session's model), hand-off answers with
  `--model claude-opus-5-5`. Ask the person before giving any handed-off work Fable.
