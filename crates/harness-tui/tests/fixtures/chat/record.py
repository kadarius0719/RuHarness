#!/usr/bin/env python3
"""The chat pane's recordings (docs/CHAT-PANE-DESIGN.md §9, Build D step 1): Claude Code with the design's exact argv (CHAT-PANE-DESIGN §1.1) and
harness-mcp --cockpit, driven by a mini cockpit (this script) that answers every
can_use_tool: reads allowed, acts run by the "cockpit" (the harness CLI here) and answered
deny + outcome. Scenarios: round (a full hand-off round), stop (interrupt while a request is
held), decline (the person declines), stream-stop (interrupt while text streams),
fold (a second message typed while the first turn streams)."""
import json, os, subprocess, sys, tempfile, threading, time, queue, signal, uuid, secrets, shutil

SCEN = sys.argv[1]
S = os.environ.get("RECORD_DIR") or tempfile.mkdtemp(prefix="chat-rec-")
REPO = os.path.realpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../../.."))
HARNESS = f"{REPO}/target/debug/harness"
MCPBIN = f"{REPO}/target/debug/harness-mcp"
TARGET = os.path.join(S, f"zopfli-{SCEN}")
UNIT = "u001-katajainen"
if os.path.exists(TARGET):
    shutil.rmtree(TARGET)
shutil.copytree(f"{REPO}/targets/zopfli", TARGET, symlinks=True)
ud = os.path.join(TARGET, "migration/units", UNIT)
for d in ("attempts", "traces", "katajainen_rs"):
    shutil.rmtree(os.path.join(ud, d), ignore_errors=True)
for f in ("oracle-latest.json", "oracle-latest.md", "oracle-last-green.json"):
    os.remove(os.path.join(ud, f))
pp = os.path.join(TARGET, "migration/plan.toml")
plan = open(pp).read().replace('id = "u001-katajainen"\nstatus = "verified"', 'id = "u001-katajainen"\nstatus = "pending"', 1)
open(pp, "w").write(plan)
for cmd in ("scan", "plan"):
    subprocess.run([HARNESS, cmd, "--target", TARGET], check=True, capture_output=True)
TARGET = os.path.realpath(TARGET)
MCP = {"mcpServers": {"harness": {"command": MCPBIN, "args": ["--cockpit", "--target", TARGET]}}}
cwd = os.path.join(tempfile.gettempdir(), f"harness-tui-chat-{os.getpid()}-{secrets.token_hex(4)}")
os.mkdir(cwd, 0o700)
sock = os.path.join(cwd, "inbox.sock")
env = {k: os.environ[k] for k in ("HOME", "PATH", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG") if k in os.environ}
env.update({k: v for k, v in os.environ.items() if k.startswith("LC_")})
env["TERM"] = "dumb"
brief = open(f"{REPO}/crates/harness-tui/src/chat_brief.md").read()
argv = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-partial-messages", "--permission-prompt-tool", "stdio", "--permission-mode", "default",
        "--tools", "", "--restricted", "--setting-sources", "", "--disable-slash-commands",
        "--strict-mcp-config", "--mcp-config", json.dumps(MCP), "--no-session-persistence",
        "--messaging-socket-path", sock, "--append-system-prompt", brief, "--model", "haiku"]
log = open(os.path.join(S, f"log-{SCEN}.jsonl"), "w")
p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.PIPE, text=True, bufsize=1, process_group=0)
q = queue.Queue()
def reader(pipe, tag):
    for line in pipe:
        q.put((tag, line.rstrip("\n")))
    q.put((tag, None))
threading.Thread(target=reader, args=(p.stdout, "out"), daemon=True).start()
threading.Thread(target=reader, args=(p.stderr, "err"), daemon=True).start()
t0 = time.time()
def L(**kw):
    kw["t"] = round(time.time() - t0, 2); log.write(json.dumps(kw) + "\n"); log.flush()
def send(obj):
    L(dir="in", msg=obj); p.stdin.write(json.dumps(obj) + "\n"); p.stdin.flush()
def user(text):
    ctx = {"type": "text", "text": "[cockpit context] About: the unit u001-katajainen (planned)."}
    send({"type": "user", "message": {"role": "user", "content": [ctx, {"type": "text", "text": text}]},
          "parent_tool_use_id": None, "session_id": "", "uuid": str(uuid.uuid4())})
def answer(rid, allow, inp=None, message=None):
    r = {"behavior": "allow", "updatedInput": inp} if allow else {"behavior": "deny", "message": message}
    send({"type": "control_response", "response": {"subtype": "success", "request_id": rid, "response": r}})
def run_cli(args, stdin=None):
    argv = [HARNESS, "--json"] + args
    L(dir="cockpit-run", argv=argv)
    r = subprocess.run(argv, input=stdin, capture_output=True, text=True)
    evs = [json.loads(l) for l in r.stdout.splitlines() if l.startswith("{")]
    L(dir="cockpit-exit", code=r.returncode, stderr=r.stderr[-2000:])
    return r.returncode, evs
def outcome(act, code, evs):
    aw = next((e for e in evs if e.get("k") == "awaiting"), None)
    att = next((e for e in evs if e.get("k") == "attempt"), None)
    err = next((e for e in evs if e.get("k") == "error"), None)
    checks = [e for e in evs if e.get("k") == "check" and not e.get("passed")]
    o = "awaiting" if aw else ("green" if att and att.get("outcome") == "green" else
        "red" if att else "refused" if err else "done" if code == 0 else "failed")
    body = {"act": act, "outcome": o, "exit": code,
            "attempt": (aw or att or {}).get("attempt") or (att or {}).get("id"),
            "awaiting": {"attempt": aw.get("attempt"), "request_key": aw.get("request_key")} if aw else None,
            "failed_checks": [{"untrusted": "check", "text": c.get("name", "")} for c in checks][:8],
            "messages": [{"untrusted": "message", "text": e.get("text", "")} for e in evs if e.get("k") == "message"][-6:],
            "omitted": None}
    return "Ran by the cockpit after the person confirmed it; the outcome: " + json.dumps(body)
model = None
held = {}
runs = 0
send({"type": "control_request", "request_id": "req_init_" + secrets.token_hex(4),
      "request": {"subtype": "initialize", "hooks": None}})
user({"round": "Please migrate this unit.", "stop": "Please migrate this unit.",
      "decline": "Please migrate this unit.",
      "stream-stop": "Write three short paragraphs about why C to Rust migration is hard.",
      "fold": "Write three short paragraphs about why C to Rust migration is hard."}[SCEN])
followup = {"stop": "Never mind. What is the unit's status?", "stream-stop": "Just one line, please."}.get(SCEN)
stopped = False
deadline = time.time() + 900
while time.time() < deadline:
    try:
        tag, line = q.get(timeout=0.5)
    except queue.Empty:
        continue
    if line is None:
        if tag == "out": break
        continue
    if tag == "err":
        L(dir="stderr", line=line[:2000]); continue
    try:
        msg = json.loads(line)
    except Exception:
        L(dir="raw", line=line[:2000]); continue
    L(dir="out", msg=msg)
    t = msg.get("type")
    if t == "system" and msg.get("subtype") == "init":
        model = msg.get("model")
    if t == "assistant":
        model = msg.get("message", {}).get("model") or model
    if SCEN == "fold" and not stopped and t == "stream_event" and \
            msg.get("event", {}).get("type") == "content_block_delta":
        stopped = True
        user("Also end with a one-line summary.")
    if SCEN == "stream-stop" and not stopped and t == "stream_event" and \
            msg.get("event", {}).get("type") == "content_block_delta":
        stopped = True
        send({"type": "control_request", "request_id": "req_" + secrets.token_hex(4), "request": {"subtype": "interrupt"}})
    if t == "control_request":
        req = msg["request"]; rid = msg["request_id"]
        if req.get("subtype") != "can_use_tool":
            send({"type": "control_response", "response": {"subtype": "error", "request_id": rid, "error": "not supported"}})
            continue
        name = req.get("tool_name", ""); inp = req.get("input", {})
        short = name.removeprefix("mcp__harness__")
        if short in ("harness_status", "harness_unit", "harness_request"):
            answer(rid, True, inp); continue
        if SCEN == "stop" and not stopped:
            stopped = True
            time.sleep(1.5)
            send({"type": "control_request", "request_id": "req_" + secrets.token_hex(4), "request": {"subtype": "interrupt"}})
            continue
        if SCEN == "decline":
            answer(rid, False, message="declined by the person"); continue
        if short == "harness_migrate" and runs < 1:
            runs += 1
            code, evs = run_cli(["migrate", UNIT, f"--target={TARGET}", "--no-promote", "--provider=external",
                                 f"--model={model}", "--requester=chat", "--allow-unsandboxed"])
            held[inp.get("unit")] = [a for a in evs if a.get("k") == "awaiting"]
            answer(rid, False, message=outcome("migrate", code, evs)); continue
        if short == "harness_answer" and runs < 4:
            runs += 1
            text = inp.get("text", "")
            args = ["migrate", UNIT, f"--target={TARGET}", "--no-promote", "--provider=external", f"--model={model}",
                    "--requester=chat", "--allow-unsandboxed", "--answer=-", f"--answer-bytes={len(text.encode())}",
                    f"--answer-key={inp.get('request_key')}"]
            code, evs = run_cli(args, stdin=text)
            answer(rid, False, message=outcome("continue", code, evs).replace("Ran by the cockpit after the person confirmed it",
                   "Continued by the cockpit under the permission the person gave when they ran the migration")); continue
        answer(rid, False, message="declined by the person")
    if t == "result":
        if followup:
            user(followup); followup = None
        else:
            p.stdin.close()
try:
    rc = p.wait(timeout=20)
except subprocess.TimeoutExpired:
    os.killpg(p.pid, signal.SIGTERM); rc = "term"
L(dir="exit", rc=rc, socket_left=os.path.exists(sock), cwd_left=os.listdir(cwd))
shutil.rmtree(cwd, ignore_errors=True)
print("exit", rc, file=sys.stderr)
