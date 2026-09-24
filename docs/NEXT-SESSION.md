# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness. Everything is on `main` and pushed (last session ended at the commit
"docs: evidence-first replay decision, gates, README"). Before doing anything else:

1. Read `DECISIONS.md` from "2026-09-23 — M4 complete: final scores" to the end, then
   `docs/REPLAY-DESIGN.md` (§R and §C are authoritative), `docs/ORACLE-HARDENING.md`
   (§A implemented; §B is a DRAFT), and `docs/SCHEMAS.md` ("Observable output",
   "Verification is evidence-first", `superseded.jsonl`, "scores.json — unmarked-UB
   additions").
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~424
   tests), `cargo run -q -p harness-cli -- bench status --suite targets/tractor`.
3. The scorer needs the gitignored `targets/tractor/.scorer-vendor/`. If this worktree
   lacks it, copy it from another worktree with `cp -cR` (APFS copy-on-write) or re-run
   the one-time `cargo vendor` step in `targets/tractor/README.md` (network, crates.io
   only). Optionally copy `targets/tractor/.bench/` too, to skip the scorer rebuild.
4. Baseline check (about 40 minutes, zero tokens):
   `cargo run -q -p harness-cli -- bench check --suite targets/tractor --replay --jobs 6`.
   Expect `198 reproduce (0 conformant, 198 drifted), 1 expected divergence(s), 0
   problem(s)` and `bench check: OK`. "Drifted" is expected: the last prompt edits changed
   every prompt, and replay now re-judges recorded evidence instead of breaking.

Then, in order:

* **Design B — the last known blind spot** (`read_scalefactors`, an FFI-boundary bug in
  verified Rust). Take the DRAFT in `docs/ORACLE-HARDENING.md` §B through the usual cycle:
  research re-check → adversarial design review → implement → adversarial code review →
  fix pass with regression tests. Then re-migrate `read_scalefactors` through the audited
  hand-off, and re-baseline `scores.json`. The translator hint it wants is now an ordinary
  prompt edit: update the fixtures with `RUHARNESS_UPDATE_PROMPT_FIXTURES=1`, review the
  diff, and land it in the same commit.
* **Then the TUI track (user decision, 2026-09-23 — do not ask again).** Start with a
  §15 research spike (current Rust TUI crates and their maintenance, dependency weight,
  how comparable tools present side-by-side code review), then design → adversarial
  design review → implementation. The design brief from the M4 roadmap notes (DECISIONS.md
  and memory): a thin, read-mostly "review cockpit" crate (`harness-tui`) derived from the
  ledger that spawns the `harness` CLI for every write; C beside Rust aligned by function;
  Accept = an explicit promote of a green attempt; Modify = a steer note that becomes a
  new oracle-judged turn (hand edits only as a labelled override); chat lives in Claude
  Code (via a small harness-mcp), not in the TUI. **CLI hardening comes first**, as its
  own milestone: a writer lock, `migrate --no-promote` + `harness promote`, cancellation
  that kills sandboxed process groups, and a `--json` events mode for the TUI to consume.
  Keep it feature-gated and lean (§10.3); vet every crate.
* After the TUI: the feature-workflow view and the C-vs-Rust performance baselines (the
  user's other post-M4 wishes), then the briefing's M5 (external detector plugin +
  `EXTENDING.md`). Carry-forwards to fold in where they fit:
  - §16 escalation automation + `harness usage`;
  - the `crash-timeout` classifier;
  - a Linux sandbox;
  - the two deferred replay items.

Environment (re-check; don't assume):
- There are no cloud API keys, so model calls go through the `external` hand-off.
- Answer hand-offs with plain Agent subagents, never Workflow agents.
- Set `--model` to the model that actually answers.
- Audit every batch with `targets/tractor/handoff-tools` before importing it.
- Never download a model without asking.

Process that works (keep it):
- Run a time-boxed research spike in subagents, and verify its premise by running it end
  to end.
- Write the design, then give it an adversarial design review from 3–4 lenses.
- Implement against the reviewed spec, then run an adversarial code review whose findings
  are verified against the code.
- In the fix pass, add regression tests that fail without the fix (mutation-check the
  rule-guarding ones).
- Hand off in DECISIONS.md, then commit and push to `main`.

Every review so far has found real bugs. Vet every new crate before adding it. No bloat.
