# Kickoff prompt for the next session

Paste the full Agent Briefing (the `# Agent Briefing: Rust Migration Harness` document)
first, then this:

---

Resume RuHarness. You are **closing out M4** (TRACTOR benchmark), then fixing the oracle
hole it found. The work is on branch `claude/rust-migration-harness-02116f` (worktree
`/Users/beaumorton/code/RuHarness/.claude/worktrees/rust-migration-harness-02116f`),
committed but **NOT merged to `main` or pushed**. Before doing anything else:

1. Read `DECISIONS.md` from "2026-09-23 — M4 research spike" to the end. The LAST section
   ("M4 state at session end") is the handoff: results so far, the three diagnosed blind
   spots, and ordered next actions. Then read `docs/SCHEMAS.md` "M4 additions" (normative),
   `docs/M4-DESIGN.md` §R (the reviewed design), `targets/tractor/README.md` (what a score
   means), and `targets/tractor/handoff-tools/README.md` (the model hand-off protocol and
   what its audit proves).
2. Confirm the tree is clean and green: `git status`, `cargo test --workspace` (~393
   tests), `cargo run -q -p harness-cli -- bench status --suite targets/tractor`
   (expect 89 verified, 10 pending, 1 no-units).
3. Check whether `targets/tractor/scores.json` exists. The previous session left a
   sequential `bench score --write` running. If `scores.json` is missing, re-run it
   (now parallel):
   `cargo run -q -p harness-cli -- bench score --suite targets/tractor --write --jobs 6`.
   If a stale `targets/tractor/.bench/LOCK` exists and no run is active, remove it first.
   The scorer needs the gitignored `targets/tractor/.scorer-vendor/`; if it's missing,
   re-create it with the one-time `cargo vendor` step in `targets/tractor/README.md`
   (network, crates.io only; checksums are verified against the committed
   `heldout/Cargo.lock`).

Then, in order:
- **Finish M4:** commit `scores.json`; run `bench check` (expect exit 0: the regression
  suite demonstrated); put the final numbers into the M4 write-up in DECISIONS.md, per
  split, organic vs synthetic, with every n. The previous session's preliminary numbers
  were public 70/77 strict-pass with 0 blind spots, and released-hidden 14/18 with
  3 blind spots. Report precisely what the evidence supports, with the caveats already
  recorded (macOS arm64; a public-vector score with a disclosed contamination risk;
  answering models were Claude subagents audited mechanically; four units escalated to
  the driver-writer's model; not comparable to the First TRACTOR Evaluation Report).
  Update README status, then **merge to `main` and push** (solo-dev rule: no PR).
- **Fix the oracle hole M4 found:** the `differential-driver` check compares stdout only,
  so `014_pow_subfunction` (which reports on stderr) was verified while wrong. Compare
  stderr too. Then re-verify: `bench check` should report that unit as a lost-verified
  REGRESSION, which is the suite working as intended. Re-migrate it and re-baseline.
- Then pick up the carry-forwards in the handoff: FFI-boundary fuzzing (the
  `read_scalefactors` class); flag unmarked-UB vectors by running vectors on the C under
  ASan (the `decorrelate` class); treat confinement setup failures as harness errors;
  §16 escalation automation + `harness usage`; fence `interface` lines in prompts.

**Environment (re-check; don't assume):** no cloud API keys; Ollama with only 1B models,
which can't hold the emission contract. Model calls therefore go through the `external`
hand-off, answered by **plain Agent subagents, never Workflow agents** (workflow agents are
framed with the user's chat message and answered it instead of the task, as recorded in
DECISIONS.md). Set `--model` to the actual answering model. Audit every batch with
`handoff-tools` before importing. Never download a model without asking.

**Process that works (keep it):** time-boxed research spike in subagents → design →
adversarial design review (3–4 lenses) → implementation against a frozen core API →
adversarial code review (security / correctness / measurement / contracts) → fix pass
with regression tests that fail without the fix → DECISIONS.md handoff → commit + push.
Every review in M4 found real bugs. Vet every new crate before adding it: maintenance,
advisories, transitive weight, license. No bloat.

**After M4 (user direction, don't start before M4 is merged):** a thin TUI that wraps the
CLI (ledger-derived; CLI hardening first: writer lock, `--no-promote` + `harness promote`,
cancellation, a `--json` events mode), a "feature workflow" view mapping user-facing
behavior to the code implementing it, and C-vs-Rust performance baselines. Each starts
with its own research spike.
