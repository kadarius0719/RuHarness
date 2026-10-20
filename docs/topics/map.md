# The project map

By the end of this page you will know how RuHarness finds the programs in a C project you downloaded, which choices it leaves to you, what accepting a program does, and what happens when the project changes later.

## Programs

A C project you download has no `harness.toml` (the file saying what to translate). `harness project map`, run inside the project's folder, makes one possible. It compiles every `.c` file in the **sandbox** (locked-down build space), never changing the project's own files, reads which functions each file offers and needs, and follows the needs from each `main()` (where a program starts).

A **program** is a `.c` file with its own `main()`, plus every file it needs. The map names it `t-` plus that file's name: liblzg holds `t-lzg` (compress), `t-unlzg` (decompress) and `t-benchmark`. For each, it lists the files and a **link check**: "linked" means every function the program calls is defined exactly once. A **shared file**, such as liblzg's `checksum.c`, is one several programs need. The map writes `migration/map/project-map.json`; it is always called the project map, and is a different thing from the feature map.

## Held choices

Sometimes two files offer the same functions. In liblzg both the library's decoder `src/lib/decode.c` and a small stand-alone one, `src/extra/lzgmini.c`, offer `LZG_Decode`. A program can link only one, and when the map cannot tell which is meant it **holds the choice**: it names it `d1`, lists the files as `d1.1` and `d1.2`, and waits for you. Both would link, and "it links" does not prove the right file was kept, so the choice stays yours. You can ask a model for advice (`harness project ask`); its answer is shown beside the choice, labelled as the model's, with one of three reasons: `platform`, `alternative-implementation` or `cannot-tell`. It never decides.

## The configuration

The map also needs to know how the project is normally built: above all the **flags** (compiler options) that change which code is compiled, such as `-I` plus a folder (where to find headers) and `-D` (a definition the C can test). That is the **configuration**, four lines you write in `migration/map/config.toml`:

```toml
[[configuration]]
name = "make"
from = "make"
flags = ["-O3", "-Isrc/include"]
```

Until you write it, the map compiles under a guess with no flags, and nothing can be accepted: a guess may hide the very errors that matter. A flag the harness does not pass, such as `-Wall`, is refused with its reason. `harness project ask --build` asks a model to propose the file.

## Accept, tool and target

`harness project accept t-lzg` checks that the map still matches the files, links the program once more with your picks (`--keep d1=d1.2` names the file to keep), and writes `migration/tools/t-lzg/harness.toml`. That file **is** the acceptance: you never write it by hand. A **tool** is an accepted program, which then plays the target's part (the target being the C project being migrated): it gets its own **ledger** (record folder) in `migration/tools/<id>/`, and every command names it with `--tool t-lzg` where a target is named with `--target`. A command without `--target` works on the folder you are in.

An accepted tool does not know how its program is run, so its whole-program check is off at first: verify shows `[SKIP] whole-program — not run`. Adding the program's arguments to the end of its `harness.toml` turns it on:

```toml
[oracle.whole_program]
args = ["-9"]
```

## What changes later

When the project's files change, the next map says which tools changed and how, and the tool's status starts with the same sentence. When only the tool's own C changed, it says to scan the tool; when what it needs or how it links changed, it says to accept it again. Accepting again rewrites only what the map decides (the files, folders, configuration, picks, run name and the map it came from) and keeps the rest: its ledger, and what you added, such as `[oracle.whole_program]`. The closing line names what was kept.

In the cockpit, a folder with no `harness.toml` opens on its **Project view**, the same full-screen look as everywhere else. Before a map the left pane holds one row, `Map the project`; after a map it is the one list of the project: a heading (`Programs (3)`, with its counts by state when they fit the pane on one line), one row per program with its state in words — `links`, `needs a decision`, `accepted`, `did not compile`, `did not link` — and the libraries when there are any. The right pane starts with the **Next step** in one line (`Next: Accept t-benchmark — Enter on its row, it asks for the file`), then a card for the selected row: its file and what the map guesses it is (`src/tools/benchmark.c · looks like a tool (it is not in a test, example or benchmark folder)`), each decision with its files numbered 1, 2, the file the model advises marked and the model's why, and what `Enter` does when the Next step does not already say it. The help (`?`) opens on the Project view's own page: its keys, its marks and what each state means. `Enter` on the `Map the project` row opens a small menu, Map first (with `--no-chat`, its dialog at once). `Enter` on a row opens its menu with the recommended act first: **Map the project** (or **Map the project again**), **Write the build configuration** while the configuration is a guess (it opens `migration/map/config.toml` in your editor, filled in with the example the map prints, checks it and writes it; then you map again), **Ask for a proposal — ask in chat** while the configuration is a guess (**Ask for advice — ask in chat** once your build configuration is written and a decision waits), **Accept this program**. Each act is the cockpit's usual dialog: it says what it does, shows the same command as above, how long it takes and what it writes, and runs only on `y`; its output is in the activity details (`c`), and the view reads the map again afterwards. While the configuration is a guess, Accept is greyed and says it waits for your build configuration. Accepting a program with a decision first asks, one menu per decision, `Keep which file for the functions both define?`, the files numbered (the one a model advised, if you asked, is marked `← the model advises this one`, with its why above, and the highlight starts on it; nothing is picked until `Enter`); the dialog then says each choice in words, and whether the model advised it. An accepted program's row is its tool's row: `Enter` on it leads with **Open tool t-lzg**; right after the Accept it is selected and the Next step names it. The tool's screen keeps a first row, **◂ Back to the project view**.

## Try it

These boxes run inside `~/lzg-map`, the guide's copy of the whole liblzg download, with the configuration above written and `t-lzg` already accepted (`harness project accept t-lzg`) and scanned.

```bash
harness project map
```

Look for: `programs: 3`, `link check: linked` under `p2 t-lzg`, and `duplicate set d1 (LZG_Decode): held, … so the choice is yours`.

```bash
harness project accept t-unlzg
```

Look for: a refusal, `error: duplicate set d1 of t-unlzg (LZG_Decode, LZG_DecodedSize) is not settled: pick its definer yourself with --keep d1=…`.

```bash
harness project accept t-unlzg --keep d1=d1.2
```

Look for: a line keeping `src/lib/decode.c` over `src/extra/lzgmini.c`, then `the whole-program check is off until you fill in [oracle.whole_program]`, and `wrote migration/tools/t-unlzg/harness.toml`.

```bash
cat migration/tools/t-lzg/harness.toml
```

Look for: the first line ``# Written by `harness project accept t-lzg` …``, the four files under `[target]`, and the `map = …` line that ties the tool to this map.

```bash
harness state status --tool t-lzg
```

Look for: one line per unit; after you change one of the tool's C files and map again, the first line is the map's note, such as `its own files changed since it was accepted (src/lib/version.c) …`.

The full walk: docs/TESTING-GUIDE.md Part 12. The ideas in full: docs/TUTORIAL.md section 13.
