# devtools — scripts for building and testing RuHarness itself

Not part of the harness and never run by it. Python 3, standard library only; run with `python3 -I`.

## cockpit-drive/ — drive the cockpit headless, like a person at a terminal

Used for the liblzg walkthrough of docs/TESTING-GUIDE.md (2026-10-07, Parts 4, 6 and 9 through the
real chat). `cockpit.py` runs `harness-tui` in a pseudo-terminal of a given size with a plain
environment (HOME PATH USER LOGNAME SHELL LANG TMPDIR, TERM=xterm-256color — as from a plain
Terminal window, so a Claude Code session's own variables never reach the chat), renders the screen
with a small VT interpreter into `<dir>/screen.txt`, and takes commands from the FIFO `<dir>/cmd`.
Each screen line is followed by `<<Y:…| G:…| I:…| D:…>>` marks: yellow, green, reverse video (the
highlighted row) and grey (a disabled item).

    mkdir -p /tmp/s1
    cd <repo>; PATH="$PWD/target/debug:$PATH" python3 -I devtools/cockpit-drive/cockpit.py /tmp/s1 50 160 -- harness-tui --target targets/lzg   # in the background
    python3 -I devtools/cockpit-drive/k.py /tmp/s1 --until "Ready." --plain             # the first screen
    python3 -I devtools/cockpit-drive/k.py /tmp/s1 "key Down 11" --plain --cols 32:112  # a key, then the View's columns
    python3 -I devtools/cockpit-drive/k.py /tmp/s1 "key Enter" --until "Asks:" --timeout 240 --plain --cols 112:160
    echo quit > /tmp/s1/cmd                                                             # end the driver

`k.py` waits until every `--until TEXT` is on screen (or `--gone TEXT` is not), else until the screen
has been quiet for `--quiet` seconds, never past `--timeout`. Lessons: a dialog ignores keys for a
moment after it opens (by design) — wait for its own `ready: → then Enter` line before `→` and
`Enter`, or the Enter lands on Cancel; `--until "ready"` can match other text on screen; the
highlighted menu row is the `I:` mark (a greyed row can be highlighted: `ID:`).

## project-map/closure.py — link closures of a C project

The spike of docs/PROJECT-MAP-INVESTIGATION.md: compiles every `.c` alone (`cc -c`, include folders
= every folder holding a `.h`, `-D` flags only when given as arguments), reads `nm -g`, and prints
JSON: each `main`'s and fuzz target's closure, outside symbols, ambiguities, files in no closure,
duplicate definitions. Compiles only — runs nothing from the project.

    python3 -I devtools/project-map/closure.py <project> <scratch dir> [-DNAME ...] > map.json
