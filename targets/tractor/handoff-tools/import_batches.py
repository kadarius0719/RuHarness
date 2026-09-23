#!/usr/bin/env python3
"""Audit-then-import for hand-off batches (scratch tooling).
usage: import_batches.py <batchmap.txt> <stage> <round> <model>
For each "batch agent_id" line: audit the agent transcript (tasks/<id>.output);
only a CLEAN audit (every tool call is Read/Write inside the batch dir) imports
the answers as <key>.response.json next to each request. Appends one line per
batch to targets/tractor/handoff-audit.jsonl (committed evidence)."""
import json, os, sys, subprocess
S = os.environ["HANDOFF_ROOT"]            # batch dirs, indexes, batch maps
TASKS = os.environ["HANDOFF_TRANSCRIPTS"]  # <agent-id>.output transcripts
TOOLS = os.path.dirname(os.path.abspath(__file__))
SUITE = os.path.join(os.environ.get("RUHARNESS_REPO", os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))), "targets/tractor")
bmap, stage, rnd, model = sys.argv[1:5]
log = open(f"{SUITE}/handoff-audit.jsonl", "a")
for line in open(bmap):
    if not line.strip(): continue
    batch, aid = line.split()
    bdir = f"{S}/handoff/{batch}"
    transcript = f"{TASKS}/{aid}.output"
    audit = json.loads(subprocess.run([sys.executable, f"{TOOLS}/audit.py", transcript, bdir], capture_output=True, text=True).stdout)
    index = json.load(open(f"{S}/handoff-index/{batch}.json"))
    keys, imported, missing = [], 0, []
    clean = not audit["violations"]
    for n, req in sorted(index.items(), key=lambda kv: int(kv[0])):
        rel = os.path.relpath(req, SUITE)
        keys.append(rel)
        ans = f"{bdir}/{n}.answer.txt"
        if not os.path.exists(ans): missing.append(n); continue
        if clean:
            resp = req[:-len(".request.json")] + ".response.json"
            json.dump({"text": open(ans).read(), "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}, open(resp, "w"))
            imported += 1
    rec = {"stage": stage, "round": rnd, "batch": batch, "model": model, "agent": aid,
           "tool_calls": audit["tool_calls"], "violations": audit["violations"],
           "deviations": audit.get("deviations", []),
           "imported": imported if clean else 0, "missing": missing, "requests": keys}
    log.write(json.dumps(rec, sort_keys=True) + "\n")
    print(f"{batch}: calls={audit['tool_calls']} breaches={len(audit['violations'])} deviations={len(audit.get('deviations', []))} imported={rec['imported']}/{len(index)} missing={missing}")
