#!/usr/bin/env python3
"""Scratch orchestration (NOT part of the harness): run one harness stage
command for every suite case in parallel and summarize the outcome lines.

usage: stage.py <gen-driver|migrate> <model> [--jobs N] [--only case,case] [--extra ARGS...]
"""
import os
import tomllib, subprocess, sys, os, concurrent.futures as cf, json, re
REPO = os.environ.get("RUHARNESS_REPO", os.path.abspath(os.path.join(os.path.dirname(__file__), "../../..")))
SUITE = REPO + "/targets/tractor"
HARNESS = REPO + "/target/debug/harness"

def unit_for(case):
    root = f"{SUITE}/cases/{case['path']}"
    plan = f"{root}/migration/plan.toml"
    if not os.path.exists(plan): return root, None
    p = tomllib.load(open(plan, "rb"))
    for u in p.get("unit", []):
        if case["symbol"] in u.get("symbols", []): return root, u
    return root, None

def run(stage, model, case, extra):
    root, unit = unit_for(case)
    if unit is None: return case["path"], "no-unit", "", []
    cmd = [HARNESS, stage, unit["id"], "--target", root, "--model", model] + extra
    p = subprocess.run(cmd, capture_output=True, text=True)
    text = p.stdout + p.stderr
    if "awaiting response" in text: state = "awaiting"
    elif p.returncode == 0: state = "green"
    elif p.returncode == 10: state = "red"
    else: state = "error"
    last = [l for l in text.splitlines() if l.strip()][-1:] or [""]
    awaited = [l.split("awaiting response: ",1)[1].strip() for l in text.splitlines() if "awaiting response: " in l]
    return case["path"], state, last[0][:220], awaited

def main():
    args = sys.argv[1:]
    stage, model = args[0], args[1]; rest = args[2:]
    jobs = 6; only = None; extra = []
    if "--jobs" in rest: i = rest.index("--jobs"); jobs = int(rest[i+1]); del rest[i:i+2]
    if "--only" in rest: i = rest.index("--only"); only = set(rest[i+1].split(",")); del rest[i:i+2]
    if "--extra" in rest: i = rest.index("--extra"); extra = rest[i+1:]; del rest[i:]
    suite = tomllib.load(open(SUITE + "/suite.toml", "rb"))
    cases = [c for c in suite["case"] if not only or c["path"].split("/")[-1] in only or c["path"] in only]
    results = []
    with cf.ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(lambda c: run(stage, model, c, extra), cases): results.append(r)
    counts = {}
    awaited = [a for r in results for a in r[3]]
    open(f"{os.environ['HANDOFF_ROOT']}/awaited-{stage}.txt", "w").write("\n".join(awaited) + ("\n" if awaited else ""))
    for path, state, last, _ in results:
        counts[state] = counts.get(state, 0) + 1
        if state not in ("green", "awaiting"): print(f"{state:8} {path}: {last}")
    print("SUMMARY", stage, model, json.dumps(counts))
    json.dump(results, open(f"{os.environ['HANDOFF_ROOT']}/stage-{stage}-last.json", "w"), indent=1)

main()
