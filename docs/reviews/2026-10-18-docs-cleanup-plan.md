# Documentation cleanup — plan and surveys (2026-10-18)

The person asked for a review of every document: what is stale, what can go, what can be consolidated, what needs updating; planned here, executed by Opus. Two surveys follow verbatim (the design documents; the operational documents), then the plan.

---

# The design documents: what each is now, and what to do with it

Surveyed 2026-10-18 at a4f4302 (claude/ruharness-resume-ee312a), against the code, `harness --help`
and every sub-help, the cockpit's Help text (crates/harness-tui/src/view.rs) and docs/SCHEMAS.md.

Two facts shape every recommendation below:
- **The program points at these documents.** `harness features --help`, `perf --help` and
  `project --help` each name their design document. The code's comments cite designs by section
  about 500 times (PROJECT-MAP-DESIGN 162, CHAT-PANE 98, FEATURES 57, PERF 52, PROBE-REDESIGN 32,
  TUI 31, REPLAY 24, CLI-HARDENING 24, ORACLE 21, WRAPPER 20, M4 17, MCP 12). Removing or renaming a
  design breaks those pointers. Removal is only cheap for the three documents the code never cites
  (FEATURES-PROGRESS, PROJECT-MAP-ROADMAP, PROJECT-MAP-INVESTIGATION).
- **SCHEMAS does not always win.** For features and speed it says the opposite: "docs/FEATURES-DESIGN.md
  governs" (line 1202) and "docs/PERF-DESIGN.md governs" (line 1310). By SCHEMAS' own words those two
  designs still set the rules, so their status lines must not say "kept only for the reasoning".

Everything surveyed is built, except three pieces of direction that were never meant to be built yet:
the map's interactive picture and model advice (PROJECT-MAP-DESIGN §3.11–§3.12, §8), and Part 2 of
the roadmap note (a fuller C–Rust boundary).

---

## 1. CHAT-PANE-DESIGN.md (1139 lines)

**What it is.** The design of the chat inside the cockpit, plus eight review records (§R–§R8). Built:
the requester label (Build C, 2026-09-27) and the pane itself (Build D, 2026-09-28). The code matches
it: the exact `claude -p …` command line of §1.1 is what `chat/runtime.rs` builds, and Help has a
Chat section.

**Living reference.** None other. SCHEMAS covers only the requester label (lines 556–575), and Help
tells a person how to use the chat. Only this document holds the runtime command line, the chat dir,
the inbox rule, the request-and-answer flow, the continuation permission and the brief, so it is the
reference.

**Untrue now.** A handful. Line 9: "Build D (the pane) is next". It is built, and §R7 in this same
document is its code review. Line 11 says it implements "COCKPIT-WRAPPER-DESIGN.md §10", a section that
still says the chat is not built. It does not mention `--tool` (in cockpit mode on a mapped project,
harness-mcp now takes `--tool ID`).

**Overlaps.** It fully supersedes TUI-DESIGN §9 (the wish for a chat) and COCKPIT-WRAPPER §10 (screen
space set aside for the chat).

**Recommendation: keep as living.** Fix the status line ("Built 2026-09-28; the code governs where
§R4–§R8 say so") and add one line about `--tool`.

## 2. COCKPIT-WRAPPER-DESIGN.md (1090 lines)

**What it is.** The design of the friendly cockpit (file tree, View, Enter menu, armed dialogs,
activity panel, mouse) and its review records. Builds A and B are built (2026-09-25, 2026-09-27).

**Living reference.** The code and its Help for keys and wording. Nothing else holds the rules (the
tree's states, the menu's gates, what arming means): docs/topics/cockpit.md is a page for readers, not
a rule book. So for the parts it covers, this is still the cockpit's rule book.

**Untrue now.** A handful, all because the chat is now built:
- line 5: "The chat pane (§10) is next."
- line 61: "Not in this milestone: The chat pane."
- line 324: greyed "Migrate — ask in chat": "model work happens in chat, which is not built yet". The
  item works now and puts the request in the chat's input.
- §10 (lines 609–621), "The chat pane (later)", and its list of "today's routes".
It also never names the menu items added later (Map the features, Edit the workloads, Measure speed,
Map the project, Ask a model, Accept as a tool). Those live in their own designs.

**Overlaps.** With TUI-DESIGN (its §3 describes this screen again) and CHAT-PANE (§10 here).

**Recommendation: keep as the design of record for the cockpit's screen and dialogs.** Add a status
line, replace §10 with one pointer to CHAT-PANE-DESIGN and update the row at line 324. Of the three
cockpit documents, this is the one to treat as the cockpit's design of record.

## 3. TUI-DESIGN.md (525 lines)

**What it is.** The first cockpit design (keys first, a `y/n` prompt), built 2026-09-24, then amended
in place so its §3 describes the wrapper's screen. It is still the only description of the engine
room:
- the read model (§2);
- the acts' exact commands and gates (§4);
- the child process and signal handling (§4, §6);
- the two CLI additions (§5). SCHEMAS cites §5 as its source for steer attempts and human attempts.

**Living reference.** SCHEMAS "Review acts" for the CLI half of §5; nothing else for §2 and §4.

**Untrue now.** A handful:
- line 25: "Not a chat client (today chat runs in a separate Claude Code session through harness-mcp …
  §9 moves chat into the cockpit)". The chat is inside the cockpit now.
- line 372: §9 is "Not built — recorded to steer the next designs". All of it is built.
- §9's suggested order (lines 425–431) and "harness-mcp … six tools, steer attempts only". There are
  seven tools now, plus a cockpit mode.

**Recommendation: keep as design history with a status line** ("Built 2026-09-24; the screen is
COCKPIT-WRAPPER-DESIGN's, the chat is CHAT-PANE-DESIGN's; §2 and §4 still describe the engine"). Cut
§9 to three lines that point at the two later designs.

## 4. FEATURES-DESIGN.md (1445 lines)

**What it is.** The features design (scenarios checked on every verify, the map, the cockpit's
Features views) and its five review records. Fully built 2026-09-29, then fixed in seven passes.

**Living reference.** Itself. SCHEMAS says "docs/FEATURES-DESIGN.md governs; additive", and
`harness features --help` points here. SCHEMAS gives a short version of the file formats and digests.

**Untrue now.** A handful, outside the parts already marked superseded:
- line 6: "Step 1 (core) is built". Every step is built.
- FEATURES-PROBE-REDESIGN replaces §5.3 (the probed copy) and §5.4 (its runtime). §5.3 carries a
  "Superseded in part" box; §5.4 does not.
- the program digest and the build (lines 240–249, 406, 411) speak only of "every top-level `.c` of
  `source_dir`". A mapped tool uses the file-list form instead (SCHEMAS "The file-list form"). This
  is still true for folder targets, but leaves tools out.

**Overlaps.** FEATURES-PROBE-REDESIGN replaces §5.3–§5.4. FEATURES-PROGRESS repeats its build history
(§R5) and holds one paragraph this document lacks (below).

**Recommendation: keep as living, the design of record for features.** Update the status line, add
the superseded box to §5.4 and one sentence on the file-list form. Carry over FEATURES-PROGRESS's
"Known gaps, judged acceptable" paragraph (its lines 336–342) into §R5 or §13.

## 5. FEATURES-PROBE-REDESIGN.md (989 lines)

**What it is.** The design of the compiler-guided probe, built 2026-09-30 (revision 3), followed by
seven fix passes recorded in §10. Its own status line already says that where the built behaviour
differs, §10 says so and the code governs. The probe version in the code is `compiler-guided-2`, as
§10.1 says.

**Living reference.** SCHEMAS ("map.json", which cites §3.7) for the record, and the code for
behaviour. Only this document states the probe's rules in full (where a note can go, the same-text
check, compile and remove). The code cites it 32 times.

**Untrue now.** A handful at most, all already flagged in §10. §3.8 describes gcc runs "on the Linux
job". That CI job exists but has failed since 2026-09-17, so the gcc path has never been proven.

**Recommendation: keep as design history with a status line** that says it replaces FEATURES-DESIGN
§5.3–§5.4 and is the reference for the probe's rules. Merging its 989 lines into FEATURES-DESIGN would
make neither document easier to read.

## 6. FEATURES-PROGRESS.md (344 lines)

**What it is.** It is named as the progress tracker for features. In fact it is a session log of
three pieces of work in one "Open … resume here" section: the features build (lines 1–31), then the
probe review, then the speed work's build and fix passes.

**Living reference.** DECISIONS has an entry that closes each piece of work (2026-09-29, 10-01, 10-03,
10-07), and NEXT-SESSION says where things stand.

**Untrue now.** Most of it, read as a statement of the present:
- line 33: "NOW (2026-10-07) … Next: fix pass 3, Part 11 on a quiet machine, the map design review".
  All of it is done, and the map is designed, built and complete (DECISIONS 2026-10-08).
- line 42: "NOW (2026-10-01 …): BUILD the C-vs-Rust performance baselines". Built and merged
  2026-10-03.
- line 158: "Branch `claude/rust-migration-harness-f7665f` … NOT yet merged to main or pushed."
- line 151: the project map is "recorded 2026-10-01, not designed".
- line 27: "960 workspace tests pass". The memory file says 1 646 now.
Yet CLAUDE.md line 6 still tells every session to read this file's "NOW", and NEXT-SESSION.md lines
395 and 417 do too.

**Recommendation: remove.** First carry the "Known gaps, judged acceptable" paragraph (lines 336–342)
into FEATURES-DESIGN §R5, and change the pointers in CLAUDE.md line 6 and NEXT-SESSION.md. Everything
else is already in DECISIONS (the closing entry of each piece of work) or in FEATURES-DESIGN §R5 and
PERF-DESIGN §10, and git history keeps the commit hashes. Nothing in the code cites it.

## 7. M4-DESIGN.md (443 lines)

**What it is.** The design of the TRACTOR benchmark and driver generation, reviewed 2026-09-23. Its §R
governs; §0–§9 are the draft that was reviewed, kept "so the reasoning survives". Built: `gen-driver`,
`bench vendor|verify-corpus|init|status|score|check`, and the driver-shape check.

**Living reference.** SCHEMAS "M4 additions" (lines 707–1025) for contracts, targets/tractor/README.md
for the suite, and DECISIONS for the why.

**Untrue now.** Dozens in §0–§9, as you would expect of a superseded draft:
- line 233: the layout `corpus/ # byte-identical subset of upstream`. The suite has `cases/` and
  `heldout/` instead.
- lines 394–398: the scores file with `"totals":{"cases":80,…}` and `"toolchain"`. The real file has
  totals per split (80 public, 20 hidden), `scorer_lock` and `environment`.
- line 387: the runner is built "in `corpus/`".

**Recommendation: keep as design history** with a status line ("Built September 2026; the reference
is SCHEMAS 'M4 additions' and targets/tractor/README.md; §0–§9 are the draft from before the review").

## 8. ORACLE-HARDENING.md (720 lines)

**What it is.** Two oracle designs written after M4. A excuses test inputs that make the C itself
break memory rules without being marked (a sanitized C run in scoring). B is the boundary check. Both
are built: A with its revision A.2 (`-fbounds-safety` beside ASan), and B built and calibrated
2026-09-23 (`bench boundary`, `[unit.oracle] boundary = true`).

**Living reference.** SCHEMAS "Oracle checks added to every verify" and "The boundary check"
(lines 739–853), and DECISIONS 2026-09-23 for the calibration.

**Untrue now.** A handful. Line 3, the status line, still says "B designed and adversarially
reviewed", but B is built. §A's "Mechanism" (lines 29–47) uses ASan only; A.2 replaces part of it and
says so. §B.R carries the review's changes to B and governs over the text it amends.

**Recommendation: keep as design history** with a status line ("A and B built 2026-09-23; the
reference is SCHEMAS; §A.2 and §B.R govern their sections"). It explains in full what B does not
prove (§B.9), which SCHEMAS only summarises. That alone is worth keeping.

## 9. CLI-HARDENING.md (388 lines)

**What it is.** The design of the writer lock, `--no-promote` with `promote`, cancellation and the
`--json` events. Built 2026-09-24; "the text below is the design as built".

**Living reference.** SCHEMAS "CLI hardening" (lines 1026–1128) has every contract; DECISIONS has the
why.

**Untrue now.** A handful, all small and dated:
- line 26: "the workspace `rust-version` moves from 1.85 to 1.89". It is 1.90 now.
- line 30: `exec::scrubbed_command` is "the only `Command::new` sites outside tests". The speed
  launcher's compiler check (perf/launcher.rs line 172) is another.
- line 384: §6 "Open questions carried into implementation". All settled.

**Recommendation: keep as design history** with a status line. Apart from the reasoning (why
`flock`, why the open protocol), almost everything in it is also in SCHEMAS. The reasoning is why it
is worth keeping.

## 10. REPLAY-DESIGN.md (220 lines)

**What it is.** The design of evidence-first replay and the prompt lock. Built; §R and §C govern over
§1–§11.

**Living reference.** SCHEMAS (lines 613–662: verification is evidence-first, `superseded.jsonl`).

**Untrue now.** Dozens in §1–§11, which §R supersedes, as the status line warns:
- line 8: "Replay today re-renders every turn's request from HEAD". That is the old behaviour this
  design replaced.
- lines 58 and 117: "`harness supersede <UNIT>`". No such command exists; to supersede an attempt, a
  person writes a line in `superseded.jsonl` by hand (§R-7).
- line 92: `PROMPT_EDITION` and `prompt_edition`. §R-10 cut both, and neither is in the code.

**Recommendation: keep as design history** with a status line ("Built; the reference is SCHEMAS;
read §R and §C first"). It is short and explains what replay does and does not prove (§10).

## 11. PERF-DESIGN.md (1162 lines)

**What it is.** The speed design (`harness perf`): revision 5 plus 30 build notes (§10). Built,
reviewed and fixed in four passes; merged 2026-10-03.

**Living reference.** Itself, by SCHEMAS' own words ("PERF-DESIGN.md governs"), and
`harness perf --help` points here. SCHEMAS "C-vs-Rust speed" holds the files, tokens and CLI.

**Untrue now.** A handful, but they matter:
- line 3: "Status: revision 5 … to be built". It is built and merged.
- line 284: "On a platform other than macOS and Linux, `perf run` refuses by name". Linux is refused
  too ("perf runs on macOS only for now — the Linux launcher is not built yet"; SCHEMAS line 1315).
  Its 25 Linux passages describe a launcher that does not exist.
- line 797: it lists `cant-tell-busy` among harness-mcp's answers. The built wording never gives it;
  a busy computer is only mentioned as a detail.
- line 925: `perf-recipe-1`. It is `perf-recipe-2` now, as its own note 31 and SCHEMAS say.

**Recommendation: keep as living, the design of record for speed.** Fix the status line and lines
797 and 925. Add one box at §3.3: "Linux: not built; `perf run` refuses it — the Linux text is a
plan".

## 12. PROJECT-MAP-DESIGN.md (1238 lines)

**What it is.** The project map design, revision 2.2, built in steps (a)–(f) on 2026-10-08 and
complete. The flags in `harness project map|ask|accept --help` match §3.8. Not built, on purpose: the
interactive picture (§3.11, "direction to investigate"), model advice (§3.12) and the open items in §8.

**Living reference.** SCHEMAS covers the contracts at length (config.toml, the map file, accept, ask,
mapped tools, `--tool`, adoption: lines 1536–2365). The design still holds:
- the walk's rules (§3.1);
- the security reasoning (§3.9);
- what the map leaves out (§6);
- the person's decisions (§7).
The code cites it 162 times, and `project --help` points here.

**Untrue now.** A handful:
- line 7: "ready to build in §5's order". It is built.
- line 22: "Today the harness migrates one folder of one program, and the person gathers that folder
  by hand". True before the map; now the map does it (the guide's Part 12).
- line 683: "today's comment that serde does so is wrong; a small fix of its own". The fix shipped in
  step (a).

**Overlaps.** PROJECT-MAP-ROADMAP: this design grew out of its Part 1, and cites its Part 2 at lines
10, 846, 850 and 1054. PROJECT-MAP-INVESTIGATION: the spike behind §2.

**Recommendation: keep as living, the design of record for the map.** Update the status line ("Built
2026-10-08; contracts in SCHEMAS; §3.11, §3.12 and §8 still open"), and take in Part 2 of the roadmap
note (below).

## 13. PROJECT-MAP-INVESTIGATION.md (157 lines)

**What it is.** A spike record: the closure experiment on zopfli and on a made-up project
(2026-10-02), then on lz4 and liblzg downloads (2026-10-07). devtools/README.md points at it for
closure.py.

**Living reference.** DECISIONS 2026-10-07 summarises the spike and the person's answers, and the
design quotes its numbers (lz4: 48 files, 33 programs, 2.2 s).

**Untrue now.** A handful:
- line 4: the script "is `scratchpad/pmap/closure.py`". It is `devtools/project-map/closure.py` now,
  as the later section says.
- line 29: "liblzg is not here (the testing guide downloads it)". It has since been downloaded, into
  ~/code/ruharness-test-downloads/.

**Recommendation: keep as design history** with a status line ("Spike record, 2026-10-02 and
10-07; the design is PROJECT-MAP-DESIGN"). It is short and holds the measurements the design relies
on, and removing it would break the pointer in devtools/README.md.

## 14. PROJECT-MAP-ROADMAP.md (118 lines)

**What it is.** A roadmap note from 2026-10-01 in two parts. Part 1: map a project. Part 2: a fuller
C–Rust boundary (shared code in several programs, calls both ways, several Rust units in one program,
shared data, the end state).

**Living reference.** For Part 1, PROJECT-MAP-DESIGN, which is built. Part 2 is written nowhere else,
and the design refers to it by name four times.

**Untrue now.** Most of it, about the present:
- line 3: "Status: idea, not designed yet."
- lines 15–28, "What the harness assumes today":
  - "No build system is read". `project ask --build` reads them now.
  - "`harness.toml` names one `source_dir`". Version 2 has the file-list form.
  - "Every `.c` in that folder is one program". No longer the only layout.
- line 116: "Answer at the time: no — it needs a `harness.toml` and one folder".
Still true: "Only leaf units migrate".

**Recommendation: fold into PROJECT-MAP-DESIGN, then remove.** Carry Part 2 (lines 82–100, five
numbered items) into PROJECT-MAP-DESIGN §8 "Open for later" as "The fuller C–Rust boundary", and point
the design's four references to "Part 2 of the roadmap note" there instead. The design answers Part 1
and the hard questions. NEXT-WEEK-PLAN.md line 153 also names this note.

## 15. MCP-DESIGN.md (388 lines)

**What it is.** The design of harness-mcp: seven tools when run on its own, a cockpit mode (§R6) and
the requester label (§R5). Built 2026-09-24, amended 2026-09-27 and 09-28.

**Living reference.** Itself. SCHEMAS has no harness-mcp section of its own, only the requester label,
the speed fields (lines 1470–1507) and the adoption refusal. The README's harness-mcp section covers
setup only.

**Untrue or missing now.** A handful:
- the flag table (lines 55–61) lacks `--tool ID` and `--cockpit`. The usage line in policy.rs has
  both.
- line 54: "`--target DIR` the default target (required)". Still true, but a mapped project now
  needs `--tool` as well, and a ledger made on another computer is refused as "not-adopted" (harness-mcp
  never adopts one).
- `harness_status` and `harness_unit` gained `features` and `speed` fields that §3 does not list.
- §7 "Later" lists a fresh-translate tool "once a ledger label records the requester". The label
  exists, and the server in cockpit mode has `harness_migrate`.

**Recommendation: keep as living.** Add `--tool`, the adoption refusal and the two fields to §1 and
§3, and update §7. It is the only full statement of the tool contract.

---

## The other two files

**targets/tractor/README.md.** The numbers and paths are right: 100 cases (80 public, 20 hidden), the
pinned upstream commit, `heldout/patches/PATCHES.md`, the `-fbounds-safety` + ASan pass, every `bench`
command and `--suite`. One gap: "Reproducing from a cold checkout" (lines 26–46) runs `bench status`,
`score` and `check` and says nothing about adoption. Those three check the suite root before they run
(SCHEMAS line 1599), so on a fresh clone they are refused until the suite is adopted once with
`--adopt`. Add one line.

**devtools/README.md.** The paths and flags match the scripts (cockpit.py's `<workdir> <rows> <cols>
-- argv`, k.py's `--until/--gone/--timeout/--quiet`, closure.py's arguments). Two gaps:
- it does not mention `devtools/guide/gates.py`, the testing guide's structure check, used in the
  2026-10-09 rewrite;
- its example uses `targets/lzg`, which exists only after Part 1 of the testing guide. Worth saying
  so.

---

## Summary

| Document | Recommendation | Why, in one line |
|---|---|---|
| CHAT-PANE-DESIGN | keep as living | The only full statement of the chat; its status says the pane is "next" though it is built |
| COCKPIT-WRAPPER-DESIGN | keep as living (the cockpit's design of record) | The rule book for the screen, menu and dialogs; fix §10 and the "not built yet" item |
| TUI-DESIGN | keep as history, status line | The engine room (read model, commands, signals) is still right; §0 and §9 are outdated now the chat is built |
| FEATURES-DESIGN | keep as living (the features design of record) | SCHEMAS says it governs; take in the known-gaps paragraph from PROGRESS |
| FEATURES-PROBE-REDESIGN | keep as history, status line | Replaces FEATURES §5.3–§5.4; its §10 already says the code governs |
| FEATURES-PROGRESS | remove (after carrying one paragraph) | An outdated session log of three pieces of work; its "NOW" is eleven days and one milestone old, yet CLAUDE.md sends sessions to it |
| M4-DESIGN | keep as history, status line | §R governs; §0–§9 are a draft with a different suite layout |
| ORACLE-HARDENING | keep as history, status line | Both designs are built; the status still says B is only designed |
| CLI-HARDENING | keep as history, status line | Every contract is in SCHEMAS; it keeps the reasoning |
| REPLAY-DESIGN | keep as history, status line | §R and §C govern; §1–§11 name a command and fields that were cut |
| PERF-DESIGN | keep as living (the speed design of record) | SCHEMAS says it governs; the status says "to be built", and the Linux text describes something the program refuses |
| PROJECT-MAP-DESIGN | keep as living (the map's design of record) | Built; holds the walk's rules and the person's decisions; should take in the roadmap's Part 2 |
| PROJECT-MAP-INVESTIGATION | keep as history, status line | A short spike record with the measurements; devtools points at it |
| PROJECT-MAP-ROADMAP | fold into PROJECT-MAP-DESIGN §8, then remove | Says "not designed yet"; only Part 2 (the C–Rust boundary) is still worth keeping |
| MCP-DESIGN | keep as living | The only full tool contract; lacks `--tool`, adoption, and the features and speed fields |

Pointers to change along with these steps: CLAUDE.md line 6 and NEXT-SESSION.md lines 395 and 417
(for FEATURES-PROGRESS); NEXT-WEEK-PLAN.md line 153 and PROJECT-MAP-DESIGN lines 10, 846, 850 and 1054
(for the roadmap). No help text or code comment cites either of the two documents proposed for removal.


---

# The working documents: a survey (2026-10-18)

At a4f4302. Checked against `harness --help` and every sub-help, `harness-tui --help`,
`harness-mcp --help` and the code's refusal sentences. Line numbers are the files' own.

## CLAUDE.md (24 lines)

**For:** the first thing every session reads: the reading order and the never-skipped rules.

**Stale pointers.**
- Lines 5-6: "it points at the current plan, today `docs/NEXT-WEEK-PLAN.md`, and
  `docs/FEATURES-PROGRESS.md` "NOW"". Not true. NEXT-WEEK-PLAN.md is the plan for the week of
  2026-10-07, written 2026-10-02; every step in it is done. FEATURES-PROGRESS "NOW (2026-10-07)"
  says the next work is fix pass 3, Part 11 and the map design review, all done since. The
  current plan is the "The work, in order" list at the top of NEXT-SESSION.md (lines 63-74).
- Lines 22-23: the effort "switch points in docs/NEXT-SESSION.md". The current kickoff has none;
  it only says "Models and effort as in CLAUDE.md" (line 4), so each file sends you to the other.
  The only switch points are in the 2026-10-07 kickoff (NEXT-SESSION lines 299-310), about work
  long finished.

**Recommend: update.** Replace lines 5-6 with: "Then `docs/NEXT-SESSION.md`'s top section: where
the last session stopped and the work, in order." Replace "at the switch points in
docs/NEXT-SESSION.md" with "when the work turns from building to design judgement (and back);
the kickoff names them when it has any". Add one line: "`docs/README.md` says what every document
is for." Rules otherwise true.

## docs/AGENT-BRIEFING.md (329 lines)

**For:** the founding brief, read whole by every session. Written before M0 and barely touched
since: it describes the plan of 2026-09-17, not the program.

**Untrue about the program today.**
- Lines 32, 34, 47, 63, 65, 71, 189, 205: the ledger is `facts.db` (SQLite) and `plan.yaml`. It
  is `migration/facts.jsonl` and `migration/plan.toml` (decided at M1, DECISIONS line 126-131;
  `harness --help`: "Scan the target and regenerate migration/facts.jsonl", "Reconcile
  migration/plan.toml"). No SQLite crate is in Cargo.lock. Line 140's `serde_yaml` and line 141's
  `rusqlite` were never added.
- Lines 31-36: the ledger list misses `observer/`, `features/`, `perf/`, `map/`, `tools/` and
  the mapped-tool layout (`migration/tools/<id>/`), all in SCHEMAS "The ledger's fixed names".
- Lines 68-76, "Current milestone plan": no status. M0 to M4 are done; M5 is next; the six tracks
  built since M4 (cockpit, chat, harness-mcp, features, speed, project map) are not mentioned.
  The README's status table (README lines 159-167) is the true one.
- Lines 99-101, "First actions": "Read the ledger if it exists (`migration/`) ... If it doesn't
  exist, you are at M0." The RuHarness repo has no `migration/` at its root, so a literal reader
  concludes "M0". This section is about a target, but reads as about the harness.
- Lines 110-117: the crate list misses `harness-tui` (the cockpit, also a library harness-mcp
  reuses); `harness-mcp` is a crate of its own, not "an optional feature".
- Line 176: `PlannerStrategy` is no trait in the code (traits that exist: `LanguageFrontend`,
  `Detector`, `ProviderAdapter`, `OracleStrategy`, plus `Linker` and progress traits).
- Lines 190, 75, 97: `EXTENDING.md` does not exist (it is M5's deliverable; say so).
- Lines 207-222, 226-228, 258-267: storage profiles (`[state] profile`), `harness state gc`,
  `[state.runtime] kind`, `[budget]` and `harness usage` are written as things the harness
  provides; none is built (`harness state --help` lists only `status`; DECISIONS carries
  "`harness usage`" and escalation as carry-forwards since M3). `sync-runtime` is a top-level
  command, not `harness state sync-runtime` (line 226).
- Line 284 holds a stray `</pasted_content id="d005">`, line 286 a stray opening tag, and lines
  287-329 are a pasted copy of the 2026-09-30 kickoff ("Resume RuHarness — finish the
  feature-workflow view...", "952 tests", "Then the C-vs-Rust performance baselines"). It is
  wrong in every particular today and is read by every session as part of "the brief".

**Documents and rules:** it names no project documents beyond DECISIONS.md and EXTENDING.md;
its rules still match practice (`forbid(unsafe_code)` everywhere, `anyhow` only in harness-cli).

**Recommend: update.** (1) Delete lines 284-329; move "Environment" (301-315) and "Process that
works" (317-327), merged with NEXT-SESSION's newer copies (359-369, 451-513) and its flakes, into a
new section 18, "How sessions work here". (2) Section 3.1 and the M1 line: the real names
(`facts.jsonl`, `plan.toml`; DECISIONS 2026-09-17) and a pointer to SCHEMAS' fixed names. (3)
Section 5: M0-M4 done with dates, the tracks since, M5 next. (4) Section 9: "for a target"; for
the harness, "read CLAUDE.md". (5) Section 10.1: add `harness-tui`; `harness-mcp` is a crate. (6)
Sections 13.1, 14.2-14.3, 16.3: one "Not built yet:" line each. Keep the spec's voice.

## docs/NEXT-SESSION.md (513 lines)

**For:** the hand-off from one session to the next. Read second by every session.

**Shape.** Lines 1-85: the current kickoff (10-08, plus 10-09 and 10-15 "Done" paragraphs).
89-287: the 10-08 build trail; 290-513: four older kickoffs (10-07, week of 10-07, 09-30, older).

**Stale pointers and claims in the current part (lines 1-85).**
- Lines 14-15: "README 'Start from your own C project'" is gone (the 10-15 reshape); the door is
  now docs/topics/map.md. Lines 37-48 describe a 641-line README with "Quick start" and "How it
  works, in eight points"; it is now 225 lines with four doors, and Quick start moved to
  FIRST-HOUR.md. The 10-15 paragraph says so, but the 10-09 paragraph still reads as current.
- Line 72: "the earlier passes' leftovers (below in the trail)". The trail mixes closed and open
  items: the wording pass c3a85d2 closed `gitignore_text`'s first line, the workloads blocker's
  wording, the perf log path and `Adoption::describe` (trail lines 215-224 still list them), and
  `scan_target` printing `ambiguous_lines` and `cmd_detect` calling `detect_reporting` (trail
  lines 187-188) are both in main.rs now (lines 954, 1455). A resuming session cannot tell which
  are open.

**What a resuming session still needs.** The top section (where things stand, the beta answer,
the work in order, once-per-checkout adoption, environment, flakes); the lasting environment and
process notes (now only in the old kickoffs, lines 359-369, 451-513); and one short list of open
leftovers, rechecked. Small open items I could confirm: the ask prompt's "in this order"
(projectask.rs lines 1005, 1030); `accept` re-linking every program; the map profile's read
allow-list; "reuse the map's objects" not done; status building a resolver per unit without
caching; attempts/driver-validation/perf/bench/promote/gen-driver still hashing the facts'
closure; ~70 recursive walks in harness-scan; driver validation carrying no configuration entry;
B01 `read_scalefactors_lib`'s old boundary digest; the view.rs test pinning a made-up idle line.

**Already in DECISIONS (compare).** Every result and decision of lines 89-370: the map's steps
and fix passes (DECISIONS lines 2923, 2954, 2994, 3040, 3104, 3135), the speed passes and map
revision 1 (2756, 2796, 2860), the three docs rewrites (3182, 3203, 3222).
**Not in DECISIONS:** the wording pass c3a85d2 (no entry; only NEXT-SESSION line 68 and the
commit message), the "about two-thirds of a usable beta" answer (lines 18-20), and the
`harness answer <unit> <file>` product candidate (lines 59-61).

**Recommend: cut** lines 87-513 (git keeps them). Before cutting: add a short DECISIONS entry for
the wording pass (its eight sentences, from the commit message) and the beta answer and `harness
answer` candidate; move the environment and process notes to the brief's new section 18; put the
rechecked open leftovers (list above) under a heading "Small open items" in the kickoff. Then
rewrite lines 1-85 as one kickoff: where things stand (one paragraph, pointing to DECISIONS' last
entries), the work in order, effort switch points if any, once per checkout, flakes. Target
under 80 lines.

## docs/NEXT-WEEK-PLAN.md (173 lines)

**For:** the plan for the week of 2026-10-07. No longer read for anything current.
**Stale.** All of it: steps 1, 2, 3, 4, 6 and 7 are done (its own status line 17-23 and DECISIONS
2026-10-03, 2026-10-07, 2026-10-08). Section 5's draft DECISIONS entry was pasted as DECISIONS
2026-10-03 (line 2676). Line 3 names the branch `claude/rust-migration-harness-f7665f` and main
at 56cd090. It is named as "the current plan" by CLAUDE.md line 6 and listed in README line 203.

**Recommend: cut** (delete; git keeps it). Nothing in it is unique: the macOS-only decision and
the red Linux CI job are in DECISIONS line 2711-2716. Remove it from CLAUDE.md and README's
layout. Two other files mention it as history only (FEATURES-PROGRESS line 146,
PROJECT-MAP-INVESTIGATION line 84): leave those.

## docs/FEATURES-PROGRESS.md (344 lines)

Named by CLAUDE.md for "NOW"; its "NOW (2026-10-07)" (lines 33-38) lists work done by 10-08; the
rest is history (in FEATURES-DESIGN §R5 and DECISIONS). **Recommend: cut** "NOW", first line "History
of the features track; current work is in docs/NEXT-SESSION.md".

## DECISIONS.md (3 236 lines, 70 entries)

**For:** the engineering log, newest last. Read (last entries) by every session; the place where
"why" lives.
**Structure.** Dated `##` entries: M0-M4 and oracle work (09-17 to 09-24, lines 5-1211); TUI,
CLI hardening, MCP, cockpit, chat (09-24 to 09-28, -2365); features and probe (09-29 to 10-01,
-2675); speed (10-03 to 10-07, -2859); the project map (10-07 to 10-08, -3181); docs (10-09 to
10-15, -3236).

**An index would help: yes.** Seventy entries in 3 236 lines, many titles alike ("state at
session end", "complete"). An index at the top (date, title, one line) lets a session find "why
plan.toml" or "why macOS only" without reading the whole log. Two titles say "READ THIS
FIRST" (lines 577, 803) and are long superseded: the index should show that, or the headings lose
the phrase.

**Contradictions.** Each later change I checked says so in words: plan.toml over plan.yaml and
facts.jsonl over facts.db (line 126-131, against the brief); `build = false` (revision 2.1, line
2906, then "Decided: ... not added", line 2945); the committed adoption tokens (line 2935, then
"no longer committed", line 3028). One soft one: 2026-10-09 (line 3207) "the README is the front
door and owns Quick start"; 2026-10-15 (line 3231) cuts the README to four doors and Quick start
goes to FIRST-HOUR.md without saying the earlier ownership rule changed. Also the 2026-10-08
entry (line 3175-3177) sends "open, in words" to NEXT-SESSION, which the cut above must keep.

**Recommend: update.** Add the index (date → title → one line) under the header; add the wording
pass entry and the beta/`harness answer` note; one sentence in the 10-15 entry: "This replaces
10-09's rule that the README owns Quick start: FIRST-HOUR.md owns it now."

## docs/SCHEMAS.md (2 365 lines)

**For:** the normative contract of every ledger file and the CLI; read by a developer changing
any reader or writer, and cited by refusals in the code.

**Checked against the program:** adoption, fixed names, `config.toml`, `project map`, the map
file and "what changed", `accept`, `ask`, mapped tools and `--tool`, `harness.toml` v2,
`sync-runtime`, the writer/lock tables, the envelope (lines 37-55, 1536-2365). About twenty quoted
refusals grepped: all in the code word for word. `--tool` is on every command listed (each help
checked); `project map` takes none; ask's default `claude-sonnet-5` matches `--help`.

**Untrue or stale.**
- Lines 2325-2328: `-o` and "the `-M` family" are listed among flags refused as "not one the
  harness passes". Since the wording pass, `-c`, `-o…`, `-MD`, `-MF…` get their own sentence:
  "the flag `-c` is added by the harness itself: remove it" (crates/harness-core/src/config/
  flags.rs lines 125-138). SCHEMAS never says it.
- The code's refusal sends the reader to `docs/SCHEMAS.md, "harness.toml, file-list form"`
  (flags.rs line 129); no heading has that name (it is "The file-list form (`schema_version =
  2`)" under "`harness.toml`, read version first", line 2277). Fix either side; the doc heading is
  cheaper.
- Lines 224-230: "facts.db (deferred to M2 — design frozen on paper, no code at M1) ... Ships with
  the first query consumer (M2 detectors or the MCP server)". Both shipped without it; nothing
  builds `facts.db`. Say "not built: no consumer has needed it".
- Lines 199-205 (CLI contract): `scan` is described only by "its walk of `source_dir`"; a
  file-list tool has no `source_dir`. One clause pointing at "Every reader reads the file-list
  form" (line 2335) is enough.
- Minor: config.toml "Three lines are enough" (line 1647) against topics/map.md's "four lines"
  (the header line counted); not a program fact, but say the same number.

**Recommend: update** with those four edits. Leave the rest.

## README.md, the reference half (lines 53-225)

**For:** a developer looking up a command, flag, the cockpit's and harness-mcp's setup, status,
layout and dependencies.
**Untrue or stale.**
- Line 87: "With two or more tools and no `--tool`, the command asks you to pick." The command
  line refuses (exit 1): "<root> has N mapped tools and no harness.toml of its own; pick one with
  --tool (t-a, t-b)" (harness-core config.rs). Only the cockpit lists them to pick.
- Line 99: "Writing commands hold a writer lock on `migration/.lock`": for a tool it is
  `migration/tools/<id>/.lock`, and `project map`/`accept`/`sync-runtime` take
  `migration/map/.lock` first (SCHEMAS line 2353-2357).
- Line 133: "Install it as in docs/FIRST-HOUR.md". FIRST-HOUR installs only `harness-cli` (line
  27); nothing in README, FIRST-HOUR, the topics or the tutorial installs `harness-tui` or
  `harness-mcp` since Quick start was removed. TESTING-GUIDE lines 611-634 do. Line 119-120
  ("reinstall it with harness-cli") assumes an install step no front page has.
- Lines 198-203: the layout lists NEXT-WEEK-PLAN.md (to be cut) and has no docs/README.md.
- Line 213: "CI (GitHub Actions, macOS + Ubuntu)": the workflow has both; DECISIONS line 2714
  says the Linux job has been red since 2026-09-17. I did not run CI; say "Ubuntu (red since
  2026-09-17, a newer clippy lint)" if still so.

**Recommend: update** those five lines; add the three `cargo install` lines (from the guide's
Step 0.19) under "The cockpit" or link Step 0.19.

## docs/TUTORIAL.md (500 lines) — contradictions only

- Line 83: "the same six the README draws: scan, plan, driver, migrate, verify, accept". The
  README no longer draws them (cut on 10-15). Say "the same six the guide walks".
- Line 247: "The README shows how to set the connector up": half true (the `.mcp.json` is there;
  its install step points at FIRST-HOUR, which has none; fixed by the README edit above).
- Line 401 (appendix): "Every one takes `--target <folder>`": `bench` takes `--suite`, and
  `project map` takes no `--tool`. Add "(`bench` takes `--suite`)".
- The rest of the appendix matches the program. **Recommend: update** (three phrases).

## docs/topics/*.md (seven pages) — contradictions only

No sentence found untrue (check names, the eight-character key, the envelope, the default
model, ids and refusals all match). Small: map.md line 17 "four lines" against `project --help`'s
"three lines"; cockpit.md assumes `harness-tui` is installed and no page says how (the README fix
covers it). **Recommend: leave** (or "three" in map.md).

## The index: is there a document saying what each document is for?

No. CLAUDE.md gives an order; README lines 198-203 list names only (22 files, reviews/, topics/).
**docs/README.md should hold** one table, rows "file — for whom — what for — current or history",
in four groups: a person using RuHarness (FIRST-HOUR, topics/, TUTORIAL, TESTING-GUIDE); a
developer (SCHEMAS as the contract; each design note — TUI, COCKPIT-WRAPPER, CHAT-PANE, MCP,
CLI-HARDENING, FEATURES, FEATURES-PROBE-REDESIGN, PERF, PROJECT-MAP, ORACLE-HARDENING, REPLAY, M4 —
marked "governs" or "history" with its authoritative section); a session (CLAUDE.md, the brief,
NEXT-SESSION, DECISIONS, in reading order); history (reviews/ with one line per report,
FEATURES-PROGRESS, PROJECT-MAP-ROADMAP, -INVESTIGATION; devtools/README.md for test scripts). One
rule: "a new document gets a row here, or it is not finished". Under 60 lines.

## Recommendations

| Document | Recommendation | Why, in one line |
|---|---|---|
| CLAUDE.md | update | Its "current plan" pointers name two finished documents; effort points point in a circle |
| docs/AGENT-BRIEFING.md | update | A pasted 09-30 kickoff and stray tags sit inside it; facts.db/plan.yaml, milestones and unbuilt commands read as true |
| docs/NEXT-SESSION.md | cut lines 87-513, rewrite 1-85 | 430 lines of trail already in DECISIONS mix closed and open items; keep one kickoff and one open list |
| docs/NEXT-WEEK-PLAN.md | cut | Every step done; its draft entry is DECISIONS 2026-10-03 |
| docs/FEATURES-PROGRESS.md | cut its "NOW", mark as history | "NOW" names work finished on 10-08 |
| DECISIONS.md | update | Add an index, the missing wording-pass entry, one line on the README rule change |
| docs/SCHEMAS.md | update | The `-c`/`-o`/`-M` refusal and the section name the code cites; facts.db "ships at M2" |
| README.md (reference) | update | "asks you to pick" is untrue on the command line; MCP/cockpit install points to a page without it |
| docs/TUTORIAL.md | update | "the six the README draws" and "every one takes --target" (three phrases) |
| docs/topics/*.md | leave | Matched the program; at most "four lines" → "three" in map.md |
| docs/README.md (new) | write | Nothing says what each of 40-odd documents is for or whether it is current |


---

# The plan (the session, 2026-10-18)

## Three kinds of document, said out loud

- **Living**: what the program is today. README, FIRST-HOUR, the topic pages, the guide, the tutorial,
  SCHEMAS (normative), AGENT-BRIEFING, NEXT-SESSION, DECISIONS, and the designs SCHEMAS or the help
  screens still name as governing (FEATURES-DESIGN with the probe redesign beside it, PERF-DESIGN,
  PROJECT-MAP-DESIGN, MCP-DESIGN, CHAT-PANE-DESIGN, COCKPIT-WRAPPER-DESIGN).
- **Design history**: written before the thing was built, kept for the reasoning, with a status
  line at the top saying when it was built and where the truth lives now (TUI-DESIGN, M4-DESIGN,
  ORACLE-HARDENING, CLI-HARDENING, REPLAY-DESIGN, PROJECT-MAP-INVESTIGATION).
- **Records**: the review bundles under docs/reviews, never edited.

**Names stay.** The code's comments cite the designs about 500 times and the help screens name
them, so no design document is renamed or moved; a status line at its top does the work.

## What goes

- `docs/NEXT-WEEK-PLAN.md`: the week of 2026-10-07, every step done, its log already DECISIONS'
  2026-10-03 entry.
- `docs/FEATURES-PROGRESS.md`: a tracker whose "NOW" is finished work and whose logs are in
  DECISIONS; its one unique paragraph ("Known gaps, judged acceptable") moves into FEATURES-DESIGN
  first.
- `docs/PROJECT-MAP-ROADMAP.md`: its Part 2 (shared code across programs, calls both ways, one Rust
  library per program) is the only written plan for that next step and moves into
  PROJECT-MAP-DESIGN §8 first; the four citations in the design repoint there.
- Every pointer to those three files is repointed (CLAUDE.md, NEXT-SESSION, the designs, README).

## What is updated

- **CLAUDE.md**: step 2 points at NEXT-SESSION's kickoff only; the effort switch points are named
  in one line here (medium by default; high for design judgement) instead of a pointer that loops.
- **AGENT-BRIEFING.md**: the pasted kickoff tail (from line 284) cut; its lasting notes become a
  short §18; the ledger is `facts.jsonl` and `plan.toml`; `harness-tui` joins the crate list; the
  milestone list gets a status (M0–M4 done; built since: the cockpit, the chat, harness-mcp,
  features, speed, the project map); §9's "no `migration/` means M0" rewritten; `PlannerStrategy`,
  `harness state gc`, storage profiles, budgets and `harness usage` marked "not built" where they
  appear; the documents list points at docs/README.md.
- **NEXT-SESSION.md**: cut to one kickoff (under 90 lines): where things stand, the person's rule,
  the work in order, the once-per-checkout note, the environment and the flakes; the three items
  that lived only in its trail go to DECISIONS first (the wording pass; the "about two-thirds of a
  beta" answer; the `harness answer` idea); open items rechecked against the code (two already
  done).
- **DECISIONS.md**: an index at the top (date → title → one line); a 2026-10-08 wording-pass entry;
  one sentence in the 10-15 entry saying the README no longer owns Quick start; the two old "READ
  THIS FIRST" headings prefixed "(superseded; see the index)".
- **SCHEMAS.md**: the harness-added flags' own refusal sentence (`-c`, `-o`, `-MD`, `-MF`); the
  section name the code's refusal cites made to exist ("harness.toml, file-list form"); the
  `facts.db` sentence replaced by one line saying it was never built and the query consumer reads
  the files; the four edits the survey lists.
- **README.md** (the reference half): "the command asks you to pick" → the command refuses and names
  the tools, the cockpit lists them; harness-mcp's install line points at the guide's install step;
  the five lines the survey lists.
- **TUTORIAL.md**: the three phrases.
- **PERF-DESIGN.md**: status "built 2026-10-03; governs with SCHEMAS"; one status note that the Linux
  launcher is not built and Linux is refused today, instead of rewriting 25 passages.
- **CHAT-PANE-DESIGN.md, COCKPIT-WRAPPER-DESIGN.md**: status lines "built 2026-09-28"; the "not built
  yet" and "next" sentences corrected.
- **MCP-DESIGN.md**: status line; `--tool` and the adoption refusal added where the design lists the
  server's flags and refusals.
- **Every other design**: a status line.
- **targets/tractor/README.md**: the once-per-checkout `--adopt` in its cold-checkout steps.
- **devtools/README.md**: `guide/gates.py`; the example path.
- **docs/README.md** (new, under 60 lines): one table, every document, its kind and one line, in
  four groups: people using RuHarness, developers, sessions, history.

## Execution

Two Opus passes by file ownership: (1) the session documents (CLAUDE.md, AGENT-BRIEFING,
NEXT-SESSION, DECISIONS, the two removals' pointer changes); (2) the designs, SCHEMAS, README's
reference half, TUTORIAL, the tractor and devtools READMEs, docs/README.md, the roadmap fold and
removal. Then the session checks: no reference anywhere (docs, code comments, help text) names a
removed file; the guide's gate passes; nothing that lived only in a removed file is gone.
