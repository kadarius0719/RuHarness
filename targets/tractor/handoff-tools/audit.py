#!/usr/bin/env python3
"""Mechanical blindness audit of an answering subagent's transcript (JSONL).
Never prints message text. Classifies every tool call:
- ok:        Read/Write/Edit whose path is inside the batch dir;
- deviation: any other tool whose every absolute path is inside the batch dir
             (recorded verbatim; blindness preserved);
- breach:    anything referencing a path outside the batch dir, or a
             pathless non-file tool call (blocks import)."""
import json, re, sys
transcript, batch = sys.argv[1], sys.argv[2].rstrip("/") + "/"
calls, deviations, breaches = 0, [], []
for line in open(transcript):
    try: ev = json.loads(line)
    except Exception: continue
    msg = ev.get("message") or {}
    content = msg.get("content") if isinstance(msg, dict) else None
    if not isinstance(content, list): continue
    for block in content:
        if not isinstance(block, dict) or block.get("type") != "tool_use": continue
        calls += 1
        name = block.get("name"); inp = block.get("input") or {}
        path = inp.get("file_path") or inp.get("path")
        if name in ("Read", "Write", "Edit") and isinstance(path, str) and path.startswith(batch):
            continue
        text = json.dumps(inp)
        paths = re.findall(r'(/[^\s"\'\\]+)', text)
        inside = paths and all(p.startswith(batch.rstrip("/")) for p in paths)
        entry = f"{name} {text[:400]}"
        (deviations if inside else breaches).append(entry)
print(json.dumps({"tool_calls": calls, "deviations": deviations, "violations": breaches}))
