#!/usr/bin/env python3
"""export_awaited.py <awaited-list> <batch-prefix> <per-batch>: the harness's
CURRENT awaited responses -> isolated prompt batches (indexes kept outside)."""
import json, os, sys
S = os.environ["HANDOFF_ROOT"]
lst, prefix, per = sys.argv[1], sys.argv[2], int(sys.argv[3])
resp = [l.strip() for l in open(lst) if l.strip()]
reqs = [r[:-len(".response.json")] + ".request.json" for r in resp]
names = []
for b in range(0, len(reqs), per):
    name = f"{prefix}-{b//per+1:02d}"; d = f"{S}/handoff/{name}"; os.makedirs(d, exist_ok=False)
    idx = {}
    for i, req in enumerate(reqs[b:b+per], 1):
        r = json.load(open(req))
        open(f"{d}/{i}.prompt.txt", "w").write("=== SYSTEM PROMPT ===\n" + r["system"] + "\n\n=== USER MESSAGE ===\n" + r["user"] + "\n")
        idx[str(i)] = req
    json.dump(idx, open(f"{S}/handoff-index/{name}.json", "w"), indent=1)
    names.append(name)
print(" ".join(names))
