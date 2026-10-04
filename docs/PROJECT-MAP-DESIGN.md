# Real project layouts: the project map (design draft)

Status: **draft 0 — not reviewed, not to be built yet** (2026-10-02). Written after
docs/PROJECT-MAP-ROADMAP.md (the idea) and docs/PROJECT-MAP-INVESTIGATION.md (the experiment).
The next step is the full process the speed work had: a research spike on two or three real
downloads, then review rounds of this draft, then a closed revision, then a build in committed
steps. Part 2 of the roadmap note — the fuller C–Rust boundary — is **out of scope** here; this
design only gets a real project to the point where today's harness can take over.

## 1. What it is, in plain words

Today the harness migrates one folder of one program, and the person gathers that folder by
hand. The goal here is the whole project: **`harness map` looks at a whole C project — every
folder — and works out, without a model, which
files make up each program (**tool**) it builds, which files several tools share (a **library**),
which files are tests, examples or for another platform, and which files it could not make sense
of. It writes that as the **project map**. Where the facts leave a choice open — two files that
define the same function, a program that may be a test, a group of files that may be one library
or two — a model is asked, with the map as its only input, and its answer is checked by actually
compiling and linking each proposed tool. The person then accepts the bins they want to migrate,
and each accepted tool becomes a target exactly like today's `targets/zopfli`.

What does not change: the scanner, the plan, the judge, features, perf and the cockpit all keep
working on **one tool at a time**, as they do now. The map's job ends when it has produced, for
each tool, the description those already understand — a list of files and the flags to build
them — and has said plainly what it left out and why.

## 2. Why now, and what today's rules cannot do

Today a target is one folder (`[target] source_dir`): every top-level `.c` there is the one
program, built with one `cc` command and one set of flags (`include_dirs` must sit inside the
folder; libraries come from `[oracle] extra_link_args`). A real download fails that in four ways
at once — several `main()`s (tools, tests, examples), files for another platform, generated
headers (`config.h`), and C spread across folders — so the person gathers one tool's files by
hand (the testing guide does this for liblzg, dropping its two other tools). The investigation
showed the mechanical part of that gathering is cheap and exact on zopfli, and that the one kind
of question it cannot settle (a duplicate definition) is small and checkable.

## 3. The design

### 3.1 The walk (deterministic, no model)

Input: a project root. Output: facts, every one reproducible from the files alone.

1. **Files.** Walk the whole root with the scanner's confined walk (links refused as today,
   `.git` and `migration/` skipped, the existing file-count and depth limits). Record every `.c`
   and `.h`: path, size, blake3. Record non-C sources (`.cc`, `.cpp`, `.m`, `.go`, `.rs`) as
   **set aside by type**, with their count per folder, never their content.
2. **Per-file facts from the scanner.** Functions defined, calls made, includes — what
   `harness scan` records today, run over every `.c` found, not one folder. A file the parser
   cannot read is recorded as such and kept.
3. **Include folders.** Every folder that holds a `.h` is a candidate `-I`. For each `.c`, the
   folders needed are found from its includes (the scanner's resolution, extended to search
   every candidate folder when the including file's own folder does not have the header).
4. **Build evidence.** If `compile_commands.json` is present at the root or one level down,
   read it: for each file it names, take its exact `-I` and `-D` flags and its working folder.
   Files it names that the walk did not find, or the reverse, are recorded. A `CMakeLists.txt`,
   `Makefile`, `configure`, `meson.build` is recorded as present (its name only); nothing is
   run.
5. **Symbols.** Compile each `.c` to an object (`cc -c -w` with the file's include folders and,
   when known, its `-D` flags) under the tool sandbox, and read with `nm` what it **defines**
   (external text, data and read-only symbols) and what it **needs** (undefined). A file that
   does not compile is recorded with the first line of the error (control characters removed,
   ≤ 160 bytes) and kept; the walk never stops on it.
6. **Entry points.** Every file that defines `main` is a candidate tool.
7. **Closures.** For each entry point, from `main`'s file: add every file that defines a symbol
   the set needs, until nothing new is added. Record the closure's files, the symbols still
   unresolved (**outside symbols** — libc, libm, an outside library) and, when a symbol is
   defined in more than one file, record the **duplicate** with all its definers and add all of
   them to the closure (the choice is not the walk's to make).
8. **Shared files.** A file in two or more closures is shared by those tools.
9. **Unreached files.** A `.c` in no closure is recorded as unreached (dead code, a library
   without a tool in this project, or a file whose entry point did not compile).

Everything in steps 5–9 the investigation did in 40 lines on zopfli (exact: 13 files, 29
outside symbols, no duplicates) and on a made-up project with three `main()`s and one duplicate
(separated correctly; the duplicate surfaced).

### 3.2 The project map file: `migration/project-map.json` (`ruharness-project-map` v1)

Written at the project root (it is not a target yet). Facts and summaries only — **no source
text**, so it fits a model's context on a large project; every string that comes from the project
(a path, a symbol name, a compile error's first line) is display-filtered and, when handed to a
model or harness-mcp, fenced as untrusted exactly as the MCP reads fence ledger text.

```
{ schema, schema_version, root_hash, taken_at_commit?,
  files:    [{path, kind: c|h|other, bytes, blake3, lang?, parsed: bool,
              compiled: ok | {error_head}, functions: n, includes: [path], include_dirs: [path],
              defines: [sym], needs: [sym], flags_from: walk | compile_commands}],
  entry_points: [path],
  closures: [{entry: path, files: [path], outside: [sym], duplicates: [{sym, definers: [path]}]}],
  shared:   [{file: path, tools: [entry]}],
  unreached: [path],
  set_aside: [{folder, lang, count}],
  build_evidence: {compile_commands: present|absent, cmake|make|configure|meson: present|absent},
  limits_hit: [..] }
```

Size: paths and symbol names dominate. A cap of 20 000 files and 200 000 symbols; past it, the
map says so and the model step is not offered. A file list per closure is kept as indexes into
`files` to halve the size (an implementation detail for the build).

### 3.3 The question to the model (Tier 1, optional, the `external` hand-off as everywhere)

Asked only when the facts leave something open; with no duplicates, one entry point and no
unreached files, the map is final without a model (zopfli). The request carries the map and asks
for exactly these decisions, in a strict reply format:

- for each entry point: a **name** (lowercase id), a one-line **purpose**, and a **kind** from
  `tool | test | example | benchmark | other`;
- for each duplicate: which definer to **keep**, and why in one line (`platform: unix`,
  `alternative implementation`, `cannot tell`);
- for shared files: **groups** named as libraries (an id and a file list);
- nothing else: it may not add files, drop files, or name a file not in the map.

The reply is read strictly (closed kinds, ids by the plan's id rule, every path one of the map's).
Everything the model writes is untrusted text until the check below passes; its names and purposes
are display-only (never in a check name, an event or a prompt, like features' names).

### 3.4 The check (deterministic)

For each entry point the reply calls a `tool`: compile its closure's files (with the duplicates
resolved as the reply says, the others dropped) and **link** them into one program with the
closure's outside libraries guessed from the outside symbols (`-lm` for `log`, `sin`…; `-lz`
when `deflate`/`inflate` are needed and zlib is on the system; otherwise the link fails and
says which symbols). A tool that does not link is **refused with the linker's missing symbols**;
the reply as a whole is not. Every `.c` in the map must end in exactly one of: a tool's closure, a
shared library, `test`/`example`/`benchmark`, unreached, set aside, could-not-compile. Nothing
is silently dropped. A reply that names a file outside the map, or leaves one unaccounted for,
is refused in full.

### 3.5 Accepting bins into targets

The cockpit (and `harness map accept <tool>`) turns an accepted tool into a target folder:
`targets/<project>/<tool>/harness.toml` with a **new `[target] files` list** (paths relative to
the project root, with each file's include folders and `-D` flags) replacing `source_dir`, plus
the guessed `extra_link_args`. `source_dir` stays valid for existing targets: a `harness.toml`
has one or the other. A shared library accepted with two tools becomes units in **each** tool's
target; §6 says why that is not yet the end state.

### 3.6 What the rest of the harness must learn

- **Scanner:** read a file list, not a folder; resolve includes through per-file include folders.
- **Plan:** no change in rule (leaf units as today), but a leaf is judged against the tool's
  closure, so the same file is a leaf in one tool and not in another.
- **Judge's build (`whole_cc_into`):** compile per file with that file's flags, then link once —
  today it is one command. Features and perf build through it, so they follow.
- **Features' program digest, perf's program digest:** over the file list, not the folder.
- **The cockpit's tree:** today it is the target's folder; with a file list it shows the tool's
  files under their real folders, with the rest of the project greyed as "not part of this tool".
- **SCHEMAS:** `project-map.json`, the `files` form of `harness.toml`, the writer rows, the fence
  on the model's reply.

### 3.7 The CLI

- `harness map [--root DIR] [--json]` — writes `migration/project-map.json`; prints the tools
  found, shared files, duplicates, unreached and set-aside counts, could-not-compile files with
  their first error line. Exit 0 when a map was written (even a partial one), 1 refused (no C
  found, limits), 2 usage.
- `harness map ask [--root DIR]` — the model step through the `external` hand-off; writes the
  checked reply beside the map as `project-map.bins.json`. Refused without open questions.
- `harness map accept <tool> [--into DIR]` — writes the tool's target.
- Events: `map-file`, `map-tool {entry, files, outside, duplicates}`, `map-bin {tool, kind, ok |
  missing: [sym]}`.

### 3.8 Security

The per-file compiles and the check links run under the existing **tool sandbox** (the compiler
is allowlisted already); nothing of the project is ever executed. The map holds paths and symbol
names from an untrusted download: display-filtered everywhere, fenced to the model and to
harness-mcp. `compile_commands.json` is read, never executed; a command in it that is not a C
compile is ignored and counted. The model's reply changes nothing until the deterministic check
passes, and even then only writes a `harness.toml` the person accepted.

### 3.9 An interactive picture of the map (the person's wish, 2026-10-02; to investigate)

The person wants to **see** the map, not read it: an interactive architecture diagram of the
project — files as nodes, calls and includes as paths, clustered around the tools the closures
found, with shared libraries sitting between the tools they serve. The cockpit (a TUI) is the
right place for walking a migration step by step, but probably the wrong medium for a graph of
hundreds of files; this is a different kind of view and may be a **throwaway export** rather than
a cockpit screen.

Direction to investigate, not decided:

- **Source of truth: the map file.** The picture is rendered *from* `project-map.json` (and,
  once a tool is a target, from the facts and plan), never from a separate analysis — so it shows
  exactly what the harness believes, and a stale picture is visibly stale (the map's `root_hash`).
- **Form:** most likely `harness map export --html` writing a single self-contained HTML file
  (one page, no network, the graph data embedded) that opens in a browser: zoom, pan, click a
  node for its facts, collapse a tool into one node, colour by state (C / Rust in use / set aside
  / could not compile / duplicate). Candidates for the drawing: a force-directed or clustered
  layout; the libraries used must pass the dependency due-diligence rule (vendored or embedded,
  pinned, no downloads at run time). A plain-text fallback (Graphviz `.dot`) is cheap and worth
  having regardless.
- **What it shows, in layers:** (1) the tools and what they share — the "which programs are in
  here?" picture; (2) inside a tool, the units of the plan and their order (leaf first) — the
  migration picture, where each unit's state is today's cockpit state (planned, tried,
  migrated, failing); (3) inside a unit, the functions and their calls. Features' map (which
  functions each feature runs) and perf's rows (which units got slower) are natural overlays
  later.
- **Interaction limits:** read-only. The picture never changes a bin, a target or the plan; an
  action chosen in it (accept this tool, migrate this unit) would hand off to the cockpit or the
  CLI, if ever. Untrusted names (paths, symbols, the model's tool names) are escaped for HTML as
  the cockpit display-filters them for the terminal.
- **Size:** a project of thousands of files needs collapsing by folder and by tool at first
  view, with expansion on click; the map's caps bound it.

**Decided direction (2026-10-03, with the person):** the picture is **read-only, and the data
flows one way** — the scan and the map feed the picture; the picture never writes a bin, a plan,
a target or a test. Two places that can change state would have to agree and would each need
the cockpit's confirm-and-show-the-command safety; not worth it before anyone has used the
picture. The first and only interaction beyond looking is **handing off**: click a tool or a unit
and get the exact `harness` command (or the cockpit opened on it); the write still happens in
the cockpit or the CLI. Reference for the *concept*, not the implementation:
[emerge](https://github.com/glato/emerge) — scan a codebase, build a dependency graph, render
an interactive HTML page with force-directed layout, clustering and metrics. Not adopted: it is
Python with its own parsers, which would be a second opinion on the code beside the harness's
scanner; the rule here is one source of truth (the map). Order unchanged: the map first (its
link closures are what make clusters mean anything — on a raw download every file is one blob),
then a throwaway HTML export of the map on zopfli (about a day) to see whether the picture earns
its place, then a design of its own if it does. A picture is also the quickest way to *check* a
map: a wrong closure is obvious drawn and invisible in JSON.

Open: whether this is a `harness map export` (throwaway file), a cockpit act that opens the
browser on that file, a page in the cockpit's own MCP-served help, or all three.

### 3.10 What the map makes possible: where a model earns its place (direction, 2026-10-03)

The founding rule (briefing §1): models for judgment, deterministic tooling for measurement. The
map produces facts no model should guess — the call graph across folders, the closures, shared
code, hazards (the detectors), feature coverage (the features map). Two judgments sit naturally
on top of those facts, and both are **advice**, offered the same way as the map's bins: the
harness gives the facts, the model proposes with reasons, the harness checks what it can, the
person accepts.

1. **What to migrate first.** Today the plan's order is structural only: leaf units in
   dependency order. With the map's facts a model can weigh value as well — "start with the
   checksum: small, no hazards, every feature runs it; leave the signal handler for last." The
   harness checks the proposed order still respects dependencies; the person sees it, with the
   reasons, before the plan changes.
2. **Where to put the C–Rust seam.** Today the boundary is fixed — one C file is one unit, only
   leaf units cross. Reading the call graph a model can propose better cuts: "these three files
   are one subsystem; migrate them together so the seam has four calls instead of forty", or
   "this struct crosses the seam; move the seam one level out." The harness checks a proposed
   unit still links as one; the person accepts. This is Part 2 of the roadmap note made
   concrete, and it is where migration pain concentrates.

Where a model stays out: deciding what is *correct* (the oracle, the link check, the differential
runs — deterministic, always), and anything that writes state unchecked.

**The principle under all of it (the person's, 2026-10-03): know what right looks like before
anything changes.** The harness already records expected behaviour at two levels — the driver
(every function's outputs on fixed inputs) and the features (what a person-visible feature
materialises as: exit status, stdout, stderr on fixed runs). Make that the organising rule as
the model's role grows: every proposal — a bin, an order, a seam, a translation — is judged
against behaviour recorded *before* the change, so a wrong change is known wrong with high
probability, and a right one known right, rather than trusted. The picture (§3.9) shows the
proposal and its reasons; the recorded behaviour judges the result; **a subject-matter expert
signs off**, and nothing is accepted without that. The gap to close for the map: once bins
exist, record each tool's expected behaviour (features for a tool, not only for one program)
*before* its first unit moves, so the whole project has a baseline to iterate against.

## 4. Tests and checks (to be completed in review)

- The walk on the 100 benchmark cases: each gives exactly one library bin, zero tools (99) or one
  (1), and the same `include_dirs` `bench init` writes today.
- zopfli: one tool, 13 files, `-lm`, zopflipng set aside as C++ — the map reproduces the committed
  `harness.toml`.
- A made-up project with three `main()`s, a duplicate and a test (the investigation's): the
  closures, the duplicate surfaced, the shared file named; a forged reply that drops a file, adds
  a file, or picks a definer not in the map is refused; the tool whose duplicate was resolved
  links, the other does not and says the missing symbol.
- A file that does not compile: recorded, the walk goes on, its closure says it is partial.
- `compile_commands.json` with a `-DHAVE_CONFIG_H` flag changes a file's defines; without it the
  file does not compile and says so.
- Limits: a project over the file cap is refused by name; a hostile path or symbol name with
  control characters never reaches the terminal unfiltered.
- The model step with no open questions is refused; with one it is offered.

## 5. Order of work (proposal)

(a) the walk and symbols over one folder, replacing nothing (a `map` that reproduces today's
`harness.toml` on zopfli and the benchmark); (b) the whole-root walk, closures, the map file;
(c) `[target] files` in `harness.toml` and the per-file build in scanner and judge, with
`source_dir` kept; (d) the model step and its check; (e) `map accept` and the cockpit; (f)
SCHEMAS, tutorial, testing guide. Each committed green; then review, fix passes, mutation checks,
DECISIONS.

## 6. Residuals and what this does not do

- **Projects that need `./configure` or CMake to generate headers** are mapped as far as they
  compile; the map says which files failed on a missing header and names it. Running the
  project's own build is out of scope (it executes untrusted code).
- **`#ifdef` whose meaning depends on flags** the walk did not know: the symbol facts are those of
  the flags used; a file built twice with different flags appears once. A later revision may
  take several configurations from `compile_commands.json`.
- **A shared library is migrated once per tool** that uses it: two targets, two copies of the
  unit's Rust and two verdicts. One Rust library per program, and non-leaf units, are Part 2 of
  the roadmap note.
- **Outside libraries** are guessed from symbols; a wrong guess fails the link and is said.
- **Static functions** are invisible to the closure (they never cross files); the scanner's
  per-file list still records them.
- **Linux**: the walk and the check run wherever `cc` and `nm` exist; nothing here is macOS-only.

## 7. Open questions for the person

1. Should `harness map` run the per-file compiles by default (slower, exact) or symbols-off by
   default (fast, parser facts only) with `--symbols` to add them?
2. When a project has one tool and no open questions, should `map` write the target itself, or
   always stop and show the map first?
3. Where do mapped targets live: `targets/<project>/<tool>/` inside RuHarness (as zopfli does) or
   a `migration/` folder inside the downloaded project?
4. Is a library with no tool in the project (the benchmark's shape) a target by itself — as today
   — or does the map need a `main()` to call something a tool?
5. The picture (§3.9): a throwaway HTML export first, or straight to something the cockpit can
   open? And which of the three layers (tools / units / functions) matters most to see first?
6. §3.10: should the first model-backed advice be the migration order (cheaper, builds on the
   plan) or the seam (more valuable, needs Part 2 of the roadmap note)?
