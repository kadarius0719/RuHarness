# Real project layouts: the project map (design, revision 2.2)

Status: **revision 2.2 — 2026-10-08; the person's decisions taken (§7); revision 2 re-checked by three
readers and corrected in place as 2.1, then read once more and corrected as 2.2
(docs/reviews/2026-10-08-project-map-rev2-check.md holds both rounds); ready to build in §5's order.** Draft 0 (2026-10-02) was reviewed from four lenses with every
finding verified (docs/reviews/2026-10-07-project-map-design-review.md, the triage at its end);
revision 1 answered that review and was checked by five readers (docs/reviews/2026-10-08-project-
map-rev1-check.md: four lenses against the code and the spike, one for the document as a whole);
revision 2 answered that check, and revision 2.1 answers its re-check. §9 says what changed and why, in plain
words. Part 2 of the roadmap
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

1. **Files.** Walk the whole root with harness-core's confined walk, which learns four things for
   the map (§5 step a; today it prunes exact paths only, drops links that leave the root without a
   record, never joins two paths to one file, and keeps whichever of two folder paths sorts first):
   pruning **by name** (`.git` and every folder whose name starts with a dot, at any depth, listed in
   `skipped_folders` with a file count from a count-only pass) and **by canonical path** (all of
   `<project>/migration/`, §3.7), so a link into either is a `walk_issue`, never walked; a link to
   a folder inside the root is not descended — the real folder is walked and the link's path is
   recorded as an alias of each file; a link to a file inside the root makes that file's second path
   an alias, the file recorded **once** under the path with no link in it; a link that leaves the
   root or points nowhere, a FIFO or a device, and a folder the walk cannot read are listed in
   `walk_issues: [{path, why}]`. Record every `.c` and `.h`: path, size, blake3 (hashed by streaming;
   a file over the 8 MiB cap is hashed over its size and its first 8 MiB — blake3 of the size in
   decimal, a newline, then the head — so a sparse file claiming a terabyte costs no more than any
   other, and a change past its head that keeps the size is not seen: it is neither parsed nor
   compiled anyway). Non-C sources (`.cc`, `.cpp`, `.cxx`, `.m`, `.go`,
   `.rs`, `.py`, `.js`, `.lua`, `.pas`), assembly (`.s`, `.S`, `.asm`) and prebuilt files (`.a`,
   `.o`, `.so`, `.dylib`) appear only as **counts per folder** in `set_aside`, never as entries and
   never read. Caps (§3.10): 20 000 walked `.c`/`.h` files, depth 32, a file over 8 MiB is recorded
   `too_large` and neither parsed nor compiled. A walk error is a fact of that path, never a stop. A file that is not UTF-8 is parsed from its
   bytes like any other (the parser takes bytes; only the scanner's text read needs UTF-8), compiled
   like any other, and recorded `not_utf8` (Latin-1 comments are common in older C).
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
   candidate). A file gets only the folders its includes need. A quoted include found in the including file's own folder is that file — the compiler takes it
   first whatever the flags (zopfli's `"util.h"` beside the SDK's `util.h`; lz4's `programs/util.h`)
   — and is never ambiguous. An include is **ambiguous** only when the map would add a folder to
   reach it and either another project folder or the system also holds the name (two `config.h`; a
   `<unistd.h>` with a project `compat/unistd.h`): the walk records an **ambiguous include** fact
   listing every candidate and picks none. The compile in step 5 passes the folders the file needs
   with `-I`, and `-I` folders are searched before the system's, so a project header in such a
   folder shadows the system's: the header the compile actually used is read from its dependency
   list (step 5) and recorded in the fact (`used: path | system`). The configuration settles it — an
   `-I`, `-iquote` or `-isystem` for the folder meant, or `system_headers = ["unistd.h"]` in
   `config.toml` for a name the project means the system's, whose folders are then passed with
   `-idirafter` so the system's header is found first — and `accept` refuses while a file of the
   closure has an ambiguous include the configuration does not settle (§3.6).
4. **Build evidence.** `compile_commands.json` at the root or one level down is read, never executed:
   for each file it names, its `command` is split by POSIX shell word rules with no expansion (or
   its `arguments` taken as they are), a separate-form option (`-I`, `-D`, `-U`, `-include`,
   `-imacros`, `-iquote`, `-isystem`, `-idirafter`, `-F`, `-L`, `-o`, `-MF`, `-MT`, `-MQ`, `-x`,
   `-arch`, `-isysroot`, `-target`, `-Xclang`, `-Xpreprocessor`, `-mllvm`) takes its next argument
   and is checked joined, a refused option drops its value with it, relative paths resolve
   against the entry's `directory`, and the flags that pass the grammar of §3.2 are kept with the
   rest counted as ignored; the entry's own compiler (`arguments[0]`, or the first word of `command`) and its
   `output` are never used — `cc` from the allowlist always compiles; an entry whose `directory`
   resolves outside the root is ignored and counted; a file it names that the walk did not find is
   recorded and never compiled; a file it lists twice with different flags is a **flags differ**
   fact. Build files found by fixed names (`Makefile`, `GNUmakefile`, `*.mk`, `CMakeLists.txt`,
   `configure`, `configure.ac`, `meson.build`) are recorded by path in `build_files`; nothing is run.
5. **Symbols.** Compile each `.c` to an object under the map's sandbox profile (§3.9), with the
   working folder in the harness's own work folder (§3.9), into a fresh temporary folder, each object
   named by the file's index: `cc -c -w` with the judge's own base flags — today `-O2
   -ffp-contract=off` — then the flags of the configuration (§3.2; when `compile_commands.json` is
   the source, the file's own entry's flags), then the file's include folders, each as `-I<abs>`,
   then the file's absolute path (every path starts with `/`, so no separator is needed and none is
   passed); plus `-MD -MF <temp>/<index>.d`, so the compiler itself lists every file the compile
   read (macro-built includes and `#embed` included; every listed path is canonicalized before it is
   compared with the root; a listed file inside the root that the walk did not record — a `.inc`, a
   `.def` — is recorded as `included_other` and counted in `root_hash`; the `.d` file is deleted with
   the object, it holds machine paths). The list does not see inline assembly's `.include` and
   `.incbin`; the map profile (§3.9), not the list, confines those. A compile that read anything
   outside the project root and the toolchain's own folders is `outside_includes: true`; its symbol
   names are **counted, not stored** (a readable file's bytes can become a symbol name), and the
   screen says so. Read each object with harness-oracle's
   `objsyms` (safe Rust, no `nm`, every offset bounds-checked), **extended for the map** (§5 step a):
   each **external** symbol it defines with a kind — `function`, `data`, `read-only`, `bss`,
   `common` (a Mach-O common symbol — undefined, external, with a size — is a definition, as is an
   ELF `SHN_COMMON` symbol; a Mach-O `__DATA,__common` section symbol under `-fno-common` is a
   strong `bss`) — and a `weak` flag (Mach-O `N_WEAK_DEF`, read on defined symbols only; ELF
   `STB_WEAK`); each external symbol it needs, with `weak` for a weak reference (`N_WEAK_REF`;
   ELF weak undefined) — a weak need still needs a definer at link time on macOS and resolves to
   null on ELF, and is recorded as a need with its flag. Local symbols are never read. One leading
   `_` is stripped on Mach-O so names equal the scanner's. A name not shaped like a C identifier
   (an `asm` label can hold any text) is counted as an odd name and never stored; a libc name with
   one `$` suffix (`realpath$DARWIN_EXTSN`, `opendir$INODE64`) is stored by its base name. An object over
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
   (§3.3) and a **kind guess** from the first folder of its relative path, compared ignoring case:
   `tests`, `test` → test; `examples` → example; `bench`, `benchmarks` → benchmark; `fuzz`,
   `ossfuzz`, `fuzzers` → test; anything else → tool. A kind is a **label**: it changes no closure,
   no check and no acceptance. A `driver` has no closure of its own.
7. **Closures.** For each `main` and `fuzz` program, from its file: add every file that is the
   **single** definer (among non-program files) of a symbol the set needs, until nothing new is
   added. A program file is never pulled into another program's closure: a need met only by another
   program's file is recorded as **"needs a symbol from program X"**, not as an outside symbol. A
   needed symbol with two or more definers among non-program files is a **duplicate** of this
   closure: its definers are recorded as *alternatives, pending*, none of them added. A need already
   met by a file in the closure is met: no duplicate, no question, nothing added; a pending set one
   of whose definers joins the closure later (for another symbol) is met the same way. Rules of the
   linker apply: a weak definition beside a strong one, a common definition beside a strong one, two
   weak ones, or several common definitions of one name are neither a duplicate nor a collision (the
   strong one, else the first in path order, defines it). After a choice (§3.5) the closure is **recomputed from
   scratch** with the chosen file (a file the unchosen alternative had pulled in must not stay). Over
   the finished closure, every symbol defined by two or more of its files is a collision, reported
   even when nothing needed it. What stays unresolved is the closure's **outside symbols** (libc,
   libm, an outside library).
   A closure is **incomplete** when a duplicate is pending; when a file that was parsed but did not
   compile has parser facts that define one of its outside symbols ("may be defined in X, which did
   not compile"); when the closure has any outside symbol and some `.c` was neither parsed nor
   compiled (too large, unreadable: "X could not be read; it may define …"); or when the walk could
   not read a folder. A file
   whose parser facts define `main` but which did not compile is listed as **a program that did not
   compile**; it has no closure, and files reached only through it are not offered as a library.
   Under a guessed configuration (§3.2) failed compiles are expected and said so.
8. **Shared files, after the choices.** A file in two or more closures is shared by those programs.
   Duplicates between programs that never meet in one closure (lz4's 14 helper names across
   `examples/` and `tests/`) are listed, not questions.
9. **Libraries.** A `.c` that is no program, no alternative (pending or not kept) and in no closure
   is unreached.
   Two unreached files join when one needs a symbol the other alone defines; the groups so formed
   (a lone file is a group) are **libraries**, each listing the files outside it that it needs. That is
   the benchmark's shape (all 100 cases have zero programs and one library), and the shape of a
   library a project ships without a program. A library can be accepted as a target by itself
   (decision 4, §7).
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
  --configuration <name>` maps under one; without the flag, the only entry; with several entries
  and no flag, the command refuses and names them; a guess when there is none. An entry may also
  hold `system_headers = ["unistd.h"]`: names the project means the system's (§3.1 step 3). The **run name** (what features run the program as: lz4's tool is `lz4`, not `lz4cli`) is a
  program's, not a configuration's: `accept --run-name`, default the program file's stem.
- **Order of trust and the flow.** The first `project map` runs with no configuration (2.2 s on lz4)
  and shows the build files it found and the closures, every one marked `guessed`. A
  `compile_commands.json`, when present, is **a proposal like the model's**: the map uses it and
  shows it, but the source stays `guessed` until `config.toml` names it (`from =
  "compile_commands"`), because it is one build's output and the project's builds disagree. From
  such a file a program's configuration is the flags shared by every file of its closure; when they
  differ, or a file is listed twice with other flags, the map records a **flags differ** fact for
  that program and the source stays `guessed`. From such a file a program's shared flags are recorded on its closure (`closures[].flags`); a
  `config.toml` entry with `from = "compile_commands"` takes each program's shared flags from the
  file. The person writes `config.toml` — or asks the model (`project ask --build`, §3.4, allowed at
  any time, since it only proposes) and copies what they accept — and runs `map` again; the
  closures are now `stated`. A `config.toml` that came with the project — present when this
  computer first recorded the root, its hash kept in the adoption record outside the project — is
  shown as proposed and keeps the source `guessed` while it has that hash; `--adopt` once, or the
  person's own edit, states it (2026-10-08). Questions about duplicates and programs (`project ask` without
  `--build`) are refused while the configuration is a guess unless `--allow-guessed` is given,
  because a wrong flag changes the closures and the questions; `accept` never takes a guessed
  configuration.
- **The flag grammar, one for every source** (`compile_commands.json`, the model's proposal, the
  person's file, and `harness.toml` when a target is loaded — `harness.toml` is untrusted input):
  - `-D<name>` and `-D<name>=<value>` with the name a C identifier; `-U<name>`;
  - `-I<dir>`, `-iquote<dir>`, `-isystem<dir>`, `-include<file>` with the path inside the project root
    after resolving and not under `migration/`;
  - `-idirafter<dir>` under the same rule, so an accepted tool can carry `system_headers`: its folder
    is searched after the system's, so a project file there is taken only for a name the system
    lacks;
  - `-std=` with a value from a fixed set (`c89 c90 c99 c11 c17 c18 c23 c2x gnu89 gnu90 gnu99
    gnu11 gnu17 gnu18 gnu23 gnu2x`);
  - `-pthread`;
  - `-f` flags from an **exact list of spellings**, extended only in RuHarness's own code, never in a
    target file: `-fno-strict-aliasing`, `-fwrapv`, `-fno-common`, `-fcommon`, `-fPIC`, `-fpic`,
    `-fsigned-char`, `-funsigned-char`, `-fno-builtin`, `-fvisibility=hidden`,
    `-fvisibility=default`;
  - `-O0`–`-O3` are accepted and **recorded, never applied**: every compile keeps its own level (the
    map and the judge `-O2`; the boundary check and the driver's validation their own);
  - every path is absolute when passed; no value may start with `@` or `-` (clang reads an `@file` argument as a file of
    options, joined or not);
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
              included_by: [path], included_other: [path],
              defined_symbols: [{name, kind, weak?}], needed_symbols: [{name, weak?}],
              odd_names: n, withheld_names?: n}],
  programs: [{id, index?: "p1", path, kind: main | fuzz | driver,
              kind_guess: tool | test | example | benchmark, serves?: [path]}],
  programs_not_compiled: [path],
  closures: [{program: id, files: [path], flags?: [flag], flags_differ?: [{path, flags}],
              incomplete: bool, incomplete_why: [{why: pending | may-be-defined-in | unread |
              unreadable-folder, path?, symbols?}], outside: [sym],
              needs_from: [{sym, program: id}],
              duplicates: [{set: "d1", symbols: [sym], definers: [{index: "d1.1", path}],
                            links: [index], choice?: {keep: index, by: links},
                            under?: index}],
              collisions: [{sym, definers: [path]}],
              ambiguous_unsettled: [{header, used, candidates}],
              linked?: ok | {missing: [sym], doubled: [sym]}, questions: [index]}],
  between_program_duplicates: [{sym, definers: [path]}],
  shared:   [{file: path, programs: [id]}],
  libraries: [{id, files: [path], needs_from_outside: [path]}],
  set_aside: [{folder, lang, count}], skipped_folders: [{path, count}],
  walk_issues: [{path, why}],
  build_evidence: {compile_commands: present | absent, ignored_entries: n, unfound_entries: [path],
                   build_files: [path]},
  limits_hit: [{limit, at}] }
```

- `root_hash` is SCHEMAS' file-set hash (blake3 over the sorted paths and contents) of every walked
  `.c`/`.h`, every `included_other` file, plus `compile_commands.json` when present. `configuration.digest` is blake3 of the
  canonical JSON of `{name, from, flags (in order), system_headers}` — `system_headers` changes the
  compile (`-idirafter`) and what settles an ambiguous include, so it moves the digest; it is left
  out when empty, so a configuration without it keeps the digest it had. `toolchain` is `{cc: the first line of cc
  --version, target: cc -dumpmachine, cflags: the judge's base flags, system_include_dirs}`.
  `inputs_hash` is blake3 of the canonical JSON of `{configuration.digest, toolchain}` (a stale
  picture is visibly stale after a compiler change). No commit hash is recorded: reading one means
  running `git` in an untrusted tree.
- Flags and include folders keep their order; every other list is sorted by its first field's bytes,
  so two runs give byte-identical files (the golden test; SCHEMAS' canonical-serialization rule).
- **Ids** are made by the harness, never by a model: a program's id is `t-<stem>` where the stem is
  its file name without `.c`, lowercased, with every character outside `[a-z0-9_-]` replaced by `-`;
  when two collide (ignoring case) the whole relative folder joins in, its characters replaced the
  same way and its `/` turned into `-` (`t-<folder>-<stem>`), the part after `t-` cut to 60
  characters, then `-2`, `-3`… until unique; a program at the path
  of an accepted tool keeps that tool's id across maps. A library's id is `l-<stem of its first
  file in path order>`, made unique the same way; `--tool` is checked against
  `^[tl]-[a-z0-9_-]{1,64}$`. **Indexes** are assigned by `map` in path order and recorded in the
  map: every `main` program `p1…` (`programs[].index`); definer sets `d1…` numbered **once per
  project** — one index per distinct set of definer paths, ordered by their sorted path lists, the
  same index in every closure that holds the set, with that closure's own symbols; definers
  `d1.1…` in path order; a set reached only under some choice (§3.5) is numbered after the others
  and marked `under: <choice>`. They are printed with the map and used by `accept --keep` with or
  without a model's reply; because a new file can shift them, `accept` prints "keeping `<path>` over
  `<path>` for `<symbols>`" before it writes, and `--keep` also takes a definer's path as the value.
- Past a cap (§3.10) the map holds the file facts only, no closures, and the command exits 1 naming
  the limit. (A map whose closures are merely incomplete is a full map: exit 0.)

### 3.4 Asking a model (Tier 2, optional, the `external` hand-off as everywhere)

The harness settles what it can first (Tier 0, §3.5): `map` links every closure that has no pending
choice and every choice of each small duplicate set. What remains **open** is: (1) the
configuration, while it is a guess; (2) a duplicate set the link could not settle (several choices
link, or the set is over the limit); (3) a program's kind, only when the person asks. With nothing
open (zopfli, once `config.toml` states its build — it has no `compile_commands.json`), the map is
final without a model and the person still accepts (decision 2, §7).

- **`project ask --build`** is allowed at any time (it only proposes, so a second configuration can
  be asked for later) and asks nothing else: it sends the build files the walk found (`build_files`,
  by their fixed names, read with a size cap of 64 KiB each and 128 KiB in all, never through a
  link; over the total the largest files are left out first and named as not sent), each in its own
  block delimited by triage's deterministic nonce (a content hash, so replay works: the trusted part
  of the prompt holds only harness-written text, project text sits in nonce-delimited blocks with
  `<` escaped), carries the grammar of §3.2 and says that link and warning flags are not asked for,
  and asks for one configuration: `{"name": "…", "from": "make | meson | cmake", "flags":
  [{"flag": "…", "cites": ["path:line"]}], "assumptions": ["…"]}` — each flag through the grammar,
  each cited path one of the files sent, `name` 1–20 characters of `[a-z0-9-]`, each assumption one
  line of at most 200 characters shown labelled as the model's words. It writes
  `migration/map/config.proposed.toml` and changes nothing else; the person copies what they accept
  into `config.toml` and runs `map` again.
- **`project ask`** (the other questions) sends only the open questions, each by its index with
  that item's facts inside nonce-delimited blocks — for a program: its path, folder and parser facts
  (bytes, functions, includes); for a duplicate set: the union of its symbols, the ids of the
  programs that hold it, and for each definer its path, folder, parser facts and a bounded slice of
  each duplicated definition (at most 120 lines per definer, as triage's source slices; §3.3 keeps
  source text out of the map file, not out of a prompt) — programs first, then definer sets, in
  index order, at most 10 per call, one hand-off per call; a run with several batches writes every
  batch's request and exits with the first awaiting path, as triage does. The strict reply is a JSON array, one
  object per index: `{"item": "p3", "kind": "tool | test | example |
  benchmark | other", "name": "…", "purpose": "…"}` (`name` 1–40 printable characters, `purpose`
  one line of at most 200, control characters refused, both display-only) and `{"item": "d1",
  "keep": "d1.2" | "undecided", "reason": "platform | alternative-implementation | cannot-tell"}`;
  the validator refuses in full, naming the index and the rule, a reply that names an index not
  asked, skips or repeats one, adds a field (unknown fields are refused, unlike triage's reader) or
  an object, gives a `keep` that is not one of that set's own definers, or holds any character the
  display filter would hide (`unsafe_to_show`: controls, bidi and invisible format characters), as
  triage's "exactly the requested ids" rule does. A duplicate's answer applies to every closure that
  holds the same definer set (one index per set, §3.3) and is advice only: `ask` compiles and links
  nothing, and the screen shows, from the map's own link results, whether an advised definer is
  among the choices that linked. The model's `kind`, like its name and purpose, is a label.
  `ask --programs t-foo,t-bar` puts named programs' kinds to the model when the person wants a
  second opinion on a guess.
- **Provider, tier, traces, resume.** The provider and model come from `--provider`/`--model` (as
  `migrate` and `gen-driver` take them), Tier 2 by default (classifying programs and choosing
  definers is judgment, not summarization); escalation is the person's re-ask at a higher tier,
  because a wrong answer that links produces no failure to escalate on. Without a target
  there is no `[llm]` section to read: the defaults are the hand-off's own (`external`,
  `claude-sonnet-5`, 8192 tokens). `ask` exits 1 with the `awaiting` event and the exact resume
  command, which carries every flag given except `--adopt` and `--json` and always names the
  provider and model used (the model id is part of the trace key), as `observe` does; traces live
  under `migration/map/traces/`; record and replay work as today's hand-off (the `TraceAdapter`,
  `checked_complete`) with one new module like triage's (prompt assembly and a strict validator).
  Live: one retry with the error appended; external: a hard error naming the response file, with
  "delete it and answer again"; replay: "record a live run". **The reply file**
  `project-map.reply.json` holds one map's answers: `{schema, root_hash, inputs_hash, items:
  {<index>: {answer…, model, provider}}}`; `ask` replaces a file bound to other digests and merges
  under the same digests batch by batch as each validates; the latest answer for an index wins and
  the screen names the model that gave it. The reply is advice: `accept` never reads it, and refuses
  only when the map's own digests no longer match the tree. A **response file** in `traces/` is
  bound to its exact question (its key is a hash of the request), not to a map: a stored answer to
  the same question is reused, which is how replay works. **Neither is authentication** — a download
  can compute the digests and the keys — which is why the model's words are only ever labels and
  advice, re-linked at `accept`, and why a ledger made elsewhere is adopted before anything reads
  it (§3.7).

### 3.5 The checks (deterministic)

Run by `project map` (so the map's screen shows what the link proved), and again by `accept`.

1. **Every `main` program whose closure has no pending choice is link-checked**, whatever kind
   guess or model label it has: compile its closure and link it into one program with the outside
   libraries guessed from the outside symbols (`-lm` for `log`, `sin`…; `-lz` when `deflate`/
   `inflate` are needed and zlib is on the system; `-lpthread` for `pthread_*` where the platform
   needs it). The result is `linked: ok` or `{missing, doubled}`, computed from the `objsyms` facts
   and the linker's exit status, never from the linker's text (it can carry project text); a program
   that does not link is shown as such and the others, and any reply, stand. A `fuzz` program is
   linked only with the project's driver; a `driver` is never linked alone. Nothing built is ever
   run.
2. **Duplicates are settled by linking** when a set is small (at most 4 definers in a set, at most
   16 choices for the program): each choice is linked with the closure **recomputed for it**
   (liblzg's `unlzg` with `decode.c` needs `checksum.c` too; with `lzgmini.c` it does not). A choice whose recomputed
   closure raises a further pending set is expanded, each combination counting toward the 16, the
   new set numbered `under` that choice (§3.3). Exactly one choice links: recorded `choice: {keep,
   by: links}`, no question. None links: the program is recorded as not linking, with the missing
   symbols of the choice that left the fewest. Several link, or the set is over the limit: the
   program is **held** — "linking cannot tell d1.1 from d1.2" — and the set is a
   question for the model (advice) and the person (the decision, §3.6). A model's pick is never
   linked as if it settled anything; it is kept beside the set as advice.
3. **Every file's status is computed by the harness** from the walk and the choices, never from a
   reply: program, in a closure, pending alternative, alternative not kept, shared, library,
   unreached, set aside, did not compile, too large, could not be read. Nothing is silently dropped;
   a reply cannot change a status except through the person's choices.
4. **What the check cannot prove** (§1) is printed once per screen. A model's kind, name and
   purpose are shown labelled as the model's.

### 3.6 The person's gate: the screen, and accepting a program

`project map` and `project ask` **always stop and show** (decision 2, §7); nothing becomes a tool until
`project accept <id>`. The screen shows, per program: its kind guess (and the model's label, when
asked); the closure's files by folder, with "incomplete" and why; the outside symbols and the guessed
libraries; the configuration's name, flags and source; each duplicate set — settled by linking, or
held with both definers, the model's advice and reason when asked, and the indexes to use; what the
link proved and did not (once per screen); what the project's build may link beyond the closure; a
driver's served fuzzers and each fuzzer's link result. Every refusal and hedge is one plain sentence
with what to do next.

`accept <id> [--keep d1=d1.2 | --keep d1=<path>]… [--run-name NAME]` resolves held sets with the
person's picks (recorded `by: person`; it prints each pick in words before writing), and refuses — in
one sentence each — while any duplicate set of the closure is unresolved, while the closure is
incomplete, while a file of the closure has an ambiguous include the configuration does not settle
(§3.1 step 3), while the configuration is a guess, when the map's digests no longer match the tree,
when a root `harness.toml` exists (the project is already a folder-form target: move it, or map a
copy), and when the link fails; it **re-runs the link check** before writing anything, never reading
a reply. It then writes the tool's target (§3.7): a
`harness.toml` in the file-list form — the files, each with its include folders, the configuration
(its own copy), the guessed `extra_link_args`, the run name, the map's `root_hash` and
`inputs_hash`, and the picks (`picks = [{definers: [path], keep: path, by: "person"}]`) — so a later `map` can
report, for each accepted tool: closure changed, configuration changed, new programs, a tool that
no longer links. The acceptance is the written `harness.toml`, reviewed with `git diff` as the plan
is; no separate record. **Accepting an id again** (after the notice of §3.7) takes the project lock,
then that tool's ledger lock, in that order; rewrites only its `harness.toml`; keeps its ledger (the
plan, the units, the verdicts — a verdict made under another program digest reads `program` beside
it, as today). A shared file accepted
with two tools becomes units in each tool's target (two ledgers, two Rust copies; §6). A library is
accepted the same way: `accept l-<stem>` compiles its files (no link) and writes the target with no
program and its id as `name` (today `[target] name` is both the label and the program features
run; a library runs nothing).

### 3.7 Where a mapped tool lives, and what the rest of the harness must learn

**The layout.** The person decided (decision 3, §7) that a mapped tool's target and ledger live inside the
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
  paths derive from it (about 51 production `Ledger::new` sites and a dozen path helpers). The plan's `driver` paths stay root-relative. The writer lock stays per ledger;
  `project map`, `ask` and `accept` take a project-level lock, `migration/map/.lock` (and `map` on a
  folder-form project takes that target's ledger lock too, since `map/` sits inside it); `accept`
  of an existing id then takes the tool's ledger lock (§3.6). `sync-runtime` writes one generated
  block per tool, marked with its id, naming that tool's ledger paths, under the project lock, since
  the root `AGENTS.md` is shared. harness-mcp is started on a project with `--tool`, like the
  cockpit; its per-call arguments do not change. `state status` counts `migration/map/` as
  project-level; there is no `state gc` today.
- **A project's own `migration/` folder** (its database migrations, say) cannot hold the harness's
  files: `project map` refuses in one sentence — "this project has a migration/ folder of its own;
  move or rename it, or map a copy" — and §6 records the limit (revisit if it is common).

**A ledger made elsewhere is adopted before anything reads it.** A download can ship a ready-made
`migration/` — a `harness.toml`, green verdicts and `verified` statuses whose digests it computed, a
promoted crate with a build script, a `target/` folder, a reply file, response files for the
hand-off. Nothing inside the project can prove who made it, so **adoption is a per-computer trust
decision kept outside the project**: the file `$RUHARNESS_ADOPTED` when that variable is set, else
`~/Library/Application Support/ruharness/adopted.toml` (the same spelling on Linux), written
atomically under a lock, records the canonical roots whose ledgers the harness created on this
computer (any command that creates `migration/` — the first `scan`, `map`, `features init`) and the
roots the person adopted, each paired with a random token the harness also writes to
`<project>/migration/.ruharness-adopted` (one per project root; a benchmark suite's at
`<suite>/.ruharness-adopted`; git-ignored), so a different tree unpacked at the same path is not
trusted by its path alone; `--adopt` always writes a fresh token over any the download shipped
(so a later tree from the same source, unpacked at the same path, is asked again); only the test
helper and a benchmark suite's adoption (trusted by `corpus.lock`) record a token already there
(changed in the fix passes after step (c), 2026-10-08). The check is one harness-core function, called by `TargetContext::load` (so every
command that opens a ledger — the CLI, the cockpit's read model, harness-mcp — goes through it) and
by the three `project` commands, which run on a root with no `harness.toml`. It refuses a ledger
whose root is not listed or whose token is missing or different, in one sentence: "this folder already holds migration results made
elsewhere (N units, M verified): to trust them here, add `--adopt` once". The cockpit asks the same
in a dialog; harness-mcp never adopts (an agent is not the person: its refusal says "ask the person
to adopt it; an agent never adopts", and every `sync-runtime` block carries that line). A
`migration/` that holds no results — only `tools/<id>/harness.toml` files, `map/config.toml`, the
locks, the `.gitignore` and the token — needs no adoption and is recorded as created here; a
`migration/` that is not the harness's at all gets the "of its own" refusal before any adoption
question. A `migration/` is the harness's when it holds `facts.jsonl`,
`plan.toml`, `map/` or `tools/` and nothing outside the ledger's fixed names (one table in SCHEMAS:
`facts.jsonl`, `plan.toml`, `DECISIONS.md`, `observer/`, `units/`, `features/`, `perf/`, `map/`,
`tools/`, `build/`, `.lock`, `.gitignore`, `.ruharness-adopted`; Finder's `.DS_Store` is ignored);
otherwise it is the project's own and `--adopt` is refused for it (§3.7's plain refusal). Adopting records the root and
its token, deletes only `migration/build/`, each unit crate's `target/`, each attempt's
`candidate/target/` and every `.promote-*/` (links themselves, never their targets), and says that
the verdicts are claims made elsewhere until `verify` runs them here; adopting a root already
listed deletes nothing. On a new computer the person is asked once again; the ledger alone still
holds everything needed to resume (briefing §2.2), so cold resume is unaffected — only the one-time
trust question is per computer. A ledger made on this computer before this rule existed has no
token and is asked once, like any other. The test suite adopts through the same function, by a test helper
that points `$RUHARNESS_ADOPTED` at a temporary file once per test process and adopts the fixture it
opens (zopfli and the cases carry a committed `.ruharness-adopted`, listed in RuHarness's own
`.gitignore` rules for scratch files only where it is not committed); the e2e tests pass `--adopt`
on their first command, and so does the documented `bench check` line, once per worktree: `bench`
commands take `--adopt` like every other command and adopt the suite root as one root (its cases lie
under it; a `corpus.lock` that verifies says nothing about the cases' ledgers, and decision 8 wants
the person's word). SCHEMAS' writer table gains the adoption file (outside the target): any command
given `--adopt`, and the first command that creates a ledger.

**Every cargo and rustc child runs outside the project.** cargo reads `.cargo/config.toml` from its
working folder and every folder above it, and rustup reads `rust-toolchain.toml` the same way, so a
download could supply a `rustc-wrapper`, a `linker`, or a `path` to its own `cargo`. The oracle
therefore starts cargo and rustc with their working folder in the harness's own **work folder**
`~/Library/Caches/ruharness/work/` (`~/.cache/ruharness/work/` on Linux; made at each run, refused
if it is a link, never under `/tmp` or `$TMPDIR`; a read root of the toolchain profile with its
ancestors in the metadata-only exception; no profile may write it or its ancestors; macOS seldom
clears `~/Library/Caches`, and a missing work folder is simply made again), with `--manifest-path`
and every path absolute, the toolchain pinned by setting `RUSTUP_TOOLCHAIN` in the child's
environment to the harness's own (the value rustup gave the harness when there is one, else
`stable`, RuHarness's own `rust-toolchain.toml`'s channel) with `RUSTUP_AUTO_INSTALL=0`, a missing
toolchain refused in one sentence and never installed (the sandbox's standing exception that lets a child read the
ancestor `rust-toolchain.toml` goes), and a `PATH` of absolute entries outside the project root only. Before any
cargo run on a unit crate (verify, perf, features, the benchmark build) the crate folder may hold
only `Cargo.toml`, an optional `Cargo.lock`, `src/*.rs` and the harness-made `target/` (a real
folder); `.DS_Store` is ignored; a `build.rs`, a `.cargo/`, or any other file refuses the unit by
name; the harness's own manifest gains `build = false`, and a manifest that is not the harness's
(zopfli's hand-written `u001-katajainen`; the benchmark's crates, which carry an earlier harness
manifest) passes only when it holds no `build`, `links`, `[dependencies]`, `[dev-dependencies]`,
`[build-dependencies]`, `[patch]` or `[target.*]` key and its `[workspace]`, if any, is empty (today only `promote`'s
closed copy list keeps a `build.rs` out, and cargo runs one it finds beside an exact manifest).

**The configuration's flags and each file's include folders reach every compile.** A unit's C and
the headers it shares with the driver are compiled in about a dozen places today (the driver-shape
compile, the C and Rust driver builds, the driver's self-validation and its mutants, the boundary
check's wrapper, the features map's probed build, perf's objects), every one through the target's
single include list and with no `-D`. Under one configuration per tool the flags are target-wide:
`Base` carries the configuration's flags and a **per-file table of include folders**, and every
compile asks `Base` for a file's arguments — the configuration's flags first, then the file's own
folders. A compile that takes several C sources (the driver builds, self-validation, the mutant
link, the features map's plain program, the whole program) compiles each file to an object with its
own folders and links once, adapting perf's `compile_objects` and `link_side` (they exist, are
proven equal to the one-command build, and are perf-shaped — slots and hash checks — so they are
generalized, not copied); folder-form targets take the same path, with one include list for every
file, so there is one build path. Harness-written wrappers and probes (the driver, the boundary
wrapper, the features probes) take the folders of the unit they are written for. **The driver's folders** are, for each `.c` of the unit, its
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
from, flags}` and `name` (the run name) are required; `map = {root_hash, inputs_hash}` and
`picks = [{definers, keep, by}]` are optional (a hand-written target has none, and gets no "what
changed" notice); a file holding both `files` and `source_dir` is refused by name; the flag grammar
is checked at load. `[llm]`, `[oracle]` and the allowlist are written by `accept` as `bench init`
writes them today.

**The symbol readers.** verify's driver-shape check parses `nm`'s lines today over an object built
from the project's headers, where an `asm` label can forge a line; it moves to `objsyms` (its
`undefined()` exists) with the identifier filter. The capability and ABI-symbol checks read the Rust
staticlib — an `ar` archive, member by member (perf's `archive.rs` already reads BSD and GNU archives
into `objsyms`), with constructor sections, which `objsyms` does not report; they keep `nm` for now,
over the model's Rust rather than the project's headers (§6), and move once `objsyms` reports section
names. `nm` stays among the required tools until then.

**The plan:** no change in rule. Units and dependencies come from the facts over the tool's file
list; a call to a project file outside the list reads as an outside call (the capability check
already reads unresolved names). A shared file is a unit in each tool that holds it (§6).

**The cockpit and harness-mcp.** The command line comes first; the cockpit runs the same three acts
(the person's rule, §7). Opened on a project root: with a root `harness.toml` it is a folder-form
target as today; with mapped tools and no `--tool`, it opens the only tool, or lists several for the
person to pick (the command line does the same); with neither it offers **Map the project**, and after a map **Ask** and **Accept a program**, each a dialog
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
  when a map was written in full (its closures may still be incomplete: that is a fact in it);
  1 refused or cut short (no C found; a cap hit — the file facts
  are still written; a ledger made elsewhere not adopted; the project's own `migration/`; no sandbox;
  a root that is or holds the home folder); 2 usage. Output lines filter newlines and tabs in every
  project string (a file name can hold a newline; the review gate must not be forged); `--json`
  escapes every character `unsafe_to_show` names as `\uXXXX` (today's comment that serde does so is
  wrong; a small fix of its own, with SCHEMAS' sentence).
- `harness project ask --target DIR [--build | --programs IDS] [--provider P] [--model M]
  [--allow-guessed]` — §3.4; builds nothing and needs no sandbox; exits 1 with the `awaiting` event
  and the resume command; writes `config.proposed.toml` (with `--build`) or
  `project-map.reply.json`. Refused when there are no open questions; `--build` is never refused
  for that reason.
- `harness project accept <id> --target DIR [--keep d1=d1.2 | --keep d1=<path>]…
  [--run-name NAME] [--allow-unsandboxed]` — §3.6; exit 1 when refused.
- Every other subcommand, the cockpit and harness-mcp gain `--tool <id>` (§3.7); `--adopt` is
  accepted by every CLI command that opens a ledger and by the cockpit's dialog, never by harness-mcp
  (§3.7). `map`'s exit-1 list also holds a root that holds the cargo or rustup home (§3.9).
- Events (`--json`): `project-file`, `project-program {id, path, kind, kind_guess, files, outside,
  incomplete, held: [index]}`, `project-link {id, ok | missing: [sym] | doubled: [sym]}`; they carry
  paths and symbol names verbatim as the ledger's events do, never a model's names. SCHEMAS' writer
  table gains `project map`, `ask` and `accept` with their locks, and the adoption file.

### 3.9 Security

The threat model is the briefing's: the download is untrusted input, the model's reply is untrusted,
and now the ledger lives inside the download.

- **The map's compile and link run under a map sandbox profile of their own** — today's tool profile
  denies writes outside its folders and starts through the system (`NO_STARTS_THROUGH_THE_SYSTEM`:
  no LaunchServices, no Apple events, no launchd jobs) but allows reads of everything outside the
  home folder (so `/Users/Shared`, `/Volumes`, the temporary folders) and writes to all of
  `/private/tmp` and `/private/var/folders`. The map profile is its own renderer (today's always adds
  the temporary folders to the writable list): it denies reading `/Users` (the home folder and
  `/Users/Shared`), `/Volumes`, `/private/tmp` and `/private/var/tmp` except the project root and the map's fresh
  folder — verified to compile and link so; `/private/var/folders` stays readable and §6 says so —
  denies the cargo and rustup homes wherever they are (a custom `CARGO_HOME` outside the home folder
  holds credentials; `cc` needs neither), and allows writes only to the map's fresh temporary
  folder, with `TMPDIR` pointed at it. The flag grammar of §3.2 is what keeps the compiler from loading or running anything
  from the project (`@file`, `-B`, `-fplugin`, `-fuse-ld`, `-Xclang -load`, `-o`, `-MF`); it is checked
  at every source of flags and again when `harness.toml` is loaded. Every compiler, cargo and rustc child's working folder is the harness's work folder (§3.7;
  a built program keeps the working folder its run asks for, as today) and its `PATH` holds
  absolute entries outside the project only (a
  relative or empty entry would run the project's own `cc`). Where no sandbox exists (Linux today),
  `project map` and `accept` refuse as every building command does unless `--allow-unsandboxed` is
  passed (`ask` builds nothing and needs none). A root that is the home folder, holds it, or holds the cargo or
  rustup home (a custom `CARGO_HOME` outside the home folder) is refused; a root holding several
  `.git` folders is warned about.
- **What a compile read is known and does not reach the map unnamed:** the compiler's own `-MD` list
  says what every compile read through the preprocessor (not inline assembly's `.include` and
  `.incbin`, which only the profile confines); a compile that read outside the root and the
  toolchain's folders has its symbol names counted, not stored; the error line is a closed reason (§3.1 step 5); every
  stored string from the project is a path or an identifier-shaped symbol name, stored raw and
  display-filtered and scrubbed of machine paths when shown; an object is read only by `objsyms`,
  bounded at 64 MiB, and deleted. The map stores no object bytes and no source text.
- **Symbols** are read by `objsyms`, never parsed from `nm`'s lines; only identifier-shaped names are
  stored; a program's `main` must be a function; verify's driver-shape check moves to `objsyms` too,
  and its staticlib checks keep `nm` over the model's Rust until `objsyms` reads archives (§3.7).
- **The ledger inside the download** (§3.7): a ledger made elsewhere is adopted once per computer,
  by the person, before any command reads it (its root and token recorded outside the project);
  cargo and rustc run from the harness's work folder with the toolchain pinned, so a project's
  `.cargo/config.toml` and `rust-toolchain.toml` are never read; a unit crate with anything beyond
  its own sources and the harness's build folder, or a manifest that makes cargo run code, is
  refused before cargo runs; `accept` re-links and never reads a reply; a reply file is bound to the
  map's digests and a response file to its question (staleness guards, not authentication).
- **Ids and names:** program and library ids are derived and validated (§3.3); the model's names are
  display-only and fenced; indexes in a reply are matched by exact membership in the map.
- **The review gate** cannot be forged by a file name: newlines and tabs are filtered in every printed
  project string; `--json` escapes everything unsafe.
- **Bounds** (§3.10): a size cap per file and per object read, a total time budget, a per-compile
  timeout, objects deleted as read, the walk's caps; neither the compile's memory nor the size of the
  object it writes is limited by a resource limit (every crate forbids unsafe code; a limit would
  need the launcher pattern perf uses — the first item to add if the map is ever run on untrusted
  downloads at scale), said honestly in §6.
- The project's own build is never run; nothing the map links is ever run.

### 3.10 Limits and bounds

20 000 walked `.c`/`.h` files (the count-only pass counts set-aside and skipped files separately),
depth 32, 200 000 distinct symbol names, a file over 8 MiB not read, an object over 64 MiB not read,
120 s per compile and link, a total budget of 30 minutes for `project map`. The budget is one
deadline through every phase — the hash and parse loops, the `compile_commands.json` reader, each
compile, and each link and probe of the link checks; the link checks compile each file with the
map's own argv and free each object once no closure still to be linked needs it. Memory and size
(2026-10-08): a symbol name over 4 KiB is an odd name; 64 MiB of kept name bytes (every file's
names, repeats counted) is a limit beside the 200 000 names; the parser's names are kept only for a
`.c` that did not compile; `compile_commands.json` is read into typed entries, at most 50 000 of
them, 64 flags and 16 KiB of flags an entry (the rest counted as ignored); a map file over 64 MiB is
a limit hit (`size`: the file facts alone are written, or nothing when even they are over it).
Past any cap: the file facts are written, no closures are computed, exit 1 names the limit. A cut-short walk, like an
unreadable folder, must never produce complete closures, because definers never reached would read
as outside symbols. The 64 MiB object cap bounds what the harness reads, not what the compiler
writes: see §6.

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
- **Its security rules, fixed now (decision 13, §7):** the data is embedded as JSON in which the
  three characters less-than, greater-than and ampersand are written as JSON unicode escapes — a
  backslash, the letter u and the four hex digits 003c, 003e and 0026 — because HTML escaping does not apply inside a `<script>` block and
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
digest moved), and perf marks its stored rows out of date ("the C changed"). The project may hold known or unknown
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
- zopfli (on a copy without its root `harness.toml`, since `accept` refuses one; with a
  `config.toml` stating `flags = []`): one `main` program, 13 files, `-lm`, zopflipng's C++ counted
  as set aside; its `migration/` is pruned; the map's closure and include folders match the
  committed `source_dir`/`include_dirs`; the file-list target written from it gives `facts.jsonl`
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
  `unistd.h` beside the system's and two folders each holding `config.h`: ambiguous-include facts
  recording which header the compile used, no silent pick, `accept` refusing until `config.toml`
  settles it (`-I` or `system_headers`); a header that includes another in a third folder.
- lz4's shape: a fuzz driver of many (kind `driver`, ten `fuzz` programs, never asked "which to
  keep", never offered); a stated configuration whose `-D` adds a file and its pthread needs; without
  it every closure marked guessed; the same project mapped with and without it gives different
  closures, each recorded with its configuration; `ask` refused while guessed (unless `--allow-guessed`), `ask --build` allowed with a
  guessed or a stated configuration; a `compile_commands.json` whose entries differ per file gives a flags-differ fact and
  stays guessed; between-program duplicates listed and never asked; the tool's closure smaller than
  the build's link list, said.
- A made-up project with three `main()`s, a duplicate and a test: the closures, the kind guesses
  labelled, the shared file named after the choice; a collision of a symbol nothing needed inside one
  closure; a definer already in the closure: the need met, no question, the other definer not added; two
  definers that both end up in the closure for other symbols: a collision; a weak/strong pair not a
  duplicate; a weak need recorded as a need on Mach-O; common symbols merged; the `objsyms`
  extension on Mach-O and on ELF objects — made on this Mac with `cc -target
  x86_64-unknown-linux-gnu -c`, gated per host (gcc has no `-target`; Mach-O objects cannot be made
  on Linux) — the same names without the underscore; an `asm`-label symbol and a data
  symbol named `main` (no program); an identifier-shaped file content included into a declaration
  never stored (counted, `outside_includes`).
- A file that does not compile, a file over 8 MiB, an object over 64 MiB, an unreadable folder: the
  closures that need them incomplete (every closure with an outside symbol, for a file neither
  parsed nor compiled), their symbols never outside symbols, `accept` refusing; a program file that
  does not compile listed as such, its files not offered as a library; a non-UTF-8 `.c` parsed from
  its bytes and recorded `not_utf8`; a `.inc` the compile read recorded and hashed.
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
  appears); a missing pinned toolchain refused, never installed; a `PATH` with an empty entry and a
  project `cc`: the project's `cc` never runs; a `compile_commands.json` with `-isystem /dir` as two
  arguments and a relative `-I` against `directory` read as one would expect.
- A ledger made elsewhere refused by every command that opens one (the CLI, the cockpit's read
  model, harness-mcp), `--adopt` recorded per computer with its token, a different tree at the same
  path refused again, `target/` and `build/` folders deleted on adoption (a link deleted, its target
  kept), adopting a listed root deleting nothing; a `build.rs` beside an exact harness manifest
  refused before cargo runs; a unit crate with an extra file refused; zopfli's hand-written crate
  and the harness-made `target/` accepted; a `migration/` holding no harness files refused for
  adoption.
- A pre-placed reply file for another map's digests replaced; a pre-placed response to the same
  question read and shown as advice only; a hand-written response that fails the contract named
  with the way forward; the adoption file at an injected path, never the person's.
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
  model's pick; a set over the limit held; a nested set numbered `under` its choice; one set shared
  by two programs carrying one index; a program labelled `example` by the model still link-checked;
  the resume command carrying `--model`, `--build` and `--allow-guessed`; the reply file replaced
  when the map changed and merged when it did not; a program stem outside the id rule and two
  colliding stems; `--programs` by id.
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
- verify's driver-shape check reads symbols with `objsyms` and refuses an `asm`-forged name; a
  libc name with a `$` suffix stored by its base.

## 5. Order of work

Each step committed green with its tests from §4; then the review, fix passes checked, mutation
checks, DECISIONS. The map's code lives in harness-oracle (`objsyms`, the sandboxed `Runner` and the
profiles are there); the commands in harness-cli; the cockpit's acts in harness-tui.

Steps run (a), (c), (b): (c) comes before (b) on purpose — the file-list target is the riskiest
change and is tested on a hand-written target before any map writes one.

(a) **The walk and symbols over one folder** (`source_dir` of an existing target, as `harness scan`
reads it; `project map` prints and writes nothing beyond the per-file facts yet), replacing nothing
(the walk's additions are new behaviour for every caller, so the cockpit's tree and the features
mirror stop descending folder links — a link's files appear once, under the real path): the walk's four additions
(§3.1 step 1) in harness-core; the `objsyms` extension (kinds, weakness, commons; ELF and Mach-O
tests); the map sandbox profile (its own renderer) and the targetless runner (extracted from bench.rs
into harness-oracle, one allowlist and timeout); the harness's work folder and the children's `PATH`
rule; the refusals (no sandbox, a home root, a ledger made elsewhere, the project's own `migration/`)
and the per-computer adoption file in `TargetContext::load` (the test suite's injected file and
`bench`'s suite adoption with it); the `--json` escape fix; the per-file facts and include folders;
`project map` on zopfli and on the benchmark printing the per-file facts and the include folders
that match today's `source_dir`/`include_dirs` (closures come in step b).

(c) **The layout and the file-list form**, tested by a hand-written file-list target over a liblzg-
shaped fixture before any map writes one: `TargetContext`'s ledger folder and `--tool` on every
subcommand with the lookup order; `harness.toml` v2 read version-first with the grammar at load and
the configuration record; `Base`'s configuration and per-file folders in every compile, the
multi-source builds on perf's compile-then-link path, the driver's folders; the scanner's file-list
read (both include forms; walk errors and non-UTF-8 as facts; `migration/` pruned; detect's walk
replaced); the v2 digest; confinement in every reader; cargo and rustc from the work folder with the
toolchain pinned and `build = false` in the harness manifest; the unit-crate file check before cargo;
verify's driver-shape check moved to `objsyms`; `sync-runtime` per tool (under the project
lock once (b) makes it).

(b) **The whole-root walk and the map**: the configuration file and `--configuration`, the flag
grammar for every source, `compile_commands.json` as a proposal (its two-argument forms), the
programs and their kinds, the closures with duplicates, collisions, incomplete closures and
libraries, the link checks of §3.5 in `map`, the indexes, the map file, `root_hash` and the "project
changed" notice, the project lock and the first `map`'s `.gitignore`, the caps.

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
- **The compile's memory and the size of the object it writes** are bounded by nothing but the
  time budget and the fresh folder: a hostile file of seventy bytes (`const unsigned char
  z[N] = {1};`) can fill the disk in one compile until the map's compiles run through the launcher
  pattern perf uses, with a file-size limit (`RLIMIT_FSIZE`). The first item to add before the map
  runs on untrusted downloads at scale.
- **The map profile still lets a compile read `/private/var/folders`** (the person's own temporary
  files and other apps' caches): what a compile reads there reaches nothing stored, since symbol
  names from such a compile are withheld, but the read itself happens.
- **Whether a file exists outside the home folder can reach the map** (2026-10-08): the profile
  allows reading outside `/Users`, `/Volumes` and the temporary folders, so `__has_include("/Applications/…")`
  steering an `#error` or a missing include tells, through the line a compile stopped at or the
  header it missed, whether that file exists. A compile whose `-MD` list shows it read outside the
  root keeps neither `at` nor `header`, and one bit per file (`outside_includes`) stays; but
  `__has_include` reads nothing, and a compile stopped by a missing header writes no list, so the
  channel stays open until the map profile's reads become an allow-list (§8).
- **A root holding many projects** (`~/code`) is not refused; the map warns when the walk finds
  more than one `.git` folder.

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
- The map profile's reads as an allow-list (the root, the fresh and work folders, the toolchain,
  `/usr`, `/System`, `/dev`), which closes the file-existence channel of §6.

## 9. What changed, and why

**After the review of steps (a) and (c)** (2026-10-08, docs/reviews/2026-10-08-map-steps-a-c-review.md,
its triage): one include rule in one place with the configuration's path flags in the compiler's
order (`-iquote` for quoted includes only; each `-include` file the first include of every listed
file), implemented once in harness-core's `sources::Resolver` and used by every reader and by the
oracle; one include reader (the lexical one; tree-sitter's include facts dropped); a missing listed
file refused at load; the harness's own C runtimes compiled without the configuration and the
wrapper C89-clean; a file-list verdict carries a `configuration` toolchain entry; a fresh token on
every `--adopt`; a results-free `migration/` made here; an agent never adopts; hints spell `--tool`;
`/private/var/tmp` denied in the map profile.

**Revision 2.1** (after the re-check of revision 2, three readers): the unit-crate rule now says what
makes cargo run code and lets zopfli's hand-written crate and the harness's own `target/` through;
the adoption file moves to an injectable path with a per-root token, the test suite and the
benchmark suite are adopted by their own means, and adoption deletes only the harness's build
folders; the walk's four additions are named as changes and scheduled in step (a); the map profile
names what it really denies, and the dependency list's blind spot (inline assembly) is said; a file
neither parsed nor compiled makes every closure with an outside symbol incomplete, and a non-UTF-8
file is parsed from its bytes; the header a compile actually used is recorded and an unsettled
ambiguous include stops `accept`; a need already met in the closure is met (not a collision);
definer sets are numbered once per project and nested sets are counted; `ask` builds nothing and
`ask --build` is always allowed; the reply file's lifecycle and the response files' binding are
written honestly; the duplicate question carries bounded source slices; `compile_commands.json`'s
two-argument forms and relative paths are read; the `-std` set grows; `--` stays out of links; the
capability and ABI-symbol checks keep `nm` over the Rust staticlib until `objsyms` reads archives;
`RUSTUP_TOOLCHAIN` takes the harness's own value with auto-install off; locks for a re-accept and
for the shared `AGENTS.md`; ids stable across maps with a last resort; libc `$` suffixes; the
escapes, the "nothing open" zopfli example, the perf sentence and the "decision 7" reference
corrected; the residuals on disk use and the temporary folder written down.

**Revision 2.2** (after one more reader): a quoted include found beside the including file is never
ambiguous (the rule as written had blocked zopfli and lz4 at `accept`), and `system_headers` folders
are passed with `-idirafter`; `ask --build` is never refused for lack of questions, said in one way
everywhere; the §4 collision line follows step 7; the adoption token is one per project root, the
check is one function the `project` commands call too, the tests adopt through it, and `bench` takes
`--adopt` like every command (decision 8; a `corpus.lock` says nothing about the cases' ledgers);
`.DS_Store` is ignored and the ledger's fixed names are one table; the manifest rule covers the
benchmark's earlier manifests and `[dev-dependencies]`; `--` dropped (every path is absolute);
`objsyms` reads archives through perf's reader already and lacks only section names; the synopses
complete; a library's `name` is its id; `root_hash` counts `included_other`.

### From revision 1 to revision 2 (statements marked "changed in 2.1/2.2" are superseded above)

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
  otherwise (changed in 2.1 and 2.2: the header actually used is recorded; a header beside the
  including file is never ambiguous), headers scanned too and given a status. (Facts.)
- **Says what `objsyms` must learn** (kinds, weakness, commons; external only), writes the weak and
  common rules into the closure step, and the definer-already-in-the-closure rule (changed in 2.1: such
  a need is met, not a collision). (Facts.)
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
  today's staleness rule and makes `root_hash` a notice, moves verify's `nm` uses to `objsyms`
  (changed in 2.1: the driver-shape check only, until `objsyms` reports section names).
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
