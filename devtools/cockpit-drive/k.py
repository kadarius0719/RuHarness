"""Send commands to cockpit.py and print the screen once it settles.

Usage: python3 -I k.py <workdir> [--until TEXT]... [--gone TEXT] [--timeout S] [--quiet S]
                       [--grep REGEX] [cmd ...]
Each positional cmd is one line for the FIFO (e.g. "key Down 3", "text Migrate", "key Enter").
Waits until every --until TEXT is on screen (and --gone TEXT is not), else until the screen has
not changed for --quiet seconds (default 1.0); never past --timeout (default 20). Prints the screen
(or only lines matching --grep).
"""
import os
import re
import sys
import time

args = sys.argv[1:]
workdir = args.pop(0)
until, gone, timeout, quiet, grep, cmds = [], [], 20.0, 1.0, None, []
cols = None
plain = False
while args:
    a = args.pop(0)
    if a == "--until":
        until.append(args.pop(0))
    elif a == "--gone":
        gone.append(args.pop(0))
    elif a == "--timeout":
        timeout = float(args.pop(0))
    elif a == "--quiet":
        quiet = float(args.pop(0))
    elif a == "--cols":
        a0, b0 = args.pop(0).split(":")
        cols = (int(a0), int(b0))
    elif a == "--plain":
        plain = True
    elif a == "--grep":
        grep = re.compile(args.pop(0))
    else:
        cmds.append(a)

path = os.path.join(workdir, "screen.txt")


def read():
    try:
        with open(path) as f:
            return f.read()
    except FileNotFoundError:
        return ""


before = read()
if cmds:
    with open(os.path.join(workdir, "cmd"), "w") as f:
        for c in cmds:
            f.write(c + "\n")
start = time.time()
last, changed_at = read(), time.time()
while time.time() - start < timeout:
    time.sleep(0.2)
    now = read()
    if now != last:
        last, changed_at = now, time.time()
    body = now.split("\n", 1)[1] if "\n" in now else now
    if until or gone:
        if all(u in body for u in until) and not any(g in body for g in gone):
            break
    elif time.time() - changed_at >= quiet and (now != before or not cmds or time.time() - start > quiet * 2):
        break
else:
    print(f"!! timeout after {timeout}s (until={until} gone={gone})")
out = read()
if plain or cols:
    lines = []
    for l in out.splitlines():
        if plain:
            l = l.split("   <<", 1)[0]
        if cols and "|" in l[:4]:
            pre, body = l.split("|", 1)
            l = pre + "|" + body[cols[0]:cols[1]].rstrip()
        lines.append(l)
    out = "\n".join(lines)
if grep:
    out = "\n".join(l for l in out.splitlines() if grep.search(l))
print(out)
