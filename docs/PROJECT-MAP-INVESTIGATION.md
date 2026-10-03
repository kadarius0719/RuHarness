# Investigation: can a project's tools be found mechanically?

Date 2026-10-02. A throwaway experiment for docs/PROJECT-MAP-ROADMAP.md, Part 1. Nothing in
RuHarness was changed; the script is `scratchpad/pmap/closure.py` (session 17c366c0) — about 40
lines: compile each `.c` to an object, read what it defines and needs with `nm`, and from every
`main()` pull in whichever file defines each needed symbol until nothing new is added.

## What is on this Mac to look at

- **The 100 benchmark cases** (`targets/tractor/cases`): every one has the same shape — a
  CMake file, `test_case/include/*.h`, `test_case/src/*.c`, built as a **library** (99 have no
  `main()` at all; one has one). So the benchmark never exercised the "which program?" question:
  each case is already one unit of one library. It does show something else: the C sits in
  `src/`, a subfolder of `source_dir`, and the harness only reads it because `bench init` wrote
  `source_dir = "test_case"` with `include_dirs` — one folder level is the most the current
  layout rule copes with.
- **zopfli**: a real project with **two programs**. `src/zopfli/` is the C compressor (13 files,
  one `main()` in `zopfli_bin.c`); `src/zopflipng/` is a second tool written in **C++** with a
  vendored library (`lodepng/`). Its CMakeLists and Makefile both name two executables and two
  libraries. Today's `harness.toml` points at `src/zopfli` and never sees the rest. Also present
  and irrelevant: a `go/` port.
- liblzg is not here (the testing guide downloads it). Its upstream layout, from the guide:
  `src/lib/` (the library), `src/tools/` (three programs: `lzg`, `unlzg`, `benchmark`),
  `src/include/` (the public header). The guide has the person copy `lib/*.c`, `lzg.c` and the
  headers into one folder by hand, and drop the other two tools — exactly the work the map should
  do.

## Result 1 — zopfli: the closure is exact

From `zopfli_bin`'s `main`, the closure pulls in all 13 C files, in one pass, with **no duplicate
symbols** and **29 outside symbols** — every one from libc or libm (`_malloc`, `_fprintf`,
`_log`, `___stack_chk_fail`, …). Nothing is left over. The C++ tool is set aside by its file
type before anything is compiled. So on zopfli the map alone, with no model, gives today's
`harness.toml` back: the right folder, the right program, `-lm` as the one outside library.

## Result 2 — a made-up multi-tool project: the ambiguity shows up where expected

Seven files: `lib/{sum,fmt,plat}.c`, `win32/plat.c`, `tools/{adder,namer}.c`,
`tests/test_sum.c`. Three `main()`s, one duplicate symbol (`plat` in `lib/` and `win32/`).

| entry point | closure | outside | note |
|---|---|---|---|
| `tools/adder` | `lib/fmt`, `lib/sum`, itself | `printf` | a tool |
| `tools/namer` | `lib/plat`, `win32/plat`, itself | `puts` | **ambiguous**: two files define `plat` |
| `tests/test_sum` | `lib/sum`, itself | none | a test program (reaches one library file, lives in `tests/`) |

Shared code: `lib/sum` is in two closures — a library the tools share. The mechanical step
separates the tools correctly and surfaces the only real question (which `plat`?) as a duplicate
it cannot resolve — the kind of question the model step is for. "Is `tests/test_sum` a test?" is
likewise a judgement (its folder name and its reaching almost nothing both say so), not a fact.

## What this settles for the design

1. **Link closures work as the backbone.** `cc -c` + `nm` per file is cheap (zopfli: well under a
   second), deterministic, and needs no build system. It gives tools, shared libraries, outside
   libraries and duplicate definitions directly.
2. **The model's job is small and well-shaped:** duplicates (platform / alternative versions),
   naming tools, marking tests / examples / benchmarks, grouping shared files. Everything it says
   is checkable by linking.
3. **Non-C files are set aside by type** (`.cc`, `.cpp`, `.go`) — zopflipng, lodepng.
4. **Per-file flags matter in practice.** The benchmark cases need `-Iinclude`; real projects
   need `-D` defines. The per-file compile must take the include folders found by walking the
   tree (every folder holding a `.h`) as a first guess, and `compile_commands.json` when present.
5. **The output of the map is today's `harness.toml` plus a list**: `source_dir` can no longer be
   one folder — a tool is a *set of files*, so the target description needs a file list (or the
   map's bin id) instead. This touches the scanner, the plan, the judge's build, features and
   perf, as the roadmap note says.

## Open questions the experiment raised

- Files that do not compile on their own (missing generated `config.h`, platform-only code):
  the experiment had none. The map must record them as "could not compile: <first error line>"
  and go on — the closure is then partial and must say so.
- A duplicate `main()` is normal (several tools) but a duplicate non-`main` symbol is either a
  platform choice (resolve by folder name and the model) or a real conflict (refuse).
- Static (file-local) functions are invisible to `nm -g`: fine for the closure (they never cross
  files), but the per-file function list the scanner already makes should stay the source for
  "what is in this file".
- Header-only dependencies (a `.h` with inline code) do not appear in the closure at all; the
  scanner's include graph covers them. The map needs both views side by side.
