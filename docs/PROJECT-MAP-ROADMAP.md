# Roadmap note: real project layouts and a fuller C–Rust boundary

Status: **idea, not designed yet.** Recorded 2026-10-01 at the person's request, so it can be
picked up after the C-vs-Rust speed work is merged. Next step: a proper design document with
review rounds (the treatment docs/PERF-DESIGN.md got), weighed against the briefing's M5 for
order.

## Why

Most real C projects are spread over folders (`src/`, `lib/`, `include/`, `third_party/`,
`tests/`, `examples/`, platform folders like `win32/`) and often build several programs from
shared code — curl, for example. RuHarness today only handles the tidy case, so a person who
downloads a project and points the harness at it hits a wall almost at once.

## What the harness assumes today

- `harness.toml` names **one** `source_dir`; the scanner reads that folder's top-level `.c`
  files and the headers they include. Subfolders are not read, except header folders listed in
  `include_dirs`, which must lie inside `source_dir`.
- Every `.c` in that folder is **one program**, built with one `cc` command and one set of flags
  (plus libraries from `[oracle] extra_link_args`). The judge, the features runs and the speed
  comparison all build it that way (`whole_cc_into` in harness-oracle).
- No build system is read: no Makefile, CMake or `compile_commands.json`.
- Only **leaf units** migrate: C that calls nothing else in the project, only the standard C
  library — so Rust never calls back into C.
- Each migrated unit is its own Rust staticlib (`logic.rs` safe Rust + `ffi.rs` exporting the
  same C names); the C program links it in place of the unit's C.

So a person gathers one program's C into one folder by hand (the testing guide does this for
liblzg). On a typical download, `scan` and `plan` still work (they parse, they don't build), but
the build fails: several `main()`s (tests, examples, tools), platform-only files, a `config.h`
that `./configure` or CMake must generate first, code spread over folders.

## Part 1 — Map a project with several programs, deterministically first

The person's direction: scan a whole project **deterministically**, produce a format a model can
read to decide what one tool is, bin the files, then break things up and investigate.

The approach agreed in conversation — **the harness finds the facts, a model judges only the
ambiguous part, the harness checks the model's answer, the person accepts** (the pattern the
rest of RuHarness uses):

1. **Deterministic map (no AI).** Walk the whole project and record:
   - every `.c` / `.h`: path, folder, size, blake3;
   - per file: functions, calls, includes (the scanner records these already, for one folder);
   - entry points: files that define `main()` — each a candidate tool;
   - **linker evidence (the strongest signal):** compile each file to an object and list, with
     `nm`, the symbols it defines and the ones it needs; from that, each `main()`'s **link
     closure** — the smallest set of files that satisfies all its needs. That closure is "one
     tool", found mechanically;
   - shared code: files in several closures are a library the tools share;
   - build evidence when present: `compile_commands.json` gives the exact files and per-file
     flags (`-I`, `-D`); CMake writes it with one switch, Bear makes one for Make projects. A
     Makefile or CMakeLists is recorded as present, not interpreted;
   - what did not resolve, and why: a file that does not compile (needs a generated
     `config.h`, or is for Windows), a symbol defined in two files (platform or alternative
     versions), a symbol from an outside library (zlib, libm).
2. **A compact, structured project-map file**: facts and summaries only, no source, so it fits
   a model's context for big projects; every text that comes from the project fenced as
   untrusted (as the harness-mcp reads do).
3. **The model resolves the ambiguous part only**: names each tool and what it does; marks tests,
   examples and benchmarks to set aside; picks between duplicate definitions ("the `unix/`
   version, not `win32/`"); groups shared files into libraries. Its answer is a strict format:
   bins, each a list of files with reasons.
4. **The harness checks the answer**: every proposed tool compiles and links from exactly its
   files; no file is silently dropped — each is in a bin or excluded with a reason; overlaps only
   as declared shared libraries. A wrong answer is refused, with the reason.
5. **The person reviews and accepts the bins.** Each accepted tool becomes a migration target;
   each shared library becomes units whose Rust is judged against **every** tool that links it.

Why this order: the link closures alone usually get it right; the model only settles what is
left, so it is cheaper and repeatable, and the link check catches a model's mistake that leaves a
symbol missing or doubled — not a wrong kind, a wrong definer when both link, or wrong flags (the
spike and the design review, 2026-10-07; design §1).

Staged value, roughly:
1. Use the project's own build information (`compile_commands.json`): several folders and
   per-file flags (which today's single flag set cannot express, e.g. `-DHAVE_CONFIG_H`).
2. Choose which program when several are built (library, tools, tests).
3. Say plainly what was left out and why (platform-only, not built, needs `./configure`).

## Part 2 — A fuller C–Rust boundary (FFI) once programs share code

Migrating an app with several tools makes the boundary harder:

1. **Shared code in several programs**: a `lib/` unit's Rust links into every tool that uses it,
   and the judge must check each tool, not one.
2. **Calls both ways**: real code is mostly non-leaf; Rust must call C that has not moved yet
   (Rust-to-C declarations kept in sync as units move; the plan's order matters more).
3. **Several Rust units in one program**: each unit is its own staticlib with its own copy of
   Rust's standard library; linking two can clash. The speed work already names these
   (mixed panic runtimes; does not link: `no-std`, `two-no-std`, `lto`, `two-lto`). At scale:
   one combined Rust library per program.
4. **Shared data and memory**: structs both sides read (`#[repr(C)]`, exact layout), globals,
   memory allocated on one side and freed on the other, callbacks (function pointers), `errno`,
   varargs.
5. **The end state**: when all of a program's units are Rust, drop the C-facing wrapper layer
   and leave a plain Rust program, not Rust posing as C forever.

Part 1 comes first; Part 2 only matters once real layouts are handled.

## Hard questions for the design

- Projects that do not compile until `./configure` or CMake has run (ask the person to run it
  first? detect and say so?).
- `#ifdef` code whose meaning changes with the configuration.
- One file built twice with different flags (two programs, two configurations).
- Size limits for very large projects (the map file, the model's context, compile time).
- Sandboxing the per-file compiles of an unknown project (the tool profile already exists).
- What a "target" becomes: one `harness.toml` per tool, or one project file listing tools.
- What this touches: the scanner (`harness-scan`), facts, the plan, the judge's build
  (`harness-oracle`, `whole_cc_into`), features, and perf (it builds as verify does).

## Where this came from

Conversation of 2026-10-01 (session 17c366c0), after the person asked whether the harness can
be pointed at a downloaded folder and "figure things out". Answer at the time: no — it needs a
`harness.toml` and one folder of one program's C; this note is the plan to change that.
