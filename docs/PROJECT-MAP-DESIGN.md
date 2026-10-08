# Real project layouts: the project map (design, revision 2)

Status: **revision 2 — 2026-10-08; the person's decisions taken (§7); under a re-check of the changed
sections, not to be built before it.** Draft 0 (2026-10-02) was reviewed from four lenses with every
finding verified (docs/reviews/2026-10-07-project-map-design-review.md, the triage at its end);
revision 1 answered that review and was checked by five readers (docs/reviews/2026-10-08-project-
map-rev1-check.md: four lenses against the code and the spike, one for the document as a whole);
this revision answers the check. §9 says what changed and why, in plain words. Part 2 of the roadmap
note (a fuller C–Rust boundary) stays out of scope: this design gets a real project to the point
where today's harness can take over, one program at a time.

**Words used here.** A **program** is any file in the project that defines a `main` (or a fuzz
target); a **tool** is a program the person has accepted as a migration target; a **library** is a
group of files no program reaches; a **configuration** is one named build and its flags; the
**ledger** is the folder of harness-written files (`migration/`, SCHEMAS); the **map** is the file
`migration/map/project-map.json`.

## 1. What it is, in plain words

Today the harness migrates one folder of one program, and the person gathers that folder by hand.
The goal here is the whole project: `harness project map` looks at a whole C project — every folder
— and works out, without a model, which files make up each program it builds, which files several
programs share, which are tests, examples or fuzzers, and which it could not make sense of. It
writes that as the project map, shows it, and stops. The project's **configuration** — which build
counts and the flags it compiles with — is the person's to state (a model may propose one from the
build files; the person confirms), because the map cannot tell two configurations apart. Where the
facts still leave a choice open — two files that define the same function inside one program, when
both choices build — the harness says so and the person decides; a model may be asked for advice. The
person then **accepts** the programs they want to migrate, and each accepted program becomes a tool:
a target that the scanner, the plan, the judge, features, perf and the cockpit handle as they handle
`targets/zopfli` today. The same three steps — map, ask, accept — are commands first and cockpit
dialogs over the same commands second (§7: simplicity means usability).

**What the link check proves, and what it does not.** Linking proves that the chosen files, with the
stated configuration and the guessed outside libraries, define every symbol the program needs exactly
once and produce a program. It does not prove that the program is a tool rather than a test, that the
right definer was chosen when more than one links (liblzg's `decode.c` and `lzgmini.c` both link),
that the configuration is the one the project's own build uses (lz4's tool links with and without
threads, and on macOS with no link flag at all), that the file list matches the project's build, or
that the program runs. The map says this once on every screen that shows a program; the person's
acceptance, not the link, settles those questions.

What does not change: the scanner, the plan, the judge, features, perf and the cockpit keep working on
**one tool at a time**. The map's job ends when it has produced, for each tool, the description those
already understand — a list of files, each with its include folders, and the configuration to build
them — and has said plainly what it left out and why.

## 2. Why now, and what today's rules cannot do

Today a target is one folder (`[target] source_dir`): every top-level `.c` there is the one program,
built with one `cc` command and one set of flags (`include_dirs` must sit inside the folder;
libraries come from `[oracle] extra_link_args`). A real download fails that in four ways at once —
several `main()`s (tools, tests, examples, fuzzers), files for another platform, generated headers
(`config.h`), and C spread across folders — so the person gathers one program's files by hand (the
testing guide does this for liblzg, dropping its two other programs). The investigation and the
spike showed the mechanical part of that gathering is cheap and exact on zopfli, lz4 (48 files, 33
programs, 2.2 s) and liblzg, and that the questions it cannot settle are few and well shaped: the
build's flags (the person's), an in-program duplicate where both choices build (the person's, with a
model's advice), and what kind of program a file is (a label).

## 3. The design

### 3.1 The walk (deterministic, no model)

Input: a project root. Output: facts, every one reproducible from the files, the configuration (§3.2)
and the recorded toolchain (§3.3).

1. **Files.** Walk the whole root with harness-core's confined walk. Pruned: `.git`, every folder
   whose name starts with a dot (pruned by name at any depth, listed in `skipped_folders` with a
   file count from a count-only pass), and all of `<project>/migration/` (§3.7). A link that leaves
   the root or points nowhere, a FIFO or a device, and a folder the walk cannot read are listed in
   `walk_issues: [{path, why}]`; a link to a file or folder inside the root is followed, and a file
   reached under two paths is recorded **once**, under the path with no link in it, with the other
   paths as `aliases`. Record every `.c` and `.h`: path, size, blake3 (hashed by streaming, so a
   file over the size cap is still hashed). Non-C sources (`.cc`, `.cpp`, `.cxx`, `.m`, `.go`,
   `.rs`, `.py`, `.js`, `.lua`, `.pas`), assembly (`.s`, `.S`, `.asm`) and prebuilt files (`.a`,
   `.o`, `.so`, `.dylib`) appear only as **counts per folder** in `set_aside`, never as entries and
   never read. Caps (§3.10): 20 000 walked `.c`/`.h` files, depth 32, a file over 8 MiB is recorded
   `too_large` and neither parsed nor compiled. A walk error or a file that is not UTF-8 is a fact of
   that path, never a stop; a non-UTF-8 `.c` is still compiled (`cc` reads bytes; Latin-1 comments
   are common), only not parsed.
2. **Per-file facts from the scanner.** Over every `.c` **and `.h`**: functions defined, calls made,
   includes — what `harness scan` records today. Include facts come from every `#if` branch (the
   parser does not evaluate them); symbol facts (step 5) come from the one configuration. A file the
   parser cannot read is recorded as such and kept.
3. **Include folders, per file.** For an include name N (which may hold folder parts: `proj/api.h`),
   a **project candidate** is every walked folder F where `F/N` is a walked header, matched by whole
   path parts (so `<proj/api.h>` at `include/proj/api.h` finds `include/`). The **system** holds N
   when one of the toolchain's own search folders does — the list is read once per toolchain from
   `cc -E -v` and recorded with it (§3.3). A quoted include is tried in the including file's own
   folder first, then among the project candidates, then the system; an angle-bracket include never
   tries the including file's folder (liblzg's tools include `<lzg.h>` from `src/include/`). Headers'
   own includes are followed the same way to closure, and each header's status is recorded
   (`included by: [files]`, or "included by no file"; a header included by no file is still a
   candidate). A file gets only the folders its includes need. When more than one project candidate
   holds N, or a project candidate and the system both hold N (two `config.h`; a project `util.h` or
   `unistd.h` beside the system's), the walk records an **ambiguous include** fact listing every
   candidate and picks none: for the compile in step 5 the system wins over a project candidate, and
   the configuration (`compile_commands.json`'s or the person's `-I` folders) decides between project
   candidates; until it does, the fact says the compile may have used the wrong header.
4. **Build evidence.** `compile_commands.json` at the root or one level down is read, never executed:
   for each file it names, the flags that pass the grammar of §3.2 are taken and the rest counted as
   ignored; the entry's own compiler (`arguments[0]`, or the first word of `command`) and its
   `output` are never used — `cc` from the allowlist always compiles; an entry whose `directory`
   resolves outside the root is ignored and counted; a file it names that the walk did not find is
   recorded and never compiled; a file it lists twice with different flags is a **flags differ**
   fact. Build files found by fixed names (`Makefile`, `GNUmakefile`, `*.mk`, `CMakeLists.txt`,
   `configure`, `configure.ac`, `meson.build`) are recorded by path in `build_files`; nothing is run.
5. **Symbols.** Compile each `.c` to an object under the map's sandbox profile (§3.9), with the
   working folder in the harness's own work folder (§3.9), into a fresh temporary folder, each object
   named by the file's index: `cc -c -w` with the judge's own base flags — today `-O2
   -ffp-contract=off` — then the configuration's flags (§3.2), then the file's include folders, each
   as `-I<abs>`, then `--` and the file's absolute path; plus `-MD -MF <temp>/<index>.d`, so the
   compiler itself lists every file the compile read (macro-built includes and `#embed` included).
   A compile that read anything outside the project root and the toolchain's own folders is
   `outside_includes: true`; its symbol names are **counted, not stored** (a readable file's bytes
   can become a symbol name), and the screen says so. Read each object with harness-oracle's
   `objsyms` (safe Rust, no `nm`, every offset bounds-checked), **extended for the map** (§5 step a):
   each **external** symbol it defines with a kind — `function`, `data`, `read-only`, `bss`,
   `common` (a Mach-O common symbol is a definition) — and a `weak` flag (Mach-O `N_WEAK_DEF`;
   ELF `STB_WEAK`); each external symbol it needs, with `weak` for a weak reference (`N_WEAK_REF`;
   ELF weak undefined) — a weak need still needs a definer at link time on macOS and resolves to
   null on ELF, and is recorded as a need with its flag. Local symbols are never read. One leading
   `_` is stripped on Mach-O so names equal the scanner's. A name not shaped like a C identifier
   (an `asm` label can hold any text) is counted as an odd name and never stored. An object over
   64 MiB is not read (`compiled: {reason: other, detail: too-large-object}`). Every object is
   deleted as soon as it is read. A file that does not compile is recorded with a **closed reason**
   (`missing-header <name>` with the header's relative name, `syntax`, `other`) and the place in the
   project; the raw first line is shown on the terminal once, scrubbed of machine paths and
   filtered, and never stored. The walk never stops on it.
6. **Programs.** A file that defines a *function* `main` is a program of kind `main` (a data symbol
   named `main` links but is no program); a file that defines `LLVMFuzzerTestOneInput` and no `main`
   is kind `fuzz`; a `main` whose need is met by two or more `fuzz` files is kind `driver` (lz4's
   `ossfuzz/standaloneengine.c`, linked once with each fuzzer). A `driver` is recorded with the
   fuzzers it serves and is never a tool; a `fuzz` program is listed with its closure, linked only
   with the project's driver when there is one (shown "fuzzer, not linked" otherwise), and never
   offered to `accept` in this design. Programs are never "unreached". Each program gets its id
   (§3.3) and a **kind guess** from its folder: `tests/`, `test/` → test; `examples/` → example;
   `bench/`, `benchmarks/` → benchmark; `fuzz/`, `ossfuzz/`, `fuzzers/` → test; anything else →
   tool. A kind is a **label**: it changes no closure, no check and no acceptance.
7. **Closures.** For each `main` and `fuzz` program, from its file: add every file that is the
   **single** definer (among non-program files) of a symbol the set needs, until nothing new is
   added. A program file is never pulled into another program's closure: a need met only by another
   program's file is recorded as **"needs a symbol from program X"**, not as an outside symbol. A
   needed symbol with two or more definers among non-program files is a **duplicate** of this
   closure: its definers are recorded as *alternatives, pending*, none of them added; a definer
   already in the closure for another symbol makes it a **collision** (the link would fail), not a
   question. Rules of the linker apply: a weak definition beside a strong one, two weak ones, or
   several common definitions of one name are neither a duplicate nor a collision (the strong one,
   else the first in path order, defines it). After a choice (§3.5) the closure is **recomputed from
   scratch** with the chosen file (a file the unchosen alternative had pulled in must not stay). Over
   the finished closure, every symbol defined by two or more of its files is a collision, reported
   even when nothing needed it. What stays unresolved is the closure's **outside symbols** (libc,
   libm, an outside library).
   A closure is **incomplete** when a duplicate is pending, when any file that did not compile, was
   too large or could not be read has parser facts that define one of its outside symbols
   ("may be defined in X, which did not compile"), or when the walk could not read a folder. A file
   whose parser facts define `main` but which did not compile is listed as **a program that did not
   compile**; it has no closure, and files reached only through it are not offered as a library.
   Under a guessed configuration (§3.2) failed compiles are expected and said so.
8. **Shared files, after the choices.** A file in two or more closures is shared by those programs.
   Duplicates between programs that never meet in one closure (lz4's 14 helper names across
   `examples/` and `tests/`) are listed, not questions.
9. **Libraries.** A `.c` that is no program, no pending alternative and in no closure is unreached.
   Two unreached files join when one needs a symbol the other alone defines; the groups so formed
   (a lone file is a group) are **libraries**, each listing the files outside it that it needs. The
   benchmark's shape — all 100 cases have zero programs and one library — and a library a project
   ships without a program. A library can be accepted as a target by itself (§7, answer 4).
10. **Textual dependencies.** A `.c` that another file includes (`#include "lz4.c"`, from a `.c` or
    a `.h`) is a fact (`included_by`) shown beside the closure; such a file is not offered as a unit
    on its own without a warning, because moving it to Rust leaves the including file still
    compiling its C text.
11. **What the project's build links beyond the closure** is not known without the build system; the
    map says, where it shows a program, that the closure is what the program needs, not what the
    build links (lz4's Makefile links `lib/lz4file.c` into the tool, which only an example and a test
    reach; without threads `programs/util.c` is unreached); a file the build links only for a
    constructor or a linker section is the exception the map cannot see (§6).

Steps 5–9 are what the spike's 90-line script did on zopfli, lz4 and liblzg, less the weak and common
rules, the recomputed closures, the collisions and the libraries, which the review added; the
closures were identical at `-O0` and `-O2` and only the outside names moved, which is why step 5 fixes
the flags.

### 3.2 The configuration: which build counts, and its flags

**The person's decision (2026-10-07):** without a `compile_commands.json`, the person states the
build and its flags; a model may read the build files and propose them; the person confirms. The
spike showed why: lz4's tool gains a file and pthreads with `-DLZ4IO_MULTITHREAD`, its Makefile and
Meson build it so and CMake does not, and every configuration links. So the baseline is **one named
configuration per accepted tool**, and a map's closures carry the configuration they were made with.

- **The file.** `migration/map/config.toml`, written by the person, holds one or more
  `[[configuration]]` entries: `name` (`make`, `meson`, `cmake`, or the person's own word), `from`
  (what it stands for: `make | meson | cmake | compile_commands | stated`), `flags` (in order,
  through the grammar below; `flags = []` states "no flags" and is not a guess). `project map
  --configuration <name>` maps under one; without the flag, the only entry, or a guess when there is
  none. The **run name** (what features run the program as: lz4's tool is `lz4`, not `lz4cli`) is a
  program's, not a configuration's: `accept --run-name`, default the program file's stem.
- **Order of trust and the flow.** The first `project map` runs with no configuration (2.2 s on lz4)
  and shows the build files it found and the closures, every one marked `guessed`. A
  `compile_commands.json`, when present, is **a proposal like the model's**: the map uses it and
  shows it, but the source stays `guessed` until `config.toml` names it (`from =
  "compile_commands"`), because it is one build's output and the project's builds disagree. From
  such a file a program's configuration is the flags shared by every file of its closure; when they
  differ, or a file is listed twice with other flags, the map records a **flags differ** fact for
  that program and the source stays `guessed`. The person writes `config.toml` — or asks the model
  (`project ask --build`, §3.4) and copies what they accept — and runs `map` again; the closures are
  now `stated`. Questions about programs and duplicates (`project ask` without `--build`) are refused
  while the configuration is a guess unless `--allow-guessed` is given, because a wrong flag changes
  the closures and the questions; `accept` never takes a guessed configuration.
- **The flag grammar, one for every source** (`compile_commands.json`, the model's proposal, the
  person's file, and `harness.toml` when a target is loaded — `harness.toml` is untrusted input):
  - `-D<name>` and `-D<name>=<value>` with the name a C identifier; `-U<name>`;
  - `-I<dir>`, `-iquote<dir>`, `-isystem<dir>`, `-include<file>` with the path inside the project root
    after resolving and not under `migration/`;
  - `-std=` with a value from a fixed set (`c89 c99 c11 c17 c23 gnu89 gnu99 gnu11 gnu17 gnu23`);
  - `-pthread`;
  - `-f` flags from an **exact list of spellings**, extended only in RuHarness's own code, never in a
    target file: `-fno-strict-aliasing`, `-fwrapv`, `-fno-common`, `-fcommon`, `-fPIC`, `-fpic`,
    `-fsigned-char`, `-funsigned-char`, `-fno-builtin`, `-fvisibility=hidden`,
    `-fvisibility=default`;
  - `-O0`–`-O3` are accepted and **recorded, never applied**: every compile keeps its own level (the
    map and the judge `-O2`; the boundary check and the driver's validation their own);
  - every path is absolute when passed, and `--` separates the options from the source; no value may
    start with `@` or `-` (clang reads an `@file` argument as a file of options, joined or not);
  - nothing else — never `-B`, `-fplugin`, `-fpass-plugin`, `-fuse-ld`, `-Xclang`, `-load`, `-o`, the
    `-M` family, `-Wl`, `-Wa`, `-wrapper`, `-x`, `-ftime-trace`, `-fprofile-instr-use`,
    `-fmodules-cache-path`. Only `-pthread` and today's `-l<name>` rule reach a link.
  A flag outside the grammar is refused by name, whoever proposed it.
- **Recorded** as `configuration {name, from, source: compile_commands | stated | guessed, flags,
  digest}` on the map (one at a time), in the checked reply, and in the accepted tool's
  `harness.toml` (its own copy). A later `map` compares an accepted tool only with a map made under
  the tool's configuration name, never with another. **Revisit when:** a tool's own files need
  different flags from each other — lz4's tests build the shared `lib/lz4.c` with `-DLZ4_DEBUG=1`
  where the tool does not, which a configuration per tool absorbs; different flags inside one tool
  would not be.

### 3.3 The project map file: `migration/map/project-map.json` (`ruharness-project-map` v1)

Written **only by `project map`** and only from the walk and the map's own link checks; the model's
labels and picks live in `project-map.reply.json` (§3.4), the person's picks in the accepted tool's
`harness.toml` (§3.6). Facts and summaries only — **no source text**. Every string from the project
(a path, a symbol name) is stored raw, so it can be matched exactly, and display-filtered only when
shown; it is fenced as untrusted when handed on: in a prompt with triage's deterministic-nonce blocks
(§3.4), in harness-mcp with its JSON fence.

```
{ schema, schema_version, root_hash, inputs_hash,
  toolchain: {cc, target, cflags, system_include_dirs: [path]},
  configuration: {name, from, source: compile_commands | stated | guessed, flags, digest},
  files:    [{path, aliases: [path], kind: c | h, bytes, blake3, parsed: bool, too_large?: bool,
              not_utf8?: bool, compiled?: ok | {reason: missing-header | syntax | other, header?,
              detail?, at?}, outside_includes?: bool, functions: n, includes: [path],
              include_dirs: [path], ambiguous_includes: [{header, candidates: [path | "system"]}],
              included_by: [path], defined_symbols: [{name, kind, weak?}],
              needed_symbols: [{name, weak?}], odd_names: n, flags_from?: compile_commands}],
  programs: [{id, path, kind: main | fuzz | driver, kind_guess: tool | test | example | benchmark,
              serves?: [path], compiled: bool}],
  closures: [{program: id, files: [path], incomplete: bool, uncompiled: [path], outside: [sym],
              needs_from: [{sym, program: id}],
              duplicates: [{index: "d1", symbols: [sym], definers: [{index: "d1.1", path}],
                            links: [index], choice?: {keep: index, by: links}}],
              collisions: [{sym, definers: [path]}], flags_differ?: [{path, flags}],
              linked?: ok | {missing: [sym], doubled: [sym]}, questions: [index]}],
  shared:   [{file: path, programs: [id]}],
  libraries: [{id, files: [path], needs_from_outside: [path]}],
  set_aside: [{folder, lang, count}], skipped_folders: [{path, count}],
  walk_issues: [{path, why}],
  build_evidence: {compile_commands: present | absent, ignored_entries: n, build_files: [path]},
  limits_hit: [{limit, at}] }
```

- `root_hash` is SCHEMAS' file-set hash (blake3 over the sorted paths and contents) of every walked
  `.c`/`.h` plus `compile_commands.json` when present. `configuration.digest` is blake3 of the
  canonical JSON of `{name, from, flags (in order)}`. `toolchain` is `{cc: the first line of cc
  --version, target: cc -dumpmachine, cflags: the judge's base flags, system_include_dirs}`.
  `inputs_hash` is blake3 of the canonical JSON of `{configuration.digest, toolchain}` (a stale
  picture is visibly stale after a compiler change). No commit hash is recorded: reading one means
  running `git` in an untrusted tree.
- Flags and include folders keep their order; every other list is sorted by its first field's bytes,
  so two runs give byte-identical files (the golden test; SCHEMAS' canonical-serialization rule).
- **Ids** are made by the harness, never by a model: a program's id is `t-<stem>` where the stem is
  its file name without `.c`, lowercased, with every character outside `[a-z0-9_-]` replaced by `-`,
  at most 64 characters; when two collide (ignoring case) the full path form `t-<folder>-<stem>`;
  never `migration`, `map`, `tools` or a name already used. A library's id is `l-<stem of its first
  file in path order>`, made unique the same way. Question indexes are assigned by `map` in path
  order: programs open to a question `p1…`, definer sets `d1…`, definers `d1.1…`; they are printed
  with the map and used by `accept --keep` with or without a model's reply.
- Past a cap (§3.10) the map holds the file facts only, no closures, and the command exits 1 naming
  the limit.

### 3.4 Asking a model (Tier 2, optional, the `external` hand-off as everywhere)

The harness settles what it can first (Tier 0, §3.5): `map` links every closure that has no pending
choice and every choice of each small duplicate set. What remains **open** is: (1) the
configuration, while it is a guess; (2) a duplicate set the link could not settle (several choices
link, or the set is over the limit); (3) a program's kind, only when the person asks. With nothing
open (zopfli), the map is final without a model and the person still accepts (answer 2).

- **`project ask --build`** is allowed only while the configuration is a guess and asks nothing
  else: it sends the build files the walk found (`build_files`, by their fixed names, read with a
  size cap of 64 KiB each and 128 KiB in all, never through a link; a file over the cap is named as
  not sent), each in its own deterministic-nonce block (triage's fence: the trusted part holds only
  harness-written text, project text sits in nonce-delimited blocks with `<` escaped), and asks for
  one configuration in the grammar of §3.2: `{"flags": [{"flag": "…", "cites": ["path:line"]}],
  "assumptions": ["…"]}` — each flag through the grammar, each cited path one of the files sent,
  each assumption one line of at most 200 characters shown labelled as the model's words. It writes
  `migration/map/config.proposed.toml` and changes nothing else; the person copies what they accept
  into `config.toml` and runs `map` again.
- **`project ask`** (the other questions) sends only the open questions, each by its index with
  that item's facts inside one fenced block — for a program: its path and folder; for a duplicate
  set: the symbols and each definer's path and folder (no compiler-derived counts, so the request
  bytes and the trace keys are the same on every platform) — programs first, then definer sets, in
  index order, at most 10 per call, one hand-off per call. The strict reply is a JSON array, one
  object per index, in a fixed field order: `{"item": "p3", "kind": "tool | test | example |
  benchmark | other", "name": "…", "purpose": "…"}` (`name` 1–40 printable characters, `purpose`
  one line of at most 200, control characters refused, both display-only) and `{"item": "d1",
  "keep": "d1.2" | "undecided", "reason": "platform | alternative-implementation | cannot-tell"}`;
  a reply that names an index not asked, skips one, or adds anything is refused in full, as
  triage's "exactly the requested ids" rule does. A duplicate's answer applies to every closure that
  holds the same definer set; the check (§3.5) links it per program. `ask --programs p3,p7` puts
  named programs' kinds to the model when the person wants a second opinion on a guess.
- **Provider, tier, traces, resume.** The provider and model come from `--provider`/`--model` (as
  `migrate` and `gen-driver` take them), Tier 2 by default (classifying programs and choosing
  definers is judgment, not summarization); escalation is the person's re-ask at a higher tier,
  because a wrong answer that links produces no failure to escalate on. `ask` exits 1 with the
  `awaiting` event and the exact resume command, which carries every flag that shapes the request
  (`--build`, `--programs`, `--provider`, `--model`, `--allow-guessed`), as `observe` does; traces
  live under `migration/map/traces/`; record and replay work as today's hand-off (the
  `TraceAdapter`, `checked_complete`) with one new module like triage's (prompt assembly and a
  strict validator). Live: one retry with the error appended; external or replay: a hard error
  naming the response file, with "delete it and answer again". The checked replies accumulate in
  `project-map.reply.json`, keyed by index, which records the map's `root_hash` and `inputs_hash` it
  answered; `accept` refuses a reply bound to other digests. **This guards against a stale reply,
  not a forged one** — a download can compute both digests — which is why the model's words are
  only ever labels and advice, re-linked at `accept`, and why a ledger made elsewhere is adopted
  before anything reads it (§3.7).

### 3.5 The checks (deterministic)

Run by `project map` (so the map's screen shows what the link proved), and again by `accept`.

1. **Every `main` program whose closure has no pending choice is link-checked**, whatever kind
   guess or model label it has: compile its closure and link it into one program with the outside
   libraries guessed from the outside symbols (`-lm` for `log`, `sin`…; `-lz` when `deflate`/
   `inflate` are needed and zlib is on the system; `-lpthread` for `pthread_*` where the platform
   needs it). The result is `linked: ok` or `{missing, doubled}`, computed from the `objsyms` facts
   and the linker's exit status, never from the linker's text (it can carry project text). A `fuzz`
   program is linked only with the project's driver; a `driver` is never linked alone. Nothing built
   is ever run.
2. **Duplicates are settled by linking** when a set is small (at most 4 definers in a set, at most
   16 choices for the program): each choice is linked with the closure **recomputed for it**
   (liblzg's `unlzg` with `decode.c` needs `checksum.c` too; with `lzgmini.c` it does not). Exactly
   one choice links: recorded `choice: {keep, by: links}`, no question. None links: the program is
   recorded as not linking with the missing symbols of its best choice. Several link, or the set is
   over the limit: the program is **held** — "linking cannot tell d1.1 from d1.2" — and the set is a
   question for the model (advice) and the person (the decision, §3.6). A model's pick is never
   linked as if it settled anything; it is kept beside the set as advice.
3. **Every file's status is computed by the harness** from the walk and the choices, never from a
   reply: program, in a closure, pending alternative, alternative not kept, shared, library,
   unreached, set aside, did not compile, too large, could not be read. Nothing is silently dropped;
   a reply cannot change a status except through the person's choices.
4. **What the check cannot prove** (§1) is printed once per screen. A model's kind, name and
   purpose are shown labelled as the model's.

### 3.6 The person's gate: the screen, and accepting a program

`project map` and `project ask` **always stop and show** (answer 2); nothing becomes a tool until
`project accept <id>`. The screen shows, per program: its kind guess (and the model's label, when
asked); the closure's files by folder, with "incomplete" and why; the outside symbols and the guessed
libraries; the configuration's name, flags and source; each duplicate set — settled by linking, or
held with both definers, the model's advice and reason when asked, and the indexes to use; what the
link proved and did not (once per screen); what the project's build may link beyond the closure; a
driver's served fuzzers and each fuzzer's link result. Every refusal and hedge is one plain sentence
with what to do next.

`accept <id> [--keep d1=d1.2]… [--run-name NAME]` resolves held sets with the person's picks
(recorded `by: person`), and refuses — in one sentence each — while any duplicate set of the closure
is unresolved, while the closure is incomplete, while the configuration is a guess, when the map's
digests changed since the reply it uses, when a root `harness.toml` exists (the project is already a
folder-form target: move it, or map a copy), and when the link fails; it **re-runs the link check**
before writing anything, never trusting a stored reply. It then writes the tool's target (§3.7): a
`harness.toml` in the file-list form — the files, each with its include folders, the configuration
(its own copy), the guessed `extra_link_args`, the run name, the map's `root_hash` and
`inputs_hash`, and the picks — so a later `map` can report, for each accepted tool: closure changed,
configuration changed, new programs, a tool that no longer links. The acceptance is the written
`harness.toml`, reviewed with `git diff` as the plan is; no separate record. A shared file accepted
with two tools becomes units in each tool's target (two ledgers, two Rust copies; §6). A library is
accepted the same way: `accept l-<stem>` compiles its files (no link, no run name) and writes the
target with no program.

### 3.7 Where a mapped tool lives, and what the rest of the harness must learn

**The layout.** The person decided (answer 3) that a mapped tool's target and ledger live inside the
project. Today the folder that holds `harness.toml` does five jobs: it holds the config, it is the
parent of the ledger, the base of every path in the facts and the plan, the folder every compiler
input must stay inside, and the sandbox's read root. For several tools in one project the first two
jobs split from the other three:

- The **project root** stays the target root for containment, the base of every path and the
  sandbox's read root: `--target <project>` as today.
- `<project>/migration/` is the ledger of a **folder-form** target only (a root `harness.toml`,
  zopfli's layout: nothing moves). Every **mapped tool** lives at `<project>/migration/tools/<id>/`,
  which holds its `harness.toml` and its ledger; `migration/map/` (the map, `config.toml`, the
  reply, the traces) is project-level, as is `migration/.gitignore`, which the **first `map`**
  writes (`build/`, `.lock`, `traces/`, `.promote-*/`, `.*.prev/`, `.replay-*/`, `target/`, and the
  attempts' `candidate/target/`).
- **Finding a target.** `--target <project>` alone loads `<project>/harness.toml` (folder form); when
  there is none and `migration/tools/` holds exactly one tool, that tool; otherwise the command
  refuses and names the tools. `--tool <id>` loads `migration/tools/<id>/harness.toml` and never the
  root file. **Every subcommand that takes `--target` takes `--tool`**, and so do the cockpit and
  harness-mcp (the id checked by the id rule of §3.3). `TargetContext` gains the ledger folder
  beside `root`; the `Ledger` API, `features_dir`, `perf_dir`, the component lists passed to
  `safe_ledger_dir`, the driver path `gen-driver` writes, perf's output paths and the cockpit's draft
  paths derive from it (about 51 production `Ledger::new` sites and a dozen path helpers; the
  check's count). The plan's `driver` paths stay root-relative. The writer lock stays per ledger;
  `project map`, `ask` and `accept` take a project-level lock, `migration/map/.lock`.
  `sync-runtime` writes one generated block per tool, marked with its id, naming that tool's ledger
  paths.
- **A project's own `migration/` folder** (its database migrations, say) cannot hold the harness's
  files: `project map` refuses in one sentence — "this project has a migration/ folder of its own;
  move or rename it, or map a copy" — and §6 records the limit (revisit if it is common).

**A ledger made elsewhere is adopted before anything reads it.** A download can ship a ready-made
`migration/` — a `harness.toml`, green verdicts and `verified` statuses whose digests it computed, a
promoted crate with a build script, a `target/` folder, a reply file, response files for the
hand-off. Nothing inside the project can prove who made it, so **adoption is a per-computer trust
decision kept outside the project**: `~/Library/Application Support/ruharness/adopted.toml` records
the canonical roots the harness created ledgers for on this computer (the first `scan` or `map` on
a folder with no `migration/`) and the ones the person adopted. **Every command that opens a ledger**
— not only `project map` — refuses a ledger at a root not in that file, in one sentence: "this
folder already holds migration results made elsewhere (N units, M verified): to trust them here, add
`--adopt` once". The cockpit asks the same in a dialog. Adopting records the root, deletes every
crate `target/`, `build/` and `.promote-*/` under the ledger, and says that the verdicts are claims
made elsewhere until `verify` runs them here. On a new computer the person is asked once again; the
ledger alone still holds everything needed to resume (briefing §2.2), so cold resume is unaffected —
only the one-time trust question is per computer. RuHarness's own fixtures (`targets/zopfli`, the
benchmark cases) are adopted by the test suite's set-up and by `bench init`.

**Every cargo and rustc child runs outside the project.** cargo reads `.cargo/config.toml` from its
working folder and every folder above it, and rustup reads `rust-toolchain.toml` the same way, so a
download could supply a `rustc-wrapper`, a `linker`, or a `path` to its own `cargo`. The oracle
therefore starts cargo and rustc with their working folder in the harness's own **work folder**
`~/Library/Caches/ruharness/work/` (never under `/tmp` or `$TMPDIR`; no sandbox profile may write it
or its ancestors), with `--manifest-path` and every path absolute, the toolchain pinned through
`RUSTUP_TOOLCHAIN=stable` (RuHarness's own `rust-toolchain.toml` says `stable`; the variable is already
on the tool environment's fixed list; sandbox.rs's "narrow exception 2" for the ancestor file goes),
and a `PATH` of absolute entries outside the project root only. Before any cargo run on a unit crate
(verify, perf, features, the benchmark build) the crate folder must hold exactly `Cargo.toml` equal
to the harness-owned manifest, an optional `Cargo.lock`, and `src/{lib,logic,ffi}.rs` with `lib.rs`
equal to the harness's; anything else — a `build.rs`, a `.cargo/`, another file — refuses the unit by
name (today only `promote`'s closed copy list keeps a `build.rs` out, and cargo runs one it finds
beside an exact manifest).

**The configuration's flags and each file's include folders reach every compile.** A unit's C and
the headers it shares with the driver are compiled in about a dozen places today (the driver-shape
compile, the C and Rust driver builds, the driver's self-validation and its mutants, the boundary
check's wrapper, the features map's probed build, perf's objects), every one through the target's
single include list and with no `-D`. Under one configuration per tool the flags are target-wide:
`Base` carries the configuration's flags and a **per-file table of include folders**, and every
compile asks `Base` for a file's arguments — the configuration's flags first, then the file's own
folders. A compile that takes several C sources (the driver builds, self-validation, the mutant
link, the features map's plain program, the whole program) compiles each file to an object with its
own folders and links once, adopting perf's `compile_objects` and `link_side`, which exist and are
proven equal to the one-command build. **The driver's folders** are, for each `.c` of the unit, its
own folder, then its recorded folders, then the folder of every header in the unit's include
closure, without repeats, in that order; the header names a driver may write (`unit_header_names`)
are each header's name relative to those folders. The benchmark scorer builds suite cases and is
untouched.

**The scanner** reads a file-list target as: the listed `.c` files plus every header reached through
each file's own include folders, both include forms (§3.1 step 3), to closure, inside the project
root and never under `migration/` (a `harness.toml` pointing into the ledger would put model-written
files into prompts and builds). Angle-bracket resolution is new to the scanner; for today's
`source_dir` targets no project header is included with angle brackets (zopfli and all 100 benchmark
cases checked), so their facts stay byte-identical (the test in §4 covers all 101 and zopfli's
`findings.jsonl`). The scanner keeps walk errors and non-UTF-8 files as facts instead of stopping,
checks a file's size before reading it, and, like harness-detect, prunes `migration/`;
harness-detect's own recursive walk (no containment, no cycle guard) is replaced by the confined
walk.

**Confinement, restated for the file-list form:** nothing outside the listed files and their reached
headers reaches a prompt or a compile, and nothing under `<project>/migration/` — any tool's ledger,
the map — ever reaches a scan, a prompt or a compile as project C. Every reader of `source_dir`
learns the rule: harness-scan (the walk, `repo_relative`, `program_c_files_in`), harness-detect (its
walk), harness-core (the planner's `source_hash` over the include closure; the features digest and
its staleness), harness-llm (`read_sources` for `migrate` and `gen-driver`), harness-oracle (the
boundary check's unit-header rule and `unit_header_names`; the features map's mirror, which copies
the listed files and headers at their project-relative paths; `Base`), harness-cli
(`stale_fact_files`, `program_digest_now`), harness-tui (the tree, preflight, the read model) and
harness-mcp.

**The program digest and staleness.** The file-list form hashes a **v2 record** — the files, each
file's include folders, the configuration's name and flags, the link arguments, the run name — so two
configurations of lz4 differ; the `source_dir` form keeps its v1 record byte for byte (zopfli's
committed digest `d191be5c…` must not move). Staleness keeps today's rule — stale only where a scan
would record otherwise, so a scan clears it: a listed file or a reached header changed or vanished,
or a `.h` appeared in a file's include folders. A changed `root_hash` is **a notice, not staleness**:
`state status` and the cockpit compute it and say "the project changed since this tool was accepted:
run `harness project map`, then `accept` again".

**`harness.toml`, file-list form.** Read **version first** (today's loader parses the struct before
checking the version, so an older harness says "missing field `source_dir`" instead of "schema too
new": `TargetConfig::load` parses to a table, reads `schema_version`, then deserializes),
`schema_version = 2`; `[target] files = [{path, include_dirs}]`, `[target] configuration = {name,
from, flags}`, `name` (the run name), `map = {root_hash, inputs_hash}`, `picks = [{set: [sym],
keep: path}]`; a file holding both `files` and `source_dir` is refused by name; the flag grammar is
checked at load.

**The symbol readers.** verify's driver-shape and capability checks parse `nm`'s lines today, over
objects built from the project's headers, where an `asm` label can forge a line; they move to
`objsyms` (its `undefined()` exists) with the identifier filter, and `nm` leaves the required tools.

**The plan:** no change in rule. Units and dependencies come from the facts over the tool's file
list; a call to a project file outside the list reads as an outside call (the capability check
already reads unresolved names). A shared file is a unit in each tool that holds it (§6).

**The cockpit and harness-mcp.** The command line comes first; the cockpit runs the same three acts
(the person's rule, §7). Opened on a project root: with a root `harness.toml` it is a folder-form
target as today; with mapped tools and no `--tool`, it lists them and the person picks one; with
neither it offers **Map the project**, and after a map **Ask** and **Accept a program**, each a dialog
that says what it runs, how long it takes and what it writes, and runs the `harness project` command
— the same words as on the command line. That is the whole project mode of this design; a map
*screen* (the picture, §3.11) and harness-mcp's read of the map are designs of their own. Once a tool
is accepted, both work on it as on any target; the cockpit's tree shows the tool's files under their
real folders, with the rest of the project greyed as "not part of this tool".

### 3.8 The CLI

- `harness project map --target DIR [--configuration NAME] [--json] [--adopt]
  [--allow-unsandboxed]` — writes `migration/map/project-map.json`; prints the configuration and its
  source, the programs (id, kind guess, files, outside symbols, incomplete and why, linked or not,
  held sets with their indexes), shared files, libraries, unreached, set-aside and skipped counts,
  files that did not compile with their closed reason, ambiguous includes, flags-differ facts. Exit 0
  when a complete map was written; 1 refused or cut short (no C found; a cap hit — the file facts
  are still written; a ledger made elsewhere not adopted; the project's own `migration/`; no sandbox;
  a root that is or holds the home folder); 2 usage. Output lines filter newlines and tabs in every
  project string (a file name can hold a newline; the review gate must not be forged); `--json`
  escapes every character `unsafe_to_show` names as `\uXXXX` (today's comment that serde does so is
  wrong; a small fix of its own, with SCHEMAS' sentence).
- `harness project ask --target DIR [--build | --programs IDS] [--provider P] [--model M]
  [--allow-guessed] [--allow-unsandboxed]` — §3.4; exits 1 with the `awaiting` event and the resume
  command; writes `config.proposed.toml` (with `--build`) or `project-map.reply.json`. Refused
  without open questions.
- `harness project accept <id> --target DIR [--keep d1=d1.2]… [--run-name NAME]
  [--allow-unsandboxed]` — §3.6; exit 1 when refused.
- Every other subcommand, the cockpit and harness-mcp gain `--tool <id>` (§3.7); `--adopt` is
  accepted by every command that opens a ledger.
- Events (`--json`): `project-file`, `project-program {id, path, kind, kind_guess, files, outside,
  incomplete, held: [index]}`, `project-link {id, ok | missing: [sym] | doubled: [sym]}`; they carry
  paths and symbol names verbatim as the ledger's events do, never a model's names. SCHEMAS' writer
  table gains `project map`, `ask` and `accept` with their lock.

### 3.9 Security

The threat model is the briefing's: the download is untrusted input, the model's reply is untrusted,
and now the ledger lives inside the download.

- **The map's compile and link run under a map sandbox profile of their own** — today's tool profile
  denies writes outside its folders and starts through the system (`NO_STARTS_THROUGH_THE_SYSTEM`:
  no LaunchServices, no Apple events, no launchd jobs) but allows reads of everything outside the
  home folder (so `/Users/Shared`, `/Volumes`, the temporary folders) and writes to all of
  `/private/tmp` and `/private/var/folders`. The map profile keeps the read rule but names it, adds
  the project root, opens neither the cargo nor the rustup home (`cc` needs neither; verified to
  compile and link so), and allows writes only to the map's fresh temporary folder, with `TMPDIR`
  pointed at it. The flag grammar of §3.2 is what keeps the compiler from loading or running anything
  from the project (`@file`, `-B`, `-fplugin`, `-fuse-ld`, `-Xclang -load`, `-o`, `-MF`); it is checked
  at every source of flags and again when `harness.toml` is loaded. Every child's working folder is
  the harness's work folder (§3.7) and its `PATH` holds absolute entries outside the project only (a
  relative or empty entry would run the project's own `cc`). Where no sandbox exists (Linux today),
  `project map`, `ask` and `accept` refuse as every building command does unless
  `--allow-unsandboxed` is passed. A root that is the home folder, holds it, or holds the cargo or
  rustup home (a custom `CARGO_HOME` outside the home folder) is refused; a root holding several
  `.git` folders is warned about.
- **What a compile read is known and does not reach the map unnamed:** the compiler's own `-MD` list
  says what every compile read; a compile that read outside the root and the toolchain's folders has
  its symbol names counted, not stored; the error line is a closed reason (§3.1 step 5); every
  stored string from the project is a path or an identifier-shaped symbol name, stored raw and
  display-filtered and scrubbed of machine paths when shown; an object is read only by `objsyms`,
  bounded at 64 MiB, and deleted. The map stores no object bytes and no source text.
- **Symbols** are read by `objsyms`, never parsed from `nm`'s lines; only identifier-shaped names are
  stored; a program's `main` must be a function; verify's own `nm` uses move to `objsyms` (§3.7).
- **The ledger inside the download** (§3.7): a ledger made elsewhere is adopted once per computer,
  by the person, before any command reads it; cargo and rustc run from the harness's work folder with
  the toolchain pinned, so a project's `.cargo/config.toml` and `rust-toolchain.toml` are never read;
  a unit crate with anything beyond the harness's own files is refused before cargo runs; `accept`
  re-links and never trusts a stored reply; a reply or response file is bound to the map it
  answered (a staleness guard, not authentication).
- **Ids and names:** program and library ids are derived and validated (§3.3); the model's names are
  display-only and fenced; indexes in a reply are matched by exact membership in the map.
- **The review gate** cannot be forged by a file name: newlines and tabs are filtered in every printed
  project string; `--json` escapes everything unsafe.
- **Bounds** (§3.10): a size cap per file and per object, a total time budget, a per-compile timeout,
  objects deleted as read, the walk's caps; the compile's memory is not limited by `setrlimit` (every
  crate forbids unsafe code; a limit would need the launcher pattern perf uses), said honestly.
- The project's own build is never run; nothing the map links is ever run.

### 3.10 Limits and bounds

20 000 walked `.c`/`.h` files (the count-only pass counts set-aside and skipped files separately),
depth 32, 200 000 distinct symbol names, a file over 8 MiB not read, an object over 64 MiB not read,
120 s per compile and link, a total budget of 30 minutes for `project map`. Past any cap: the file
facts are written, no closures are computed, exit 1 names the limit. A cut-short walk, like an
unreadable folder, must never produce complete closures, because definers never reached would read
as outside symbols.

### 3.11 An interactive picture of the map (the person's wish, 2026-10-02; last, as agreed)

The person wants to **see** the map, not read it: an interactive architecture diagram of the project
— files as nodes, calls and includes as paths, clustered around the programs the closures found, with
shared libraries sitting between the programs they serve. The cockpit (a TUI) is the right place for
walking a migration step by step, but probably the wrong medium for a graph of hundreds of files;
this is a different kind of view and may be a **throwaway export** rather than a cockpit screen.

Direction to investigate, not decided:

- **Source of truth: the map file.** The picture is rendered *from* `project-map.json` (and, once a
  program is a tool, from the facts and plan), never from a separate analysis — so it shows exactly
  what the harness believes, and a stale picture is visibly stale (the map's `root_hash` and
  `inputs_hash`).
- **Form:** most likely `harness project export --html` writing a single self-contained HTML file
  (one page, no network, the graph data embedded) that opens in a browser: zoom, pan, click a node
  for its facts, collapse a program into one node, colour by state (C / Rust in use / set aside /
  did not compile / duplicate). Candidates for the drawing: a force-directed or clustered layout;
  the libraries used must pass the dependency due-diligence rule (vendored or embedded, pinned, no
  downloads at run time). A plain-text fallback (Graphviz `.dot`) is cheap and worth having
  regardless.
- **Its security rules, fixed now (decision 7):** the data is embedded as JSON in which the three
  characters less-than, greater-than and ampersand are written as their backslash-u escapes
  (`<`, `>`, `&`), because HTML escaping does not apply inside a `<script>` block and
  an end-script tag inside a string would end it; names are inserted with `textContent`, never
  `innerHTML`; a Content-Security-Policy meta (`default-src 'none'`, script and style by hash) makes
  the browser enforce "no network"; the file is written with an exclusive create into
  `migration/map/`, never where a shipped file could be opened in its place.
- **What it shows, in layers:** (1) the programs and what they share — the "which programs are in
  here?" picture; (2) inside a tool, the units of the plan and their order (leaf first) — the
  migration picture, where each unit's state is today's cockpit state (planned, tried, migrated,
  failing); (3) inside a unit, the functions and their calls. Features' map (which functions each
  feature runs) and perf's rows (which units got slower) are natural overlays later.
- **Interaction limits:** read-only. The picture never changes a tool, a target or the plan; an
  action chosen in it (accept this program, migrate this unit) would hand off to the cockpit or the
  CLI, if ever.
- **Size:** a project of thousands of files needs collapsing by folder and by program at first view,
  with expansion on click; the map's caps bound it.

**Decided direction (2026-10-03, with the person):** the picture is **read-only, and the data flows
one way** — the scan and the map feed the picture; the picture never writes a tool, a plan, a target
or a test. Two places that can change state would have to agree and would each need the cockpit's
confirm-and-show-the-command safety; not worth it before anyone has used the picture. The first and
only interaction beyond looking is **handing off**: click a program or a unit and get the exact
`harness` command (or the cockpit opened on it); the write still happens in the cockpit or the CLI.
Reference for the *concept*, not the implementation: [emerge](https://github.com/glato/emerge) —
scan a codebase, build a dependency graph, render an interactive HTML page with force-directed
layout, clustering and metrics. Not adopted: it is Python with its own parsers, which would be a
second opinion on the code beside the harness's scanner; the rule here is one source of truth (the
map). Order unchanged: the map first (its link closures are what make clusters mean anything — on a
raw download every file is one blob), then a throwaway HTML export of the map on zopfli (about a day)
to see whether the picture earns its place, then a design of its own if it does. A picture is also
the quickest way to *check* a map: a wrong closure is obvious drawn and invisible in JSON.

Open for later: whether this is a `harness project export` (throwaway file), a cockpit act that opens
the browser on that file, a page in the cockpit's own MCP-served help, or all three; and which of the
three layers matters most to see first.

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

Open for later: which of the two comes first (the order is cheaper and builds on the plan; the seam
is more valuable and needs Part 2 of the roadmap note).

Where a model stays out: deciding what is *correct* (the oracle, the link check, the differential
runs — deterministic, always), and anything that writes state unchecked.

**The principle under all of it (the person's, 2026-10-03): know what the project does before
anything changes, and judge every change as a match or a mismatch against that.** The word is
**match**, not "right": the baseline is the untouched project's behaviour as it is, bugs and all —
built by the harness from the closure under **one named configuration** (§3.2), which the map
records; a program built under another configuration reads `program` beside its verdicts (the
digest moved), and perf refuses to compare rows across it. The project may hold known or unknown
vulnerabilities and room for improvement; the harness does not know and must not pretend to.
Matching is the first step — it is what makes moving code over safe, one bug-for-bug-compatible piece
at a time. Improving is a separate, later step: any change a model proposes as an improvement is, by
definition, a deliberate *mismatch*, and must be labelled as one, judged against a new expected
behaviour the person wrote down, and accepted knowingly — never slipped in with a translation that is
supposed to match. The harness already records expected behaviour at two levels — the driver (every
function's outputs on fixed inputs) and the features (what a person-visible feature materialises as:
exit status, stdout, stderr on fixed runs). Make that the organising rule as the model's role grows:
every proposal — a program, an order, a seam, a translation — is judged against behaviour recorded
*before* the change, so a wrong change is known wrong with high probability, and a matching one known
to match, rather than trusted. The picture (§3.11) shows the proposal and its reasons; the recorded
behaviour judges the result; **a subject-matter expert signs off**, and nothing is accepted without
that. The gap to close for the map: once a program is accepted, record its expected behaviour
(features for the tool, under its configuration) *before* its first unit moves. Today nothing
enforces it: the cockpit's tree shows "Features (none yet)" for a tool without features, and units
verified before features exist read `not-yet` once they do; §5 puts the step in the order, and the
person decided (§7) that it is shown, not enforced.

## 4. Tests and checks

The walk and the map (fixtures are small made-up projects in the test suite, shaped like the
downloads; the real liblzg and lz4 runs are manual checks in the testing guide, since the downloads
are not in the repository):
- The 100 benchmark cases: zero programs each, one library each, the same `include_dirs` `bench
  init` writes today.
- zopfli: one `main` program, 13 files, `-lm`, zopflipng's C++ counted as set aside; its own
  `migration/` is pruned (the harness's, created here); the map's closure and include folders match
  the committed `source_dir`/`include_dirs`; the file-list target written from it gives `facts.jsonl`
  and every `source_hash` byte-identical to the `source_dir` form, and `verify u001-katajainen` stays
  green; the committed program digest of the `source_dir` form is unchanged (`d191be5c…`); all 101
  committed `facts.jsonl` and zopfli's `findings.jsonl` byte-identical under the new include
  resolution; `bench check --replay` unchanged.
- liblzg's shape: a program including `<proj.h>` from a sibling `include/` folder; two definers of
  the same two functions (`decode.c`, `mini.c`) in two programs' closures, both choices linking, one
  needing a third file: both programs held, `accept --keep` settles them, the closure recomputed
  (the third file stays only with the one definer), the unchosen file "alternative not kept", not
  shared, not a library; a positive `compile_commands.json` whose `-D` makes a file compile
  (`-DHAVE_CONFIG_H`).
- A header named with a folder part (`<proj/api.h>` at `include/proj/api.h`) found; a project
  `unistd.h` beside the system's and two folders each holding `config.h`: ambiguous-include facts,
  the system winning, no silent pick; a header that includes another in a third folder.
- lz4's shape: a fuzz driver of many (kind `driver`, ten `fuzz` programs, never asked "which to
  keep", never offered); a stated configuration whose `-D` adds a file and its pthread needs; without
  it every closure marked guessed; the same project mapped with and without it gives different
  closures, each recorded with its configuration; `ask` refused while guessed, `ask --build` allowed
  only then; a `compile_commands.json` whose entries differ per file gives a flags-differ fact and
  stays guessed; between-program duplicates listed and never asked; the tool's closure smaller than
  the build's link list, said.
- A made-up project with three `main()`s, a duplicate and a test: the closures, the kind guesses
  labelled, the shared file named after the choice; a collision of a symbol nothing needed inside one
  closure; a definer already in the closure a collision, not a question; a weak/strong pair not a
  duplicate; a weak need recorded as a need on Mach-O; common symbols merged; the `objsyms`
  extension on Mach-O and on ELF objects made here with `cc -target x86_64-unknown-linux-gnu -c`
  (no Linux machine needed), the same names without the underscore; an `asm`-label symbol and a data
  symbol named `main` (no program); an identifier-shaped file content included into a declaration
  never stored (counted, `outside_includes`).
- A file that does not compile, a file over 8 MiB, an object over 64 MiB, an unreadable folder: the
  closures that need them incomplete, their symbols never outside symbols, `accept` refusing; a
  program file that does not compile listed as such, its files not offered as a library; a non-UTF-8
  `.c` compiled but not parsed.
- The compile flags pinned and the outside symbols tied to them; a map made under another toolchain
  identity flagged; the system include folders recorded.
- A cut-short walk: file facts, no closures, exit 1 by name; a dot-folder listed as skipped with its
  count; an in-tree symlinked `.c` recorded once under its real path with the alias; a `.c` included
  by another `.c` and by a `.h`.
- The map run twice gives the same bytes; `root_hash` changes with a header and not with a README;
  `inputs_hash` changes with the configuration and with the toolchain; flags keep their order.
- A path, a symbol or a file name with control characters, a newline, or the fence delimiter's
  shape never reaches the terminal, `--json` or a prompt unfiltered; `--json` escapes every unsafe
  character.

Security:
- `compile_commands.json` with `@file` (separate and joined), `-B`, `-fplugin`, `-fuse-ld=<path in
  the project>`, `-Xclang`, `-o`, `-MF`, `-include` of a file outside the root, an entry naming its
  own compiler, and a `directory` outside the root: each refused or ignored by name; the same flags
  from `config.toml` and from `harness.toml`.
- A project `.cargo/config.toml` with a `rustc-wrapper`, a `rust-toolchain.toml` with a `path`, a
  `.cargo/config.toml` in an ancestor of the work folder: never read (the wrapper's marker never
  appears); a `PATH` with an empty entry and a project `cc`: the project's `cc` never runs.
- A ledger made elsewhere refused by every command that opens one, `--adopt` recorded per computer,
  `target/` and `build/` folders deleted on adoption; a `build.rs` beside an exact harness manifest
  refused before cargo runs; a unit crate with an extra file refused.
- A pre-placed response file or reply file for another map's digests ignored; a hand-written response
  that fails the contract named with the way forward.
- `cc` compiles and links under the map profile; `--target ~` refused; no sandbox → refused.
- A compile error whose first line would quote a readable file's bytes: the stored reason is closed,
  the shown line scrubbed.

The model step (with recorded traces replayed in CI from a fixed map fixture, so the request bytes
and trace keys are the same on every platform, as §16.2 of the briefing requires):
- A forged reply that names an unasked index, skips one, adds a field, a kind outside the set, a
  name over 40 characters or a control character: refused in full; a reply for another map's
  digests: not read; a `--build` reply with a flag outside the grammar or a cite to a file not sent:
  refused in full.
- `undecided` going to the person; a set with several linking choices held, never settled by the
  model's pick; a set over the limit held; a program labelled `example` by the model still
  link-checked; the resume command carrying `--model`, `--build` and `--allow-guessed`.
- `accept` refused with an unresolved set, an incomplete closure, a guessed configuration, after
  the map changed, with a root `harness.toml`, and when the link fails; `accept` re-links before
  writing; `accept --keep` works with no reply.
- The folder-name guesses labelled as guesses; `ask` refused with no open questions.

The rest of the harness:
- The configuration's `-D` that changes a struct layout in a unit's header is seen by the driver
  build, the boundary check and the features map; a unit whose files have different include folders
  compiles each with its own.
- The v2 digest covers the configuration; the v1 digest is byte-identical to before; a changed
  `root_hash` is a notice, not staleness; a scan clears staleness.
- A `schema_version = 3` file reads "schema too new"; a file with both forms refused; `--tool` on
  every subcommand, with the lookup order; two tools' `sync-runtime` blocks side by side.
- harness-detect's walk confined; the scanner and detect prune `migration/` when `source_dir = "."`.
- verify's driver-shape check reads symbols with `objsyms` and refuses an `asm`-forged name.

## 5. Order of work

Each step committed green with its tests from §4; then the review, fix passes checked, mutation
checks, DECISIONS. The map's code lives in harness-oracle (`objsyms`, the sandboxed `Runner` and the
profiles are there); the commands in harness-cli; the cockpit's acts in harness-tui.

(a) **The walk and symbols over one folder**, replacing nothing: the `objsyms` extension (kinds,
weakness, commons; ELF and Mach-O tests); the map sandbox profile and the targetless runner (bench.rs
already builds one); the harness's work folder and the children's `PATH` rule; the refusals (no
sandbox, a home root, a ledger made elsewhere, the project's own `migration/`) and the per-computer
adoption file, applied to every command that opens a ledger (the fixtures adopted by the tests'
set-up and `bench init`); the per-file facts and include folders; `project map` on zopfli and on the
benchmark printing the closure and the include folders that match today's `source_dir`/
`include_dirs`.

(c) **The layout and the file-list form**, tested by a hand-written file-list target over a liblzg-
shaped fixture before any map writes one: `TargetContext`'s ledger folder and `--tool` on every
subcommand with the lookup order; `harness.toml` v2 read version-first with the grammar at load and
the configuration record; `Base`'s configuration and per-file folders in every compile, the
multi-source builds on perf's compile-then-link path, the driver's folders; the scanner's file-list
read (both include forms; walk errors and non-UTF-8 as facts; `migration/` pruned; detect's walk
replaced); the v2 digest and the notice; confinement in every reader; cargo and rustc from the work
folder with the toolchain pinned; the unit-crate file check before cargo; verify's `nm` uses moved to
`objsyms`; `sync-runtime` per tool.

(b) **The whole-root walk and the map**: the configuration file and `--configuration`, the flag
grammar for every source, `compile_commands.json` as a proposal, the programs and their kinds, the
closures with duplicates, collisions, incomplete closures and libraries, the link checks of §3.5 in
`map`, the indexes, the map file, the caps.

(d) **The model step**: `ask` and `ask --build` with their reply contracts, the traces and the
resume command, the replay fixtures.

(e) **`project accept`** and the "what changed" report of a later `map`; the cockpit's three acts
(Map the project, Ask, Accept a program) as dialogs over the same commands, and its tool chooser.

(f) SCHEMAS (the map, `harness.toml` v2, the writer rows, the fence on the reply), the tutorial, the
testing guide.

(g) A process step, not code: after the first accepted tool, record its features before its first
unit moves (the cockpit shows "Features (none yet)" until then).

## 6. Residuals and what this does not do

- **Projects that need `./configure` or CMake to generate headers** are mapped as far as they
  compile; the map says which files failed on a missing header and names it, and where a stray header
  of that name exists in the tree the ambiguous-include fact says the compile may have used the wrong
  one. Running the project's own build is out of scope (it executes untrusted code).
- **One configuration per accepted tool.** `#ifdef` code whose meaning depends on flags the
  configuration does not state is compiled as the configuration says; a file the project builds
  twice with different flags appears once, under the tool's configuration. Revisit when a tool's own
  files need different flags from each other.
- **A project with its own `migration/` folder** cannot be mapped in place; the person moves or
  renames it, or maps a copy. Revisit if it is common.
- **A shared library is migrated once per tool** that uses it: two targets, two copies of the unit's
  Rust and two verdicts. One Rust library per program, and non-leaf units, are Part 2 of the roadmap
  note.
- **Outside libraries** are guessed from symbols; a wrong guess fails the link and is said. When a
  link fails on outside symbols and the project holds set-aside assembly or prebuilt files, the
  failure says "this project has set-aside files in X"; the map cannot read what they define.
- **Static functions** are invisible to the closure (they never cross files, except by textual
  inclusion, which step 10 records); the scanner's per-file list still records them.
- **Files the project's build links for a constructor or a linker section**, reached by no symbol,
  are unreached to the map; the project's binary runs them and the harness's rebuild does not. Not
  seen on lz4 or liblzg; the map cannot see it.
- **A map describes the platform it was made on**: the walk and the checks run wherever `cc` exists
  and a sandbox does (macOS today; Linux once its sandbox exists); needed and outside symbols come
  from the host's headers and `#if defined(__APPLE__)` branches, and macOS links `libm` and pthreads
  without a flag — a map is marked with its toolchain and does not promise to link elsewhere.
- **A forged ledger, once adopted,** is trusted as a clone of the person's own repository is today;
  adoption deletes its build folders and `verify` re-runs its claims, but a verdict file is a file.
  The gate is the person's one-time, per-computer "adopt".
- **The compile's memory** is bounded by the sandbox's write roots, the object cap and the time
  budget, not by a resource limit.

## 7. Decisions made (the person)

2026-10-07: (1) the per-file compiles run by default (2.2 s on lz4's 48 files); (2) `map` always
stops and shows; the person accepts which programs become tools — the same review gate as the plan;
(3) a mapped tool's target and ledger live inside the project (`<project>/migration/…`); RuHarness's
`targets/` stays for its own fixtures; (4) a library with no program stays a target on its own (the
benchmark's shape); (5) the build's flags are the person's to state when there is no
`compile_commands.json`; a model may propose them, the person confirms; the baseline is one named
configuration; (6) the picture (§3.11) is read-only with one-way data flow and comes after the map;
the advice (§3.12) after that; an improvements mode (intended mismatches) later still, the vocabulary
settled.

2026-10-08, on revision 1's proposals — **all as recommended, under one rule: simplicity means
usability** (easy and intuitive, starting at the command line, and runnable from the cockpit too):
(7) the layout of §3.7 — the project root the target root, mapped tools under `migration/tools/<id>/`
(revision 2 drops revision 1's "first tool at `migration/`": it nested every later tool inside the
first tool's ledger and clashed with a root `harness.toml`); (8) a ledger made elsewhere is refused
until the person adopts it, and cargo and rustup run from outside the project with the toolchain
pinned (revision 2 keeps the adoption record per computer, outside the project, because a record
inside it could be shipped with the ledger); (9) the person states the build in
`migration/map/config.toml`, a model may propose it with `ask --build`, and `ask`'s other questions
wait for a stated configuration; (10) the commands are `harness project map | ask | accept`, the
model at Tier 2 by default; (11) the command line first, the cockpit running the same three acts as
dialogs, the map *screen* a design of its own; (12) a duplicate both of whose choices build is the
person's to resolve at `accept`, the model's pick advice; (13) the picture's security rules fixed now;
and, from the check of revision 1, the harness answers kinds and small duplicate sets itself (Tier 0)
and asks a model only what linking leaves open; a mapped tool's missing features are shown, not
enforced.

## 8. Open for later

- The picture: a throwaway export, a cockpit act, a help page, or all three; which layer first.
- The advice: the migration order first, or the seam.
- A project with its own `migration/` folder, if it turns out common.
- Linux: the map once a Linux sandbox exists; the `objsyms` extension is written for ELF too.

## 9. What changed from revision 1, and why

Revision 1 answered every confirmed finding of the four-lens review on paper; its check (five
readers) found that several answers did not fit the code or each other. This revision:

- **Settles the layout for good**: every mapped tool under `migration/tools/<id>/`, the lookup order
  for `--target` and `--tool`, `--tool` on every command, a project-level lock, one `sync-runtime`
  block per tool, the full `.gitignore` written by the first `map`; and gives up mapping a project
  that has its own `migration/` folder (a plain refusal, §6) instead of inventing a second ledger
  name. (The check's integration and coherence readers.)
- **Keeps the adoption record outside the project**, per computer, checked by every command that
  opens a ledger, because a record inside the project could be shipped with a forged ledger and only
  `map` had checked it; adoption deletes build folders and says the verdicts are claims. Cold resume
  is unaffected: the ledger still holds everything; only the one-time trust question is per
  computer. (Security.)
- **Names the `-f` flags, forbids `-fuse-ld`**, ignores an entry's own compiler and `output`, says
  which flags reach a link, records `-O` without applying it, and fixes the work folder, the
  children's `PATH`, the map profile's real read surface, the object cap and the unit-crate file
  check before cargo. (Security.)
- **Makes the configuration per tool in fact**: several named entries in `config.toml`,
  `--configuration` on `map`, the run name at `accept`, `compile_commands.json` a proposal until
  named, flags-differ facts, `ask --build` allowed only while guessed, `accept` never guessed; "per
  scope" gone. (Coherence, the model step, integration.)
- **Fixes the include rule**: candidates matched by whole path parts, the system's search folders
  read once and recorded, the system winning over a project header unless the configuration says
  otherwise, headers scanned too and given a status. (Facts.)
- **Says what `objsyms` must learn** (kinds, weakness, commons; external only), writes the weak and
  common rules into the closure step, and the definer-already-in-the-closure rule. (Facts.)
- **Treats a file that did not compile, was too large or unreadable** like a cut-short walk: the
  closure incomplete, its symbols never outside, a program that did not compile listed, `accept`
  refusing; libraries defined so pending alternatives and programs are never one. (Facts,
  coherence.)
- **Settles by linking first**, in `map`, and asks a model only what linking leaves open; the kind is
  a label the harness guesses; indexes are the harness's and exist without a model; the reply
  contracts, caps, refusal words and the way forward are written; the replay fixture is
  platform-independent; the binding is said to guard staleness, not forgery. (The model step,
  coherence.)
- **Makes the multi-source compiles per file** (perf's path), says the driver's folders, keeps
  today's staleness rule and makes `root_hash` a notice, moves verify's `nm` uses to `objsyms`.
  (Integration.)
- **Corrects the wrong examples and claims**: `lib/lz4file.c` is reached by an example and a test;
  the spike's script did less than steps 5–9; the cockpit shows "Features (none yet)", not
  `not-yet`; a changed configuration marks `program`, it does not refuse; the export's escapes
  written out; `--root` → `--target`; the exit words; no commit hash. (Facts, the model step,
  security, coherence.)
- **Restores what draft 0 had and revision 1 lost**: the open questions on the picture and the
  advice (§8), the greyed "not part of this tool", a positive `compile_commands.json` test, a
  non-linking program refused while the reply stands, SCHEMAS' writer rows and the fence on the
  reply. (Coherence.)
- **Reorders §5** so each step has what it needs, assigns the unplaced security work, and fixes (a)'s
  acceptance test. (Coherence, integration.)
