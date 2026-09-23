# Kickoff prompt for the next session

Paste the full **Agent Briefing** (the `# Agent Briefing: Rust Migration Harness`
document) first, then this:

---

Resume RuHarness at `/Users/beaumorton/code/RuHarness`. You are at **M4** (TRACTOR
benchmark). Before doing anything else:

1. Read `DECISIONS.md` — the last three sections are the M3 record and the §17 handoff
   with M4 next-actions and carry-forwards. Then `docs/SCHEMAS.md` (normative contracts),
   `README.md` ("How it works" + command reference), and `targets/zopfli/migration/`
   (the ledger: `plan.toml`, `observer/observations.md`, `units/u001-katajainen/`).
2. Confirm the tree is clean and green: `git status`, then
   `cargo test --workspace` (~290 tests, ~40 s; the e2e runs the whole pipeline in a temp
   copy) and `cargo run -q -p harness-cli -- state status --target targets/zopfli`.
3. Environment facts, verified 2026-09-19: no cloud API keys (api.anthropic.com has never
   been exercised); Ollama is installed at `/opt/homebrew/bin/ollama` with `llama3.2:1b`
   and the derived `llama3.2-1b-32k` — start it with `ollama serve`, point the harness
   at it with `export RUHARNESS_PROVIDERS=$PWD/providers.example.toml`. **Never download
   a model without asking.** Re-check keys/models before assuming.

Process that has worked every milestone (keep it): time-boxed research spike in
subagents → design draft → adversarial design review (3–4 lenses) → parallel
implementation against a frozen core API → adversarial code review → fix pass with
regression tests → DECISIONS.md handoff → commit and push to `main` (solo developer,
no PRs). Every review so far found real blockers — do not skip them. For proofs,
sequence commits so `git diff --stat` between them IS the claim.

M4 in one line: get the TRACTOR public corpus (pinned, checksummed) under `targets/`,
build the missing executor stage the corpus forces — LLM driver/test generation
(§3.5 step 1; today `harness migrate` refuses units without a driver) with C-vs-C
self-validation — run it, and record scores as the regression suite. Report what the
evidence supports, precisely, as the M3 write-up did.
