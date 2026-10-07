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

## The spike on real downloads (2026-10-07)

Step 7 of docs/NEXT-WEEK-PLAN.md: the closure script, rewritten (about 90 lines, the session's
scratchpad `pmap/closure.py`: every `.c` compiled alone with `cc -c -w -O0`, include folders = every
folder holding a `.h`, `-D` flags only when given; `nm -g` for defined and needed; closures from every
`main` and every `LLVMFuzzerTestOneInput`; a file that defines `main` is never pulled into another
closure). Run on two downloads kept outside the repository (`~/code/ruharness-test-downloads/`, the
person's rule: test data only, nothing installed): **lz4** at 0774d05 (a library, a command-line
tool, tests, examples, fuzzers; three build systems) and **liblzg** at 182b56c (the testing guide's
library, its three tools and an `extra/` folder).

### Result 3 — lz4: the closures separate 33 programs in two seconds

48 `.c` files, all compiling alone with no flags (2.2 s). 33 entry points: 23 `main`s (1 tool in
`programs/`, 10 in `examples/`, 12 in `tests/`) and 10 fuzz targets in `ossfuzz/`. The folder names
alone would classify every one of them. `lib/lz4.c` is in 32 of the 33 closures — the library they
all share; `tests/datagencli.c` reaches into `programs/lorem.c` — sharing across folders. The tool:

| entry | closure (besides itself) | outside |
|---|---|---|
| `programs/lz4cli.c` (no flags) | `lib/{lz4,lz4frame,lz4hc,xxhash}.c`, `programs/{bench,lorem,lz4io,threadpool,timefn}.c` | 60 (libc) |
| `programs/lz4cli.c` with `-DLZ4IO_MULTITHREAD -DNDEBUG` | the same **plus `programs/util.c`** | the same **plus 11 `pthread_*` and `sysctlbyname`** |

What it found that the design underrates:

1. **The build's flags decide the program, not only a residual.** lz4's Makefile builds `lz4` with
   `-DLZ4IO_MULTITHREAD` (and `-pthread`) whenever pthreads exist. Without that define the closure
   misses a file and — more important for a migration — misses that the tool is multithreaded. Both
   configurations compile and link, so **a proposed flag cannot be checked by linking**: the link
   check of design §3.4 does not catch a wrong configuration. lz4 ships no `compile_commands.json`.
2. **The project's own build systems disagree.** The Makefile and the Meson file
   (`build/meson/meson/programs/meson.build`: `multithread_args = ['-DLZ4IO_MULTITHREAD']`) build the
   tool with threads; `build/cmake/CMakeLists.txt` builds it without. "The untouched project's
   behaviour" (design §3.10's principle: match, not right) therefore needs **one named build
   configuration** as its baseline, chosen by the person — the map cannot pick it.
3. **The project links more than the program needs.** The Makefile links `$(wildcard lib/*.c)` and
   `$(wildcard *.c)`: `lib/lz4file.c` (and, without threads, `programs/util.c`) are linked but never
   reached. The closure is the smaller set; matching behaviour is unaffected, but the map's file list
   and the project's build will differ by design — say so where the map shows a tool.
4. **One `main` for many programs.** `ossfuzz/standaloneengine.c` defines `main` and needs
   `LLVMFuzzerTestOneInput`, which 10 fuzzers define: it is a driver linked once with each fuzzer, not
   a program. The closure reports it as a 10-way ambiguity, correctly; the question for the model is
   "what is this?", not "which one?".
5. **Duplicates between programs are harmless.** 14 helper names (`write_bin`, `read_bin`,
   `compare`, …) are defined in several `examples/` and `tests/` files, each its own program and
   never in one closure. Only a duplicate met **inside** a closure matters — the investigation's
   open question ("platform choice or conflict") applies to those alone.
6. Tools are not on this machine: no `cmake`, `meson` or `bear` (the usual ways to get a
   `compile_commands.json`). Running a project's build to learn its flags would execute its build
   logic (Makefile `$(shell …)` probes, CMake scripts) — outside the sandbox today.

### Result 4 — liblzg: the closure is the guide's hand-picked file list

8 `.c` files, all compiling. `src/tools/lzg.c`'s closure is `src/lib/{checksum,encode,version}.c` —
exactly what Part 1 of the testing guide has the person copy (the guide also copies `decode.c`, which
`lzg` never reaches; harmless). A **real duplicate**, the case the model step exists for:
`src/extra/lzgmini.c` (a stand-alone mini decoder) defines `LZG_Decode` and `LZG_DecodedSize` like
`src/lib/decode.c`, so `unlzg` and `benchmark` are each ambiguous between the two — and either choice
links. `ShowProgress`/`ShowUsage` are defined in two tools: harmless (never in one closure).

### What the spike changes, for the design review

- **Flags are evidence the map must be given, not guessed.** Order of trust: a
  `compile_commands.json`; else flags the person states for a named build (`make`, `meson`, `cmake`)
  — a model may read the build files and propose them, but a proposal is advice the person
  confirms, because the link check cannot tell configurations apart; else the walk's guess, marked
  as a guess on every closure it shaped. Design §3.1 step 4 and §6's `#ifdef` residual need this.
- **The baseline is one named configuration** (design §3.10's "match" principle): record which
  build and which flags the baseline program was made with, and refuse to compare against another.
- **Entry kinds:** `main`, fuzz target (`LLVMFuzzerTestOneInput`), and "driver of many" (a `main`
  whose need is met by many files) — the third is not a program.
- **Only in-closure duplicates are questions;** between-program duplicates are listed, not asked.
- Nothing here argues for a bigger mechanism: the closures themselves were exact and cheap on both
  downloads. The open part is the configuration, and that is a question for the person.
