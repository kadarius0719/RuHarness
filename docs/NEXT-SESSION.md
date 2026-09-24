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
* **Then ask the user which track comes next.** Their stated direction after M4 (see the
  memory and the DECISIONS M4 roadmap notes): a TUI review cockpit (CLI hardening first:
  writer lock, `migrate --no-promote` + `harness promote`, cancellation, a `--json`
  events mode), a feature-workflow view, and C-vs-Rust performance baselines. The
  briefing's M5 is the extension proof (an external detector plugin + `EXTENDING.md`).
  Smaller carry-forwards:
  - §16 escalation automation + `harness usage`;
  - the `crash-timeout` classifier, which matches substrings of details that can quote
    child stderr;
  - a Linux sandbox;
  - the two deferred replay items: default-run evidence reuse / `--new-trial`, and
    per-prompt score labels.

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
