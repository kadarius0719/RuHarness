# Real project layouts: the project map (design, revision 1)

Status: **revision 1 — 2026-10-08, after the four-lens review; not yet checked, not to be built.**
Draft 0 (2026-10-02) was reviewed from four lenses (the facts, security, integration, the model step)
with every finding verified against the code and the spike (docs/reviews/2026-10-07-project-map-
design-review.md, with the triage at its end). This revision takes in the spike
(docs/PROJECT-MAP-INVESTIGATION.md "The spike on real downloads"), the person's answers of
2026-10-07 (§7) and the review. §8 lists the decisions this revision proposes for the person; §9 maps
every change back to the review. Next: CHECK THE REVISION (Opus checkers), the person's answers to §8,
then the build in §5's order. Part 2 of the roadmap note (a fuller C–Rust boundary) stays out of
scope: this design gets a real project to the point where today's harness can take over, one tool at
a time.

## 1. What it is, in plain words

Today the harness migrates one folder of one program, and the person gathers that folder by hand.
The goal here is the whole project: `harness project map` looks at a whole C project — every folder
— and works out, without a model, which files make up each program (**tool**) it builds, which files
several tools share, which files are tests, examples, fuzzers or for another platform, and which
files it could not make sense of. It writes that as the **project map**. The project's **build
configuration** — which build counts and the flags it compiles with — is the person's to state (a
model may propose it from the build files; the person confirms), because the map cannot tell two
configurations apart. Where the facts still leave a choice open — two files that define the same
function inside one program, a program that may be a test — a model is asked, with only the open
questions as its input, and its answer is checked by compiling and linking each proposed tool. The
person then accepts the tools they want to migrate, and each accepted tool becomes a target that the
scanner, the plan, the judge, features, perf and the cockpit handle as they handle `targets/zopfli`
today.

**What the link check proves, and what it does not.** Linking proves that the chosen files, with
the stated configuration and the guessed outside libraries, define every symbol the program needs
exactly once and produce a program. It does not prove that the entry is a tool rather than a test,
that the right definer was chosen when more than one links (liblzg's `decode.c` and `lzgmini.c`
both link), that the configuration is the one the project's own build uses (lz4's tool links with
and without threads, and on macOS with no link flag at all), that the file list matches the
project's build, or that the program runs. The map says this where it shows a tool; the person's
acceptance, not the link, settles those questions.

What does not change: the scanner, the plan, the judge, features, perf and the cockpit keep working
on **one tool at a time**. The map's job ends when it has produced, for each tool, the description
those already understand — a list of files, each with its include folders, and the configuration to
build them — and has said plainly what it left out and why.

## 2. Why now, and what today's rules cannot do

Today a target is one folder (`[target] source_dir`): every top-level `.c` there is the one program,
built with one `cc` command and one set of flags (`include_dirs` must sit inside the folder;
libraries come from `[oracle] extra_link_args`). A real download fails that in four ways at once —
several `main()`s (tools, tests, examples, fuzzers), files for another platform, generated headers
(`config.h`), and C spread across folders — so the person gathers one tool's files by hand (the
testing guide does this for liblzg, dropping its two other tools). The investigation and the spike
showed the mechanical part of that gathering is cheap and exact on zopfli, lz4 (48 files, 33 entry
points, 2.2 s) and liblzg, and that the questions it cannot settle are few and well shaped: the
build's flags (the person's), an in-program duplicate (a model proposes, the person decides), and
what kind of program an entry is.

## 3. The design

### 3.1 The walk (deterministic, no model)

Input: a project root. Output: facts, every one reproducible from the files, the configuration (§3.2)
and the recorded toolchain (§3.3).

1. **Files.** Walk the whole root with harness-core's confined walk, pruning `.git` and every
   dot-folder (listed as skipped, with counts) and the harness's own ledger folder (§3.7). Links that
   leave the root or point nowhere are left out; a link to a file inside the root is followed, and a
   file reached under two paths is recorded **once** with its other paths as aliases (a symlinked
   `.c` must not become a duplicate definer). Record every `.c` and `.h`: path, size, blake3. Record
   non-C sources (`.cc`, `.cpp`, `.cxx`, `.m`, `.go`, `.rs`, `.py`, `.js`, `.lua`, `.pas`), assembly
   (`.s`, `.S`, `.asm`) and prebuilt files (`.a`, `.o`, `.so`, `.dylib`) as **set aside by type**,
   with their count per folder, never their content. Caps (§3.10): 20 000 walked files, depth 32, a
   file over 8 MiB is recorded as too large and not read. A walk error or a file that is not UTF-8 is
   a fact of that file, never a stop.
2. **Per-file facts from the scanner.** Functions defined, calls made, includes — what `harness scan`
   records today — run over every `.c` found. Include facts come from every `#if` branch (the parser
   does not evaluate them); symbol facts (step 5) come from the one configuration. A file the parser
   cannot read is recorded as such and kept.
3. **Include folders, per file.** A file's include folders are found from its includes, both forms:
   a quoted include is tried in the including file's folder first, then among the **candidate
   folders** (every folder that holds a `.h`); an angle-bracket include is tried among the candidate
   folders only, never the including file's folder, and left to the system when none holds it
   (liblzg's tools include `<lzg.h>` from `src/include/`). Headers' own includes are followed the same
   way to closure. A file gets only the folders its includes need. When more than one candidate
   folder holds a header's name (two `config.h`; a project `util.h` beside the system's), the walk
   records an **ambiguous include** fact listing every candidate and picks none: the configuration
   (`compile_commands.json` or the person's flags) decides, and until it does the file's compile in
   step 5 may fail or use the wrong header — the fact says so.
4. **Build evidence.** `compile_commands.json` at the root or one level down is read, never
   executed: for each file it names, the flags that pass the grammar of §3.2 are taken (the rest
   counted as ignored), an entry whose `directory` resolves outside the root is ignored and counted,
   a file it names that the walk did not find is recorded and never compiled. A `CMakeLists.txt`,
   `Makefile`, `configure`, `meson.build` is recorded as present, by name; nothing is run.
5. **Symbols.** Compile each `.c` to an object (`cc -c -w`, the judge's own base flags — today
   `-O2 -ffp-contract=off` — plus the configuration's flags and the file's include folders) under
   the map's sandbox profile (§3.9), into a harness-owned temporary folder, each object named by the
   file's index; read the object with harness-oracle's `objsyms` (safe Rust, no `nm`) for what it
   **defines** (text, data, read-only, bss, common and weak symbols, with each symbol's kind) and
   what it **needs** (undefined; a weak undefined symbol is a need on macOS, where the linker still
   wants a definer, and optional on ELF, where it resolves to null — recorded with its kind), and
   delete the object. One leading `_` is stripped on Mach-O so names equal the scanner's. A symbol
   whose name is not shaped like a C identifier (an `asm` label can hold any text) is counted as an
   odd name and never stored as text. A file that does not compile is recorded with a **closed
   reason** (`missing-header <name>` with the header's relative name, `syntax`, `other`) and the
   place in the project; the raw first line is shown on the terminal once, scrubbed of machine paths
   and filtered, and never stored. The walk never stops on it.
6. **Entry points.** A file that defines a *function* `main` is an entry of kind `main` (a data
   symbol named `main` links but is not an entry); a file that defines `LLVMFuzzerTestOneInput` and
   no `main` is kind `fuzz`; a `main` whose need is met by two or more entry files is kind `driver`
   (lz4's `ossfuzz/standaloneengine.c`, a driver linked once with each fuzzer). A `driver` is recorded
   with the entries it serves, is never a tool, and is never asked "which one".
7. **Closures.** For each `main` and `fuzz` entry, from its file: add every file that defines a symbol
   the set needs, until nothing new is added. A file that defines `main` is never pulled into
   another entry's closure: a need met only by another program's `main` file is recorded as **"needs
   a symbol from program X"**, not as an outside symbol. A symbol defined in more than one file is a
   **duplicate**: its definers are recorded as *alternatives, pending*, none of them added, and the
   closure is marked partial until a choice is made (§3.4–§3.5); after a choice the closure is
   recomputed from scratch with the chosen file (a file the unchosen alternative had pulled in must
   not stay). Over the finished closure every symbol defined by two or more of its files is a
   collision, reported even when nothing needed it (the link would fail). What stays unresolved is
   the closure's **outside symbols** (libc, libm, an outside library — or, as §6 says, set-aside
   assembly).
8. **Shared files, after the choices.** A file in two or more closures is shared by those tools.
   Duplicates between programs that never meet in one closure (lz4's 14 helper names across
   `examples/` and `tests/`) are listed, not questions.
9. **Unreached files.** A `.c` in no closure. Those that need each other form a **library bin**
   (the connected groups of the need graph among unreached files): the benchmark's shape — all 100
   cases have zero entry points and one library — and a library a project ships without a tool. A
   library bin can be accepted as a target by itself (§7, answer 4).
10. **Textual dependencies.** A `.c` that another file includes (`#include "lz4.c"`) is a fact shown
    beside the closure; such a file is not offered as a unit on its own without a warning, because
    moving it to Rust leaves the including file still compiling its C text.
11. **What the project's build links beyond the closure** is not known without the build system; the
    map says, where it shows a tool, that the closure is what the program needs, not what the build
    links, and that on these downloads the extra files (lz4's `lib/lz4file.c`) are unreached; a file
    the build links only for a constructor or a linker section is the exception the map cannot see.

Everything in steps 5–9 the spike did in 90 lines on zopfli, lz4 and liblzg; the closures were
identical at `-O0` and `-O2` and only the outside names moved, which is why step 5 fixes the flags.

### 3.2 The configuration: which build counts, and its flags

**The person's decision (2026-10-07):** without a `compile_commands.json`, the person states the
build and its flags; a model may read the build files and propose them; the person confirms. The
spike showed why: lz4's tool gains a file and pthreads with `-DLZ4IO_MULTITHREAD`, its Makefile and
Meson build it so and CMake does not, and every configuration links. So the baseline is **one named
configuration**, stated per accepted tool, and a map's closures carry the configuration they were
made with.

- **Order of trust.** `compile_commands.json` (source `compile_commands`); else the person's stated
  configuration (source `stated`); else the walk's guess — no flags, every candidate folder a
  possibility — marked `guessed` on the map and on every closure it shaped.
- **The flow.** The first `harness project map` runs with no configuration (2.2 s on lz4) and shows
  the build files it found and the closures, marked guessed. The person writes
  `migration/map/config.toml` — a name (`make`, `meson`, `cmake`, or their own), which build it
  stands for, the flags, and the program's **run name** (`[target] name`: what features run the
  program as; lz4's tool is `lz4`, not `lz4cli`) — or asks the model to propose one (§3.4), and runs
  `map` again. `project ask` is refused while the configuration is a guess unless `--allow-guessed`
  is given, because a wrong flag changes the closures and the questions.
- **The flag grammar, one for every source** (`compile_commands.json`, the model's proposal, the
  person's file, and `harness.toml` when the target is loaded — `harness.toml` is untrusted input):
  joined `-D<name>[=<value>]` with the name a C identifier, `-U<name>`, `-I<dir>`, `-iquote<dir>`,
  `-isystem<dir>` and `-include<file>` with the path inside the root after resolving, `-std=` from a
  fixed set, `-O0`–`-O3`, `-pthread`, and `-f…` from a fixed list; no value may start with `@` or `-`
  (clang reads an `@file` argument as a file of options, joined or not); every path is absolute when
  passed and `--` separates the options from the source; never `-B`, `-fplugin`, `-Xclang`, `-load`,
  `-o`, the `-M` family, `-Wl`, `-Wa`, `-wrapper`, `-x`. Link arguments keep today's `-l<name>` rule.
  A flag outside the grammar is refused by name, whoever proposed it.
- **Recorded** as `configuration {name, source, flags, run_name, digest}` on the map, in the checked
  reply, and in the accepted target's `harness.toml`; a per-folder or per-file configuration is not
  expressible. **Revisit when:** a project's flags differ per file in a way one stated configuration
  cannot express — lz4's tests build the shared `lib/lz4.c` with `-DLZ4_DEBUG=1` where the tool does
  not, which a configuration per accepted tool absorbs; a tool whose own files need different flags
  would not be.

### 3.3 The project map file: `migration/map/project-map.json` (`ruharness-project-map` v1)

Written in the project's ledger folder (§3.7). Facts and summaries only — **no source text**. Every
string that comes from the project (a path, a symbol name) is stored raw, so it can be matched to
files exactly, and display-filtered only when shown; it is fenced as untrusted when handed to a
model or to harness-mcp, with the fence the cockpit's chat and harness-mcp use.

```
{ schema, schema_version, root_hash, inputs_hash, taken_at_commit?,
  toolchain: {cc, target},
  configuration: {name, source: compile_commands | stated | guessed, flags, run_name, digest},
  files:    [{path, aliases: [path], kind: c | h | other | asm | prebuilt, bytes, blake3, lang?,
              parsed: bool, too_large?: bool, not_utf8?: bool,
              compiled: ok | {reason: missing-header | syntax | other, header?, at?},
              functions: n, includes: [path], include_dirs: [path], ambiguous_includes: [{header,
              candidates: [path]}], outside_includes: bool, included_by: [path],
              defined_symbols: [{name, kind}], needed_symbols: [{name, weak?}], odd_names: n,
              flags_from: compile_commands | stated | guessed}],
  entry_points: [{path, kind: main | fuzz | driver, serves?: [path]}],
  closures: [{entry, kind, files: [index], partial: bool, outside: [sym], needs_from: [{sym, program}],
              duplicates: [{symbols: [sym], definers: [path], choice?: {keep, by: links | model |
              person, reason}}], collisions: [{sym, definers}], guessed_flags: bool}],
  shared:   [{file, tools: [entry]}],
  unreached: [path], libraries: [{id, files: [path]}],
  set_aside: [{folder, lang, count}], skipped_folders: [{path, count}],
  build_evidence: {compile_commands: present | absent, ignored_entries: n, cmake|make|configure|meson:
                   present | absent},
  limits_hit: [..] }
```

- `root_hash` is SCHEMAS' file-set hash over every walked `.c`/`.h` plus `compile_commands.json`
  when present; `inputs_hash` covers the configuration and the toolchain (a stale picture is
  visibly stale after a compiler change). `taken_at_commit` is information only; nothing keys on it.
- Every list is sorted (files by path bytes; `definers`, `outside`, `shared.tools` by name) so two
  runs give byte-identical files (the golden test, SCHEMAS' canonical-serialization rule).
- The ids: a tool's id is derived from its entry file as the planner derives unit ids (the file's
  stem, lowercase, `t-<stem>`, with the full-path form when two collide; unique ignoring case; never a
  reserved name). The model's name and purpose (§3.4) are display-only and never ids.
- Size: paths and names dominate; the caps of §3.10 bound it. Past a cap the map holds the file facts
  only, no closures, and the command exits 1 naming the limit (one behaviour, §3.8).

### 3.4 The question to the model (Tier 2, optional, the `external` hand-off as everywhere)

Asked only when the map holds **open questions**; with none (zopfli), the map is final without a
model and the person still accepts (answer 2). What is open, and the order it is settled in:

1. **The configuration**, when it is a guess (§3.2): `harness project ask --build` sends the build
   files (capped, each fenced with the deterministic-nonce fence triage uses) and asks for a proposed
   configuration in the grammar of §3.2, per scope, each flag citing `file:line` and each assumption
   stated (a `$(shell …)` probe can only be guessed). The reply is advice shown to the person, never
   applied until confirmed.
2. **Entry kinds** for `main` entries the folder rules leave open. The walk first guesses from the
   folder names (`tests/`, `test/`, `examples/`, `bench*/`, `fuzz*/`, `ossfuzz/` → test, example,
   benchmark, fuzz) and labels the guess; `fuzz` and `driver` need no model. Only an entry outside
   those folders, or one the person disputes, is a question.
3. **In-closure duplicates** (never between-program ones), one question per set of definers that
   recur together (liblzg's `LZG_Decode` and `LZG_DecodedSize` share one pair), with the scope: this
   tool, or every closure that holds the pair.

The request carries **only the open questions**, each by a harness-made index (`e3`, `d1`) with
that item's facts (path, folder, closure size, outside-symbol count, the definers' folders) inside
one fenced block, in batches of at most 10, under the request-size cap; the trusted part holds
only harness-written text. The strict reply names indexes only: for an entry, a **name** (display
only), a one-line **purpose** (display only), a **kind** from `tool | test | example | benchmark |
other`; for a duplicate, `keep: <definer index>` or `undecided`, with a reason from a closed list
(`platform`, `alternative-implementation`, `cannot-tell`); nothing else. A reply that names an index
not asked, skips one, or adds anything is refused in full, as triage's "exactly the requested ids"
rule does. Everything the model writes is untrusted text until the check passes. The provider and
model come from `--provider`/`--model` (as `migrate` and `gen-driver` take them), Tier 2 by default
(classifying programs and choosing definers is judgment, not summarization); escalation is the
person's re-ask at a higher tier, because a wrong answer that links produces no failure to escalate
on. Traces live under `migration/map/traces/`; record and replay work as today's hand-off (the
`TraceAdapter`, `checked_complete`), with one new module like triage's (prompt assembly and a strict
validator). The reply is **bound to the map's `root_hash` and the configuration's digest** it
answered; a reply for another map is not read.

### 3.5 The check (deterministic)

1. **Every `main`-kind entry is link-checked**, whatever kind the model gave it: compile its closure
   with the duplicates resolved as chosen and link it into one program with the outside libraries
   guessed from the outside symbols (`-lm` for `log`, `sin`…; `-lz` when `deflate`/`inflate` are
   needed and zlib is on the system; `-lpthread` for `pthread_*` where needed). A program that does
   not link is **refused with the linker's missing or duplicate symbols**; the reply as a whole is
   not. `fuzz` entries are linked only with the project's driver when there is one; `driver` entries
   are never linked alone. Nothing built is ever run.
2. **A duplicate is checked by linking every choice** when the set is small (at most 4 definers, at
   most 16 combinations per tool) — it costs seconds. When exactly one choice links, that choice is
   recorded `by: links` and the model's pick is only confirmed or contradicted. When more than one
   links, the check cannot decide: the tool is **held** — "the model picked X; linking cannot tell X
   from Y" — and the person chooses (§3.6). `undecided` always goes to the person.
3. **Every file's status is computed by the harness** from the walk and the choices, never taken
   from the reply: entry, in a closure, alternative not kept, shared, library, unreached, set aside,
   could not compile, too large. Nothing is silently dropped; the reply cannot change a status
   except through its choices.
4. **What the check cannot prove** is printed with the map (§1): the kind, the configuration, the
   file list against the project's build, that the program runs. The reply's kinds and names are
   shown labelled as the model's.

### 3.6 The person's gate, and accepting a tool

`harness project map` and `project ask` **always stop and show** (answer 2); nothing becomes a target
until `harness project accept <tool-id>`. The screen shows, per tool: the closure's files by folder;
the outside symbols and the guessed libraries; the configuration's name, flags and source; each
duplicate's choice and who made it (`links`, `model`, `person`); the model's kind and purpose,
labelled; what the link proved and did not (once per screen); whether the closure is partial, and
what the project's build may link beyond it. The person resolves a held duplicate here
(`accept --keep d1=<definer>`), and may dispute a kind.

`accept` refuses while any in-closure duplicate is unresolved (the planner keeps the last definer
silently otherwise), while the configuration is a guess (unless `--allow-guessed`), and when the map
or the configuration changed since the reply (their digests); it **re-runs the link check** before
writing anything, never trusting the stored reply. It then writes the tool's target (§3.7): a
`harness.toml` in the file-list form — the files, each with its include folders, the configuration,
the guessed `extra_link_args`, the run name, the map's `root_hash` and the choices — so a later `map`
can say what changed: closure changed, configuration changed, new entry points, an accepted tool
that no longer links. The acceptance is the written `harness.toml`, reviewed with `git diff` as the
plan is; no separate record. A shared file accepted with two tools becomes units in each tool's
target (two ledgers, two Rust copies; §6). A library bin is accepted the same way, with no entry.

### 3.7 Where a mapped target lives, and what the rest of the harness must learn

**The layout (proposed, §8).** The person decided (answer 3) that a mapped tool's target and ledger
live inside the project. Today the folder that holds `harness.toml` does five jobs at once: it holds
the config, it is the parent of the ledger, the base of every path in the facts and the plan, the
folder every compiler input must stay inside, and the sandbox's read root. For several tools in one
project the first and second jobs must split from the other three:

- The **project root** stays the target root for containment, the working folder, the sandbox's
  read root and the base of every path (`--target <project>` as today).
- The **ledger folder** is `<project>/migration/` for a project's first tool (zopfli's layout today,
  nothing moves) and `<project>/migration/tools/<id>/` for each further tool, chosen with
  `--tool <id>`; `harness.toml` sits in the ledger folder of a file-list target. `TargetContext`
  gains the ledger folder beside `root`; the `Ledger` API takes it; the plan's `driver` paths stay
  root-relative. `migration/map/` (the map, the configuration, the traces) is project-level.
- `accept` writes a harness-owned `migration/.gitignore` (the scratch files: `build/`, `.lock`,
  `traces/`, `.promote-*`) into the person's repository.
- **A project that already ships a `migration/` folder** — its own database migrations, or a
  hostile download's ready-made ledger with green verdicts, a promoted crate with a build script, a
  `target/` folder or a `project-map.bins.json` — is refused by `project map` before the first map:
  it names the folder and what it holds, and goes on only when the person adopts it explicitly
  (`--adopt`), which is recorded in the ledger (not in state outside the project: an agent must be
  able to resume from the ledger alone on another computer). The walk prunes the ledger folder only
  once it is the harness's own.

**Per-file flags reach every compile.** A unit's C and the headers it shares with the driver are
compiled in about a dozen places (the driver-shape compile, the driver builds, the driver's
self-validation and mutants, the boundary check's wrapper, the features map's probed build, perf's
objects), every one through `Base::includes()` today, with no `-D`. Under one named configuration
the flags are target-wide: `Base` carries the configuration's flags and a per-file table of include
folders, and **every compile asks `Base` for a file's arguments** (the driver is compiled with the
flags and folders of the unit file it tests). The whole-program build adopts perf's compile-objects-
then-link path (`compile_objects`, `link_side`), which already exists and is proven equal to the one-
command build. The benchmark scorer builds suite cases and is untouched.

**The scanner** reads a file-list target as: the listed `.c` files plus every header reached through
each file's own include folders, both include forms, to closure, inside the project root and never
under the ledger folder (a `harness.toml` pointing into the ledger would put model-written files into
prompts and builds). Angle-bracket resolution is new to the scanner; for today's `source_dir`
targets (zopfli, the benchmark) no project header is included with angle brackets, so their facts
stay byte-identical (the test in §4). The scanner and harness-detect prune the ledger folder; detect's
own recursive walk (no containment, no cycle guard) is replaced by the confined walk.

**Confinement, restated for the file-list form:** nothing outside the listed files and their reached
headers reaches a prompt or a compile. Every reader of `source_dir` learns the rule: harness-scan
(the walk and `repo_relative`), harness-detect (its walk), harness-core (the planner's `source_hash`
over the include closure; the features digest and its staleness), harness-llm (`read_sources` for
`migrate` and `gen-driver`), harness-oracle (the boundary check's unit-header rule; the features
map's mirror, which copies the listed files and headers at their project-relative paths; `Base`),
harness-cli (`stale_fact_files`, `program_digest_now`), harness-tui (the tree, preflight, the read
model) and harness-mcp.

**The program digest and staleness.** The file-list form hashes a **v2 record** — the files, each
file's include folders, the configuration's name and flags, the link arguments, the run name — so
two configurations of lz4 differ; the `source_dir` form keeps its v1 record byte for byte (zopfli's
committed digest `d191be5c…` must not move). Facts are stale for a file-list target when a listed
file or a reached header changed or vanished, or when the project's `root_hash` changed (a new file
the closure would now need) — `state status` says "the project changed since this tool was accepted;
run harness project map".

**`harness.toml`, file-list form.** Read **version first** (today's loader parses the struct before
checking the version, so an older harness would say "missing field `source_dir`" instead of "schema
too new": the version read moves first), `schema_version = 2`; `[target] files = [{path, include_dirs,
flags_from}]`, `[target] configuration = {...}`, `name`; a file holding both `files` and `source_dir`
is refused by name. `nm` stays on the allowlist of every accepted target (verify's driver-shape check
runs it); the map itself needs only `cc`.

**The plan:** no change in rule. Units and dependencies come from the facts over the tool's file
list; a call to a project file outside the list reads as an outside call (the capability check
already reads unresolved names). A shared file is a unit in each tool that holds it (§6).

**The cockpit and harness-mcp:** for this design the command line is the only way in (`project
map|ask|accept`); the cockpit refuses a folder without `harness.toml` today and a project mode (a map
screen, a tree for a folder that is not yet a target) is a short design of its own, as is
harness-mcp's read of the map. Once a tool is accepted, both work on it as on any target; the
cockpit's tree shows the tool's files under their real folders.

### 3.8 The CLI

- `harness project map --target DIR [--json] [--adopt]` — writes `migration/map/project-map.json`;
  prints the configuration and its source, the tools found (entry, kind guess, files, outside
  symbols, partial), held duplicates, shared files, libraries, unreached, set-aside and skipped
  counts, could-not-compile files with their closed reason, ambiguous includes. Exit 0 when a map
  was written; 1 refused (no C found; a cap hit — the file facts are still written; a shipped
  `migration/`; no sandbox; a root that is or holds the home folder); 2 usage. It takes the ledger's
  writer lock. Output lines filter newlines and tabs in every project string (a file name can hold a
  newline; the review gate must not be forged); `--json` escapes every character `unsafe_to_show`
  names as `\uXXXX` (today's comment that serde does so is wrong; a small fix of its own).
- `harness project ask --target DIR [--build] [--provider P] [--model M] [--allow-guessed]` — the
  model step through the `external` hand-off; exits 1 with the `awaiting` event and the exact resume
  command, as `observe` does; writes the checked reply beside the map as `project-map.reply.json`,
  bound to the map's and the configuration's digests. Refused without open questions, and while the
  configuration is a guess unless allowed.
- `harness project accept <tool-id> --target DIR [--tool-dir] [--keep d1=<definer>]…` — writes the
  tool's target (§3.6, §3.7); exit 1 when refused.
- Events (`--json`): `project-file`, `project-tool {id, entry, kind, files, outside, duplicates,
  partial}`, `project-check {id, ok | missing: [sym] | held: [sym]}`; they carry paths and symbol
  names verbatim as the ledger's events do, never the model's names.
- The name: `harness project …`, not `harness map`, beside the existing `harness features map`.

### 3.9 Security

The threat model is the briefing's: the download is untrusted input, the model's reply is untrusted,
and now the ledger lives inside the download.

- **The map's compile and link run under a map sandbox profile**: the tool profile's rules (no
  network, writes only to the harness's temporary folder, `NO_STARTS_THROUGH_THE_SYSTEM`) with the
  read roots narrowed to the project root and the system — **not** the cargo and rustup homes, which
  `cc` does not need (a test proves `cc` still works). The flag grammar of §3.2 is what keeps the
  compiler from loading or running anything from the project (`@file`, `-B`, `-fplugin`,
  `-Xclang -load`, `-o`, `-MF`); it is checked at every source of flags and again when
  `harness.toml` is loaded. Where no sandbox exists (Linux today), `project map`, `ask` and `accept`
  refuse as every building command does unless `--allow-unsandboxed` is passed. A root that is the
  home folder, holds it, or holds the cargo or rustup homes is refused.
- **What a compile can read never reaches the map:** the error line is a closed reason (§3.1 step 5);
  every stored string from the project is a path or a symbol name, stored raw, display-filtered and
  scrubbed of machine paths when shown; `#embed` or an `#include` of a readable file can put its
  bytes into an object or an error line — the object is deleted unread by anyone but `objsyms`, the
  error line is shown once and not stored. The map stores no object bytes.
- **Symbols** are read by `objsyms`, not parsed from `nm`'s lines (an `asm` label can forge a line);
  only identifier-shaped names are stored; an entry point must be a function.
- **The ledger inside the download** (§3.7): a shipped `migration/` is refused before the first map;
  cargo and rustup run with their working folder in a harness-owned folder outside the project and
  `--manifest-path`, so a project's `.cargo/config.toml` (a `rustc-wrapper`, a `linker`) and
  `rust-toolchain.toml` (a `path` to the project's own `cargo` and `rustc`) are never read, with the
  harness's own toolchain pinned through `RUSTUP_TOOLCHAIN` (today it relies on its
  `rust-toolchain.toml` being found above the target); a promoted crate with a build script is
  refused (the harness never writes one); `accept` re-links and never trusts a stored reply; a
  stored reply or response file is bound to the map it answered.
- **Ids and names:** tool ids are derived, validated, unique ignoring case (§3.3); the model's names
  are display-only and fenced; paths in a reply are matched by exact membership in the map, never
  sanitized.
- **The review gate** cannot be forged by a file name: newlines and tabs are filtered in every
  printed project string, `--json` escapes everything unsafe.
- **Bounds** (§3.10): a size cap per file, a total time budget, a per-compile timeout, objects
  deleted as read, and the walk's caps; the compile's memory and disk are not limited by `setrlimit`
  (every crate forbids unsafe code; a limit would need the launcher pattern perf uses), said
  honestly.
- The project's own build is never run; nothing the map links is ever run.

### 3.10 Limits and bounds

20 000 walked `.c`/`.h` files (set-aside and skipped files are counted, not walked into), depth 32,
200 000 distinct symbol names, a file over 8 MiB not read, 120 s per compile and link, a total
budget of 30 minutes for `project map` (configurable in `harness.toml` once a target exists; a flag
before). Past any cap: the file facts are written, no closures are computed, exit 1 names the limit.
A cut-short walk must never produce closures, because definers in folders never reached would read
as outside symbols.

### 3.11 An interactive picture of the map (the person's wish, 2026-10-02; last, as agreed)

The person wants to **see** the map, not read it: an interactive architecture diagram of the project
— files as nodes, calls and includes as paths, clustered around the tools the closures found, with
shared libraries sitting between the tools they serve. The cockpit (a TUI) is the right place for
walking a migration step by step, but probably the wrong medium for a graph of hundreds of files;
this is a different kind of view and may be a **throwaway export** rather than a cockpit screen.

Direction to investigate, not decided:

- **Source of truth: the map file.** The picture is rendered *from* `project-map.json` (and, once a
  tool is a target, from the facts and plan), never from a separate analysis — so it shows exactly
  what the harness believes, and a stale picture is visibly stale (the map's `root_hash` and
  `inputs_hash`).
- **Form:** most likely `harness project export --html` writing a single self-contained HTML file
  (one page, no network, the graph data embedded) that opens in a browser: zoom, pan, click a node
  for its facts, collapse a tool into one node, colour by state (C / Rust in use / set aside / could
  not compile / duplicate). Candidates for the drawing: a force-directed or clustered layout; the
  libraries used must pass the dependency due-diligence rule (vendored or embedded, pinned, no
  downloads at run time). A plain-text fallback (Graphviz `.dot`) is cheap and worth having
  regardless.
- **Its own security rules, when it is designed:** the data is embedded as JSON with `<`, `>` and
  `&` written as `<`, `>`, `&` (HTML escaping does not apply inside a `<script>`
  block: a `</script>` inside a string would end it); names are inserted with `textContent`, never
  `innerHTML`; a Content-Security-Policy meta (`default-src 'none'`, script and style by hash) makes
  the browser enforce "no network"; the file is written with an exclusive create, outside the
  project or in the ledger folder, never where a shipped file could be opened in its place.
- **What it shows, in layers:** (1) the tools and what they share — the "which programs are in
  here?" picture; (2) inside a tool, the units of the plan and their order (leaf first) — the
  migration picture, where each unit's state is today's cockpit state (planned, tried, migrated,
  failing); (3) inside a unit, the functions and their calls. Features' map (which functions each
  feature runs) and perf's rows (which units got slower) are natural overlays later.
- **Interaction limits:** read-only. The picture never changes a bin, a target or the plan; an
  action chosen in it (accept this tool, migrate this unit) would hand off to the cockpit or the
  CLI, if ever.
- **Size:** a project of thousands of files needs collapsing by folder and by tool at first view,
  with expansion on click; the map's caps bound it.

**Decided direction (2026-10-03, with the person):** the picture is **read-only, and the data flows
one way** — the scan and the map feed the picture; the picture never writes a bin, a plan, a target
or a test. Two places that can change state would have to agree and would each need the cockpit's
confirm-and-show-the-command safety; not worth it before anyone has used the picture. The first and
only interaction beyond looking is **handing off**: click a tool or a unit and get the exact `harness`
command (or the cockpit opened on it); the write still happens in the cockpit or the CLI. Reference
for the *concept*, not the implementation: [emerge](https://github.com/glato/emerge) — scan a
codebase, build a dependency graph, render an interactive HTML page with force-directed layout,
clustering and metrics. Not adopted: it is Python with its own parsers, which would be a second
opinion on the code beside the harness's scanner; the rule here is one source of truth (the map).
Order unchanged: the map first (its link closures are what make clusters mean anything — on a raw
download every file is one blob), then a throwaway HTML export of the map on zopfli (about a day) to
see whether the picture earns its place, then a design of its own if it does. A picture is also the
quickest way to *check* a map: a wrong closure is obvious drawn and invisible in JSON.

Open: whether this is a `harness project export` (throwaway file), a cockpit act that opens the
browser on that file, a page in the cockpit's own MCP-served help, or all three.

### 3.12 What the map makes possible: where a model earns its place (direction, 2026-10-03)

The founding rule (briefing §1): models for judgment, deterministic tooling for measurement. The map
produces facts no model should guess — the call graph across folders, the closures, shared code,
hazards (the detectors), feature coverage (the features map). Two judgments sit naturally on top of
those facts, and both are **advice**, offered the same way as the map's questions: the harness gives
the facts, the model proposes with reasons, the harness checks what it can, the person accepts.

1. **What to migrate first.** Today the plan's order is structural only: leaf units in dependency
   order. With the map's facts a model can weigh value as well — "start with the checksum: small, no
   hazards, every feature runs it; leave the signal handler for last." The harness checks the
   proposed order still respects dependencies; the person sees it, with the reasons, before the plan
   changes.
2. **Where to put the C–Rust seam.** Today the boundary is fixed — one C file is one unit, only leaf
   units cross. Reading the call graph a model can propose better cuts: "these three files are one
   subsystem; migrate them together so the seam has four calls instead of forty", or "this struct
   crosses the seam; move the seam one level out." The harness checks a proposed unit still links as
   one; the person accepts. This is Part 2 of the roadmap note made concrete, and it is where
   migration pain concentrates.

Where a model stays out: deciding what is *correct* (the oracle, the link check, the differential
runs — deterministic, always), and anything that writes state unchecked.

**The principle under all of it (the person's, 2026-10-03): know what the project does before
anything changes, and judge every change as a match or a mismatch against that.** The word is
**match**, not "right": the baseline is the untouched project's behaviour as it is, bugs and all —
built by the harness from the closure under **one named configuration** (§3.2), which the map
records and the target refuses to compare against another. The project may hold known or unknown
vulnerabilities and room for improvement; the harness does not know and must not pretend to.
Matching is the first step — it is what makes moving code over safe, one bug-for-bug-compatible piece
at a time. Improving is a separate, later step: any change a model proposes as an improvement is, by
definition, a deliberate *mismatch*, and must be labelled as one, judged against a new expected
behaviour the person wrote down, and accepted knowingly — never slipped in with a translation that is
supposed to match. The harness already records expected behaviour at two levels — the driver (every
function's outputs on fixed inputs) and the features (what a person-visible feature materialises as:
exit status, stdout, stderr on fixed runs). Make that the organising rule as the model's role grows:
every proposal — a bin, an order, a seam, a translation — is judged against behaviour recorded
*before* the change, so a wrong change is known wrong with high probability, and a matching one known
to match, rather than trusted. The picture (§3.11) shows the proposal and its reasons; the recorded
behaviour judges the result; **a subject-matter expert signs off**, and nothing is accepted without
that. The gap to close for the map: once a tool is accepted, record its expected behaviour (features
for the tool, under its configuration) *before* its first unit moves. Today nothing enforces it: the
cockpit shows `not-yet` beside a tool without features; §5 puts the step in the order and the person
decides whether a mapped tool's first unit is refused without a features run.

## 4. Tests and checks

The walk and the map:
- The 100 benchmark cases: zero entry points each, one library bin each, the same `include_dirs`
  `bench init` writes today.
- zopfli: one `main` entry, 13 files, `-lm`, zopflipng set aside as C++; the file-list target the map
  writes gives `facts.jsonl` and every `source_hash` byte-identical to the `source_dir` form, and
  `verify u001-katajainen` stays green; the committed program digest of the `source_dir` form is
  unchanged (`d191be5c…`); `bench check --replay` unchanged.
- A tool including `<proj.h>` from a sibling `include/` folder (liblzg's shape); two folders each
  holding `config.h`, and a project `util.h` beside a system one — the ambiguous-include fact, no
  silent pick; a header that includes another in a third folder.
- liblzg from the testing guide: a file-list target over the untouched download, with no copying and
  without Step 1.5's edit, gives the guide's units; the alternative decoder: `unlzg` and `benchmark`
  held because both choices link, the correct "keep `decode.c`" reply accepted, `lzgmini.c` ending as
  "alternative not kept", not shared; the closure recomputed after the choice (`checksum.c` stays only
  with `decode.c`).
- A fuzz driver of many (lz4's shape): each fuzzer an entry of kind `fuzz`, the driver kind `driver`,
  never asked "which to keep", never offered as a tool.
- A made-up project with three `main()`s, a duplicate and a test: the closures, the duplicate
  surfaced, the shared file named after the choice; a collision of a symbol nothing needed inside one
  closure reported; between-program duplicates listed and never asked.
- A stated configuration whose `-D` adds a file and its pthread needs (lz4's shape); without it every
  closure marked guessed; the same project mapped with and without it gives different closures, each
  recorded with its configuration; `ask` refused while guessed.
- A weak/strong pair not a duplicate; a weak undefined symbol a need on Mach-O; common symbols
  merged; the `nm`-free reader on ELF objects made here with `cc -target x86_64-unknown-linux-gnu -c`
  (no Linux machine needed) giving the same names without the underscore; an `asm`-label symbol and a
  data symbol named `main` (not an entry).
- The compile flags pinned and the outside symbols tied to them; a map made under another toolchain
  identity flagged.
- A cut-short walk: file facts, no closures, exit 1 by name; an unreadable folder, a non-UTF-8 file, a
  file over 8 MiB recorded while the walk goes on; a dot-folder listed as skipped; an in-tree
  symlinked `.c` recorded once with its alias; a `.c` included by another file.
- The map run twice gives the same bytes; `root_hash` changes with a header and not with a README;
  `inputs_hash` changes with the configuration.
- A path, a symbol or a file name with control characters, a newline, or the fence delimiter's
  shape never reaches the terminal, `--json` or a prompt unfiltered; `--json` escapes every unsafe
  character.

Security:
- `compile_commands.json` with `@file` (separate and joined), `-B`, `-fplugin`, `-Xclang`, `-o`,
  `-MF`, `-include` of a file outside the root, and a `directory` outside the root: each refused or
  ignored by name; the same flags from the person's configuration file and from `harness.toml`.
- A project `.cargo/config.toml` with a `rustc-wrapper`, and a `rust-toolchain.toml` with a `path`:
  never read (the wrapper's marker never appears); the harness's own toolchain still used.
- A shipped `migration/` refused and named; `--adopt` recorded; a promoted crate with a build script
  refused.
- A pre-placed response file or reply file for another map digest ignored.
- `cc` works under the map profile without the cargo and rustup homes; `--root` at `~` refused; no
  sandbox → refused.
- A compile error whose first line would quote a readable file's bytes: the stored reason is closed,
  the shown line scrubbed.

The model step (with recorded traces replayed in CI, as §16.2 of the briefing requires):
- A forged reply that names an unasked index, skips one, adds a file or a kind outside the set:
  refused in full; a reply for another map digest: not read.
- `undecided` going to the person; a both-choices-link duplicate held, not accepted; a tool
  mislabelled `example` still link-checked.
- `accept` refused with an unresolved duplicate, with a guessed configuration, after the map changed;
  `accept` re-links before writing.
- The folder-name guesses labelled as guesses; `ask` refused with no open questions.

The rest of the harness:
- A per-file `-D` that changes a struct layout in a unit's header is seen by the driver build, the
  boundary check and the features map.
- The v2 digest covers the configuration; the v1 digest is byte-identical to before.
- The file-list `harness.toml` under an older loader reads "schema too new"; a file with both forms
  refused.
- harness-detect's walk confined; the scanner and detect prune the ledger folder when
  `source_dir = "."`.

## 5. Order of work (proposal)

(a) the walk and symbols over one folder with `objsyms`, the map sandbox profile and the harness-
owned runner (bench.rs already builds one without a target), replacing nothing (a `project map` that
reproduces today's `harness.toml` on zopfli and the benchmark's libraries); (c) **the layout (§3.7)
and the file-list form** — `TargetContext`'s ledger folder, `harness.toml` v2 read version-first,
`Base`'s configuration and per-file folders in every compile, the scanner's file-list read, the v2
digest and staleness, confinement in every reader, the whole-program build on perf's path — tested
by a hand-written file-list target over the untouched liblzg download before any map writes one;
(b) the whole-root walk, the configuration, the closures, the entry kinds, the map file, the caps;
(d) the model step and the check, with recorded traces; (e) `project accept`; (f) SCHEMAS, the
tutorial, the testing guide; (g) after the first accepted tool: record its features before its first
unit moves (the step this design adds to the briefing's loop — nothing enforces it yet). Each step
committed green, with its tests from §4; then the review, fix passes checked, mutation checks,
DECISIONS. The picture (§3.11) and the advice (§3.12) come after, as agreed.

## 6. Residuals and what this does not do

- **Projects that need `./configure` or CMake to generate headers** are mapped as far as they
  compile; the map says which files failed on a missing header and names it, when the header is
  truly missing — when a stray header of that name exists elsewhere in the tree, the ambiguous-include
  fact says so and the compile may have used the wrong one. Running the project's own build is out
  of scope (it executes untrusted code).
- **One configuration per accepted tool.** `#ifdef` code whose meaning depends on flags the
  configuration does not state is compiled as the configuration says; a file built twice with
  different flags by the project's build appears once, under the tool's configuration. Revisit when a
  tool's own files need different flags from each other.
- **A shared library is migrated once per tool** that uses it: two targets, two copies of the unit's
  Rust and two verdicts. One Rust library per program, and non-leaf units, are Part 2 of the roadmap
  note.
- **Outside libraries** are guessed from symbols; a wrong guess fails the link and is said. A symbol
  that set-aside assembly or a prebuilt file in the tree may define is reported as "may come from
  set-aside files in X", not guessed as a library.
- **Static functions** are invisible to the closure (they never cross files, except by textual
  inclusion, which step 10 records); the scanner's per-file list still records them.
- **Files the project's build links for a constructor or a linker section**, reached by no symbol,
  are unreached to the map; the project's binary runs them and the harness's rebuild does not. Not
  seen on lz4 or liblzg; the map cannot see it.
- **A map describes the platform it was made on**: the walk and the check run wherever `cc` exists
  and a sandbox does (macOS today; Linux once its sandbox exists); needed and outside symbols come
  from the host's headers and `#if defined(__APPLE__)` branches, and macOS links `libm` and pthreads
  without a flag — a map made here is marked with its toolchain and does not promise to link
  elsewhere.
- **The compile's memory and disk** are bounded by the sandbox's write roots and the time budget,
  not by a resource limit.

## 7. Decisions already made (the person, 2026-10-07)

1. The per-file compiles run by default (2.2 s on lz4's 48 files).
2. `map` always stops and shows; the person accepts which tools become targets — the same review
   gate as the plan.
3. A mapped tool's target and ledger live inside the project (`<project>/migration/…`); RuHarness's
   `targets/` stays for its own fixtures.
4. A library with no tool stays a target on its own (the benchmark's shape).
5. The build's flags are the person's to state when there is no `compile_commands.json`; a model may
   propose them, the person confirms; the baseline is one named configuration.
6. The picture (§3.11) is read-only with one-way data flow and comes after the map; the advice
   (§3.12) after that; an improvements mode (intended mismatches) later still, the vocabulary settled.

## 8. Decisions this revision proposes (for the person, before the build)

1. **The layout for several tools** (§3.7): the project root stays the target root; the first tool's
   ledger at `<project>/migration/`, further tools at `<project>/migration/tools/<id>/` with `--tool`.
   The alternative — one tool per project, zopfli's layout, several tools deferred — is a smaller
   first build but cuts against answer 2 (the person accepts *tools*).
2. **A shipped `migration/` is refused until adopted** (`--adopt`, recorded in the ledger), and cargo
   and rustup run from outside the project with the toolchain pinned. The cost: a project with its own
   `migration/` folder needs the person's one-time adoption.
3. **The configuration file** `migration/map/config.toml` as the way the person states the build, and
   `ask --build` as the model's proposal of it; `ask` refused while the configuration is a guess
   unless `--allow-guessed`.
4. **The commands are `harness project map | ask | accept`**, beside `features map`; the model runs
   at Tier 2 by default with `--provider`/`--model`.
5. **The cockpit's project mode and harness-mcp's map read are separate designs**; the command line
   is the only way in for this one.
6. **A held duplicate** (every choice links) is the person's to resolve at `accept`; the model's pick
   is advice.
7. **The picture's security rules** (§3.11) are fixed now, before its design.

## 9. What changed from draft 0, and why (traceability to the review)

Numbers are the review's, by lens: F (facts), S (security), I (integration), M (the model step).

- §1: what the link proves and does not (M3, F-verifier: no link flag on macOS); the configuration
  as the person's (M4).
- §3.1 step 1: dot-folders listed, files under two paths once (F-missed 2–3), assembly and prebuilt
  set aside (F12), caps and the 8 MiB file (F9, S9), walk errors and non-UTF-8 kept (F9, S-missed 4);
  step 2: `#if` (F-missed 4); step 3: angle-bracket includes, per-file folders, ambiguous includes
  (F1, F2); step 4: the grammar, `directory`, unfound files (S1); step 5: the judge's flags, `objsyms`,
  symbol kinds, weak and common, the underscore, odd names, closed reasons, the temp folder, objects
  by index (F6, F7, S4, S5, S10); step 6: `main` | `fuzz` | `driver`, a function `main` (F5, M1,
  S-missed 3); step 7: definers not added, recomputed closures, the `main`-file rule, collisions,
  "needs from program X" (F4, M-missed 1–4); step 8: sharing after choices (F4, M13); step 9: library
  bins (I6); steps 10–11: textual inclusion, what the build links (F11, F13).
- §3.2 (new): the configuration, the flow, the grammar for every source, the run name, the revisit
  trigger (F3, M4, M5, M6, S1, I-missed 1).
- §3.3: `inputs_hash`, `toolchain`, `configuration`, the new per-file fields, raw strings, sort keys,
  derived ids (F3, F8, F10, S6, M8, I7); one behaviour past a cap (F9).
- §3.4: Tier 2, open questions only with indexes and batches, the closed reply with `undecided`,
  folder-name guesses, the build-flags mode, the binding to the map digest, provider flags and
  traces (M2, M4, M6, M7, M9, M10, M14, I8).
- §3.5: every `main` entry link-checked, choices linked, held duplicates, statuses computed by the
  harness, what the check cannot prove (M2, M3, M13, M-missed 3).
- §3.6: the accept screen, the refusals, re-linking, the acceptance in `harness.toml`, a later `map`
  compares (M10, M11, I7).
- §3.7 (new): the layout, `.gitignore`, the shipped `migration/`, per-file flags in every compile,
  the scanner's file-list read, confinement by crate, the v2 digest and staleness, `harness.toml`
  v2 version-first, the plan sentence, the cockpit and harness-mcp deferred (I1–I5, I9, I13–I15,
  I-missed 2, S2, S-missed 1).
- §3.8: `harness project …`, `--target`, exit codes, the lock, filtered output, `--json` escaping,
  events without the model's names (I12, S7).
- §3.9: rewritten honestly — the map profile, the grammar as the defence, no sandbox refused, closed
  reasons, `objsyms`, the ledger's mitigations, ids, the review gate, bounds (S1–S7, S9–S11,
  S-missed 1, 3, 5).
- §3.10 (new): the limits, one behaviour (F9, S9).
- §3.11: the export's security rules added (S8); §3.12: the baseline under one configuration, the
  features step and that nothing enforces it (M12).
- §4: the full test list (F14, S-missing tests, M15, I11).
- §5: (a), (c), (b), (d), (e), (f), (g) (I10, M12).
- §6: every residual re-stated against the findings (F2, F8, F11–F13, M5).
- docs/PROJECT-MAP-INVESTIGATION.md's counts corrected (`lib/lz4.c` in 29 of 33 closures; tests 11,
  examples 10, programs 1, ossfuzz 1; no benchmark case has a `main`); docs/PROJECT-MAP-ROADMAP.md's
  "the link check catches a model's mistake" qualified (F-missed 1, I-missed 4, M3).
