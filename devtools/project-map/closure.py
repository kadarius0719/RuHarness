"""Link closures of a C project: which files make up each program (docs/PROJECT-MAP-INVESTIGATION.md).

Usage: python3 -I closure.py <project dir> <scratch dir> [-D...]...
Compiles every .c alone (cc -c, include folders = every folder holding a .h), reads each object's
defined and needed global symbols with nm, then from every object that defines main pulls in, file
by file, whichever object defines each needed symbol. Prints JSON to stdout: per entry point its
closure, outside symbols, ambiguous symbols (several definers), and the files left in no closure.
Objects that define main are never pulled into another closure.
"""
import json
import os
import subprocess
import sys

root = os.path.realpath(sys.argv[1])
scratch = os.path.realpath(sys.argv[2])
defines = [a for a in sys.argv[3:] if a.startswith("-D")]
os.makedirs(scratch, exist_ok=True)

c_files, h_dirs, other = [], set(), {}
for d, dirs, files in os.walk(root):
    dirs[:] = sorted(x for x in dirs if not x.startswith("."))
    for f in sorted(files):
        p = os.path.join(d, f)
        ext = os.path.splitext(f)[1]
        if ext == ".c":
            c_files.append(os.path.relpath(p, root))
        elif ext == ".h":
            h_dirs.add(d)
        elif ext in (".cc", ".cpp", ".cxx", ".m", ".go", ".rs", ".py"):
            other[ext] = other.get(ext, 0) + 1

incs = [f"-I{d}" for d in sorted(h_dirs)]
objs, failed = {}, {}
for rel in c_files:
    obj = os.path.join(scratch, rel.replace("/", "__") + ".o")
    r = subprocess.run(["cc", "-c", "-w", "-O0", *defines, *incs, "-o", obj, os.path.join(root, rel)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        first = next((l for l in r.stderr.splitlines() if "error" in l), r.stderr.strip()[:200])
        failed[rel] = first.replace(root + "/", "")
        continue
    nm = subprocess.run(["nm", "-g", obj], capture_output=True, text=True).stdout
    defs, needs = set(), set()
    for line in nm.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0] == "U":
            needs.add(parts[1])
        elif len(parts) == 3 and parts[1] in "TDBSC":
            defs.add(parts[2])
    objs[rel] = {"defs": defs, "needs": needs}

definers = {}
for rel, o in objs.items():
    for s in o["defs"]:
        definers.setdefault(s, []).append(rel)
mains = sorted(r for r, o in objs.items() if "_main" in o["defs"])
fuzz = sorted(r for r, o in objs.items() if "_LLVMFuzzerTestOneInput" in o["defs"])

entries = []
in_some = set()
for entry in mains + fuzz:
    closure, todo, outside, ambiguous = {entry}, [entry], set(), {}
    while todo:
        cur = todo.pop()
        for s in sorted(objs[cur]["needs"]):
            ds = [d for d in definers.get(s, []) if d not in mains or d == entry]
            if not ds:
                outside.add(s)
            elif len(ds) > 1:
                ambiguous[s] = ds
            elif ds[0] not in closure:
                closure.add(ds[0])
                todo.append(ds[0])
    in_some |= closure
    entries.append({"entry": entry, "kind": "main" if entry in mains else "fuzz",
                    "closure": sorted(closure), "outside": sorted(outside),
                    "ambiguous": ambiguous})

shared = {}
for e in entries:
    for f in e["closure"]:
        if f != e["entry"]:
            shared.setdefault(f, []).append(e["entry"])
dups = {s: ds for s, ds in definers.items() if len(ds) > 1 and s != "_main"}
print(json.dumps({
    "c_files": len(c_files), "compiled": len(objs), "failed": failed, "other_sources": other,
    "include_dirs": [os.path.relpath(d, root) for d in sorted(h_dirs)],
    "entries": entries,
    "in_no_closure": sorted(set(objs) - in_some),
    "shared_by": {f: sorted(es) for f, es in sorted(shared.items())},
    "duplicate_definitions": dups,
}, indent=1))
