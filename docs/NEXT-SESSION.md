# Next session — kickoff (2026-10-18)

This is the current plan; no other file holds one. Older kickoffs and the 2026-10-08 build trail
were cut on 2026-10-18 (git history keeps them; every result in them is in DECISIONS.md).

## Where things stand (2026-10-18)

The project map is complete on `main` (steps (a) to (f), reviewed in three rounds, eight fix passes
and a wording pass; 1 646 tests, `bench check --replay` green): a newcomer agent reached a verified
Rust unit on liblzg and on zopfli by the docs alone, and the zopfli tool accepted from the map is
byte-identical to the hand-written target. The documentation was then rewritten for a
non-technical reader (2026-10-09) and reshaped for a person in a hurry (2026-10-15: FIRST-HOUR.md,
seven topic pages, a README of four doors), and on 2026-10-18 cleaned up
(docs/reviews/2026-10-18-docs-cleanup-plan.md). The session's answer to the person's question "how close to a usable
beta?" is about two-thirds of one, with reasons in DECISIONS 2026-10-08; the last entries there
hold the rest.

## The person's rules that matter for resuming

- Prove before planning: no new design until something has been run. The person's own run decides
  what comes next.
- Simplicity means usability: easy and intuitive, the command line first, the cockpit runs the same
  acts.
- Ask before any download; test downloads live only in `~/code/ruharness-test-downloads/`.
- Models and effort as in CLAUDE.md. Effort: medium for running and building; ask for high before
  judging what the person's run found or designing anything new, and for medium again after.
- Plain words in every report; commit to `main` and push at each milestone.

## Done (2026-10-18): the cockpit made easy for a newcomer

The session's critique and the newcomer's walk are in docs/reviews/2026-10-18-cockpit-usability-
critique.md; DECISIONS' 2026-10-18 cockpit entry says what the four passes built (one vocabulary
and a Next step on every screen; Units/Features/Speed first and a unit's View in words; the
Project view replacing the text menu; the newcomer's seventeen snags, among them the cockpit
writing the build configuration itself). A reader who had read only docs/topics/cockpit.md reached
every goal in under eight minutes. Open from it: the headless driver (devtools/cockpit-drive) does
not clear the screen when the cockpit goes full-screen and drops key bursts sent as one command
(driver issues, not cockpit ones).

## Done (2026-10-19): the cockpit's colours

The person found the colours hard to read. Now (`crates/harness-tui/src/theme.rs`): ten named
roles (text, muted, heading, accent, good, bad, attention, selected, disabled, border); four
themes — terminal (the default, the terminal's own green/red/yellow/cyan by role, nothing dim, the
person's idea), dark and light (designed, contrast-tested against their backgrounds), mono (no
colour; `NO_COLOR` picks it); `--theme`, `RUHARNESS_THEME`, and a settings file
(`~/Library/Application Support/ruharness/cockpit.toml`, `$RUHARNESS_COCKPIT_CONFIG`) with
`[theme] name` and per-role `[colors]` overrides, bad values refused by name. Rules enforced by a
test over every golden screen: colour only on a state beside its mark or word, one accent per
screen (the Next step label and the recommended item), no dim, no dark grey, no blink. The 41
yellows became 18 attention, 15 bold, 3 muted, 4 plain, 1 accent. Code views lost syntax colours
(keywords bold, comments muted). The headless driver tags the accent as `A`.

## In progress (2026-10-19): sections, status and notices get their own hues; the hand-off pause

The person's screenshot (a green-on-black terminal) showed nearly all text green: the terminal
theme's plain text is the terminal's foreground. One Opus pass adds roles with their own hue in
every theme — section (pane titles, headings), status (the Ready/Last line), notice, label — and
fixes a bug the screenshot showed: a hand-off pause (`project ask` exit 1 with "awaiting
response") was labelled "failed"; it must read "waiting for your answer: <file> — …".

## Queued (2026-10-19): model work in the cockpit goes through the chat, with a sign-in walk-through

The person's "Ask a model" from the Project view ran the command-line hand-off and stopped on a
file nobody writes. Next pass, after the colour pass merges: "Ask a model" becomes "Ask in chat"
with the question pre-typed (the chat agent answers through the harness and the cockpit runs
`project ask` again; the command-line hand-off stays for the CLI); before any model act the
cockpit checks Claude Code is installed and signed in, and if not shows a plain walk-through (open
a new Terminal window, type `claude`, sign in, come back, "Check again"); the same dialog when the
chat's process dies. The person agreed ("sounds good").

## The work, in order

1. **The person's own cold-start run.** docs/FIRST-HOUR.md as written (liblzg), then lz4 through
   the README's "move your own C program" door alone, past one unit, ideally to several promoted
   units, with every stumble written down. Step (g) of the map (record the first accepted tool's
   features before its first unit moves) happens inside it. Fix what the run shows first.
2. **`harness answer` and a live provider on the command line.** `harness answer <unit> <file>`
   wraps a plain file into the hand-off's envelope (the idea in DECISIONS 2026-10-15; decide its
   shape with the person first). A live provider: the `anthropic` and OpenAI-compatible adapters
   exist, but no cloud endpoint has been used (DECISIONS 2026-09-19) and there are no keys here;
   the person decides whether and how.
3. **The scale run on lz4**: many units of one lz4 program moved, promoted and kept green, to see
   what breaks at size (time, disk, the small open items below that are about speed).
4. **Then the briefing's next milestone (M5, the extension proof)**, or what the scale run shows,
   as the person decides.

## Small open items (rechecked against the code on 2026-10-18)

- The ask prompt still asks for fields "in this order" (crates/harness-llm/src/projectask.rs);
  nothing checks the order. Keep it, or drop it and re-record the trace keys.
- Ask's answer line says "in <program> it did not link"; it could add "when the map was made" like
  its neighbour.
- `project accept` link-checks every program again (as long as a map); the link check compiles its
  own objects instead of reusing the map's.
- The map's sandbox profile does not yet limit its reads to an allow-list (PROJECT-MAP-DESIGN §8).
- `state status` builds an include resolver per unit without caching reads (slow on tools with
  many units).
- Attempts, driver validation, `perf`, `bench`, `promote` and `gen-driver` still hash the facts'
  include closure, not the resolver's (safe direction: an attempt shows unbound).
- About 70 recursive tree walks remain in harness-scan (lint, interface, mutate).
- The driver-validation record carries no configuration entry.
- The committed B01 `read_scalefactors_lib` verdict holds the old boundary digest (status does not
  compare it).
- A test in crates/harness-tui/src/view.rs pins a made-up "all 8 checks passed" idle line.

## Once per checkout

The adoption tokens are not committed, so a fresh checkout or worktree asks once:
`harness state status --target targets/zopfli --adopt`. The bench adopts its suite by itself
(`bench check … --adopt`).

## Environment

- The harness binaries are not installed: put a worktree's `target/debug` first on PATH.
- `bench check --replay` needs the gitignored `targets/tractor/.scorer-vendor/` and `.bench/`
  (the person copies them with `cp -cR`; the agent is refused) and a quiet machine; expect
  198 reproduce, 2 expected divergences, 0 problems (about 28 minutes).
- How sessions work here (hand-offs, reviews, mutation checks, driving the cockpit headless,
  the live chat tests): docs/AGENT-BRIEFING.md §18.
- Known flaky tests under load; rerun one alone before calling it a failure:
  `chat::tests::an_exited_leaders_pipes_are_read_first`, `golden_dialog_before_and_after_arming`
  and `the_ready_line_invites_a_click_only_when_one_answers` (harness-tui), chat_e2e's
  `a_deaf_or_stopped_runtime_is_ended_on_quit` and `a_request_is_reviewed_run_and_answered`,
  harness-core's `a_reader_never_makes_a_writer_fail`.
- Many merged `worktree-agent-*` branches and worktrees remain; they can be cleaned.
