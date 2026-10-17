"""Drives `asides mcp` the way a coding agent does, for scripts/e2e.sh, and starts agent sessions
with `asides claude`, `codex` and `opencode` (fake agents that record how they were started).

Usage: e2e-mcp.py <asides CLI> <repo> <fixtures dir> <config.toml> <listeners dir> <another repo>
"""

import base64
import glob
import http.server
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

cli, repo, fixtures, config_path, listeners, other = sys.argv[1:7]
sessions_dir = os.path.join(os.path.dirname(listeners), "sessions")
repo_name = os.path.basename(repo)


def passed(what):
    print(f"  \033[32m✓\033[0m {what}", flush=True)


def failed(what, detail=""):
    print(f"  \033[31m✗\033[0m {what}", flush=True)
    if detail:
        print(f"--- {detail}", flush=True)
    sys.exit(1)


def check(what, ok, detail=""):
    passed(what) if ok else failed(what, detail)


class Client:
    def __init__(self, name="e2e-agent"):
        self.name = name
        self.proc = subprocess.Popen(
            [cli, "-C", repo, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
        )
        self.messages = queue.Queue()
        self.last_id = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.messages.put(json.loads(line))

    def send(self, message):
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", **message}) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params=None):
        self.last_id += 1
        self.send({"id": self.last_id, "method": method, **({"params": params} if params is not None else {})})
        return self.last_id

    def response(self, request, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                message = self.messages.get(timeout=max(0.05, deadline - time.time()))
            except queue.Empty:
                break
            if message.get("id") == request:
                return message
        return None

    def call(self, method, params=None, timeout=10):
        message = self.response(self.request(method, params), timeout)
        if message is None:
            failed(f"{method} answered")
        return message.get("result", message.get("error"))


def text(result):
    return "\n".join(c.get("text", "") for c in result["content"] if c["type"] == "text")


def tool(name, **arguments):
    return {"name": name, "arguments": arguments}


def simulate(wav, *args):
    subprocess.run([cli, "-C", repo, "simulate", os.path.join(fixtures, wav), *args], check=True, capture_output=True)


def status():
    return subprocess.run([cli, "-C", repo, "status"], check=True, capture_output=True, text=True).stdout


def session_of(pid):
    for path in glob.glob(os.path.join(sessions_dir, "*.json")):
        with open(path) as f:
            record = json.load(f)
        if record.get("pid") == pid:
            return record
    return None


def wait_until(what, test, timeout=10):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if test():
            return
        time.sleep(0.3)
    failed(what, status())


def connect(client):
    init = client.call("initialize", {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {"name": client.name, "version": "1"},
    })
    client.send({"method": "notifications/initialized"})
    return init


def entries():
    note = subprocess.run([cli, "-C", repo, "path"], check=True, capture_output=True, text=True).stdout.strip()
    with open(note) as f:
        return sum(1 for line in f if line.startswith("## ") and line[3:4].isdigit())


def wait_for_entries(count):
    for _ in range(240):
        if entries() >= count:
            return
        time.sleep(0.5)
    failed(f"entry {count} written")


client = Client()
init = connect(client)
check("initialize: server info and protocol version",
      init["serverInfo"]["name"] == "asides" and init["protocolVersion"] == "2025-06-18", init)
mine = session_of(client.proc.pid)
check("the agent's session is recorded, not listening yet", mine and not mine["listening"], mine)
check("its instructions name its listen command",
      f"listen --session {mine['id']}" in init["instructions"] and "bind_asides" not in init["instructions"],
      init["instructions"])
tools = {t["name"]: t for t in client.call("tools/list")["tools"]}
expected = ["acknowledge_aside", "dismiss_aside", "get_asides", "get_pending_asides",
            "list_asides_notes", "measure_image", "reply_to_aside", "resolve_aside", "wait_for_asides"]
check("tools: listening, reading, the feedback loop, measuring", sorted(tools) == expected, sorted(tools))
check("reading tools are read-only, the feedback loop's aren't",
      all(tools[n]["annotations"]["readOnlyHint"] for n in ["get_asides", "get_pending_asides", "measure_image"])
      and not tools["resolve_aside"]["annotations"]["readOnlyHint"])
names = sorted(p["name"] for p in client.call("prompts/list")["prompts"])
check("prompts: asides, listen, open", names == ["asides", "listen", "open"], names)
note = text(client.call("tools/call", tool("get_asides")))
check("get_asides reads the latest note with its metadata", 'Note "' in note and "branch:" in note and "## 1 ·" in note, note)
prompt = client.call("prompts/get", {"name": "asides"})["messages"][0]["content"]["text"]
check("the asides prompt carries the note", prompt.startswith("Work through my asides") and "## 1 ·" in prompt, prompt)

# The feedback loop: the agent acts on the marks of a Measure & Annotate entry (#4.1, #4.2, #4.3).
pending = text(client.call("tools/call", tool("get_pending_asides")))
check("get_pending_asides lists the Measure & Annotate entry", "## 4 · " in pending and "### 3. Arrow from" in pending,
      pending)
resolved = text(client.call("tools/call", tool("resolve_aside", aside="4.2", summary="Removed the empty space")))
check("resolve_aside on one mark", resolved.startswith("Resolved #4.2"), resolved)
dismissed = text(client.call("tools/call", tool("dismiss_aside", aside="4.3", reason="It's already there")))
check("dismiss_aside on another", dismissed.startswith("Dismissed #4.3"), dismissed)
replied = text(client.call("tools/call", tool("reply_to_aside", aside="4.1", message="Left or right edge?")))
check("reply_to_aside on the last one", replied.startswith("Replied to #4.1"), replied)
check("an aside that doesn't exist is refused",
      client.call("tools/call", tool("dismiss_aside", aside="4.9", reason="x"))["isError"])
pending = text(client.call("tools/call", tool("get_pending_asides")))
check("pending asides show the agent's question, the entry open while a mark is",
      "> **e2e-agent**: Left or right edge?" in pending and "## 4 · " in pending, pending)
note_path = subprocess.run([cli, "-C", repo, "path"], check=True, capture_output=True, text=True).stdout.strip()
with open(os.path.join(os.path.dirname(note_path), "threads.jsonl")) as f:
    events = [json.loads(line) for line in f]
check("threads.jsonl records what the agent did",
      [(e["aside"], e.get("status")) for e in events][:3] == [("4.2", "resolved"), ("4.3", "dismissed"), ("4.1", None)],
      events)
notes = text(client.call("tools/call", tool("list_asides_notes")))
check("list_asides_notes", "Untitled" in notes and "open" in notes, notes)
measured = text(client.call("tools/call", tool("measure_image", path=os.environ["E2E_SHOT"], x=200, y=280, scale=2)))
check("measure_image: distances between edges in a screenshot", "640 px (320 pt at @2x)" in measured, measured)
measured = text(client.call("tools/call", tool("measure_image", path=os.environ["E2E_SHOT"],
                                              area={"x": 100, "y": 320, "width": 240, "height": 100})))
check("measure_image: the object inside an area", "(120, 340), 200 px wide × 48 px high" in measured, measured)

# Bound elsewhere: listening makes this agent's session the one the user's asides go to, and binds
# the app to its repo. No binding step.
subprocess.run([cli, "-C", other], check=True, capture_output=True)
first = text(client.call("tools/call", tool("wait_for_asides", timeout_seconds=2), timeout=30))
check("the first wait listens", "No new asides yet" in first, first)
now = status()
check("listening binds the app to the agent's repo", re.search(rf"bound to .*/{re.escape(repo_name)}$", now, re.M), now)
check("and sends the user's asides to its session", f"asides go to e2e-agent · {repo_name}" in now, now)
check("its record says it listens", session_of(client.proc.pid)["listening"])

# Each aside, as soon as it's transcribed.
before = entries()
waiting = client.request("tools/call", tool("wait_for_asides", timeout_seconds=180))
time.sleep(1.5)
record = os.path.join(listeners, f"{client.proc.pid}.json")
with open(record) as f:
    listener = json.load(f)
check("listening shows in the app's listener records",
      listener["waiting"] and listener["client"] == "e2e-agent" and listener["session"] == mine["id"], listener)
simulate("en_bug.wav")
answer = client.response(waiting, 180)
result = answer and answer["result"]
check("a new recording reaches the waiting agent", result and "padding on the summary card" in text(result),
      result and text(result))
check("as one new aside", text(result).startswith("A new aside from the user."), text(result))
check("nothing is delivered twice",
      "No new asides yet" in text(client.call("tools/call", tool("wait_for_asides", timeout_seconds=2), timeout=10)))
note_dir = os.path.dirname(subprocess.run([cli, "-C", repo, "path"], check=True, capture_output=True, text=True).stdout.strip())


def routes():
    with open(os.path.join(note_dir, "routes.jsonl")) as f:
        return [json.loads(line) for line in f]


check("the entry's route names the session", routes()[-1]["session"] == mine["id"], routes())

# A second agent in the same repo: once it listens, the asides go to it, and only to it.
second = Client("second-agent")
connect(second)
theirs = session_of(second.proc.pid)
waiting_second = second.request("tools/call", tool("wait_for_asides", timeout_seconds=120))
wait_until("the second session becomes the target", lambda: f"asides go to second-agent · {repo_name}" in status(), 20)
waiting_first = client.request("tools/call", tool("wait_for_asides", timeout_seconds=25))
before = entries()
simulate("en_bug.wav")
answer = second.response(waiting_second, 180)
check("the aside reaches the session it was spoken to", answer and "padding on the summary card" in text(answer["result"]),
      answer)
answer = client.response(waiting_first, 40)
result = answer and text(answer["result"])
check("not the other one, which is told where asides go",
      result and "No new asides yet" in result and "another session" in result, result)
check("the route says which", routes()[-1]["session"] == theirs["id"], routes())
got = text(client.call("tools/call", tool("get_asides", after_entry=before), timeout=10))
check("reading the note, the other session sees who it was for", "said this to another session (second-agent)" in got, got)
second.proc.stdin.close()
second.proc.wait(timeout=10)
wait_until("when it hangs up, the asides go back to the first session",
           lambda: f"asides go to e2e-agent · {repo_name}" in status(), 15)
check("and its record is gone", session_of(second.proc.pid) is None)

# Batches: held until Send to Agent.
with open(config_path) as f:
    config = re.sub(r"(?m)^agent_delivery = .*\n", "", f.read())
with open(config_path, "w") as f:
    f.write('agent_delivery = "batch"\n' + config)
simulate("ro_bug.wav")
wait_for_entries(before + 2)
held = text(client.call("tools/call", tool("wait_for_asides", timeout_seconds=3), timeout=10))
check("in batch mode, a transcribed aside waits", "No new asides yet" in held and "Send to Agent" in held, held)
waiting = client.request("tools/call", tool("wait_for_asides", timeout_seconds=60))
sent = subprocess.run([cli, "-C", repo, "send"], check=True, capture_output=True, text=True).stdout
check("asides send", sent.startswith("Sent"), sent)
answer = client.response(waiting, 30)
result = answer and answer["result"]
check("Send to Agent delivers the batch", result and "Butonul de salvare" in text(result), result and text(result))

# Cancelling a wait, then hanging up.
waiting = client.request("tools/call", tool("wait_for_asides", timeout_seconds=60))
client.send({"method": "notifications/cancelled", "params": {"requestId": waiting, "reason": "user"}})
check("a cancelled wait gets no answer", client.response(waiting, 2) is None)
check("and the server keeps serving", client.call("ping") == {})
client.proc.stdin.close()
client.proc.wait(timeout=10)
check("hanging up removes the listener record", not os.path.exists(record))
check("and the session's", session_of(client.proc.pid) is None)

# asides listen: the same asides on standard output, for agents that watch a command in the background.
env = {k: v for k, v in os.environ.items() if k != "CLAUDECODE"}
env["AI_AGENT"] = "codex"
listen = subprocess.Popen([cli, "-C", repo, "listen"], stdout=subprocess.PIPE, text=True, bufsize=1, env=env)
lines = queue.Queue()
threading.Thread(target=lambda: [lines.put(line) for line in listen.stdout], daemon=True).start()


def heard(needle, timeout):
    seen = ""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            seen += lines.get(timeout=max(0.05, deadline - time.time()))
        except queue.Empty:
            break
        if needle in seen:
            return seen
    return seen


started = heard("Listening for asides in", 10)
check("asides listen says it's listening, and how asides arrive", "in batches" in started, started)
time.sleep(1)
with open(os.path.join(listeners, f"{listen.pid}.json")) as f:
    listener = json.load(f)
check("it shows as a listener, named after the agent running it",
      listener["waiting"] and listener["client"] == "Codex", listener)
wait_until("listening makes its session the target", lambda: f"asides go to Codex · {repo_name}" in status(), 15)
before = entries()
simulate("en_bug.wav")
wait_for_entries(before + 1)
time.sleep(2.5)  # the app reads the listener records every 2 s
sent = subprocess.run([cli, "-C", repo, "send"], check=True, capture_output=True, text=True).stdout
check("asides send reaches it", sent.startswith("Sent"), sent)
batch = heard("padding on the summary card", 30)
check("the batch is printed as one block", "A new aside from the user." in batch and "## " in batch, batch)
listen.terminate()
listen.wait(timeout=10)
check("stopping it removes its listener record", not os.path.exists(os.path.join(listeners, f"{listen.pid}.json")))

# Nobody listening: nothing is sent, and the user is told.
time.sleep(2.5)
nobody = subprocess.run([cli, "-C", repo, "send"], capture_output=True, text=True)
check("with no agent listening, Send to Agent says so and sends nothing",
      nobody.returncode != 0 and "No agent is listening" in nobody.stderr, nobody.stdout + nobody.stderr)

# asides claude, codex, opencode: sessions started from the command line. Fake agents record their
# arguments, environment and directory, then wait while FAKE_SLEEP says.
fakes = tempfile.mkdtemp()
for name in ["claude", "codex", "opencode"]:
    with open(os.path.join(fakes, name), "w") as f:
        f.write(f"""#!/bin/sh
[ "$1" = --version ] && echo "{name} 2.0.0" && exit 0
[ "$1" = serve ] && exit 1
printf '%s\\0' "$@" > "{fakes}/{name}.args"
env | grep -E '^(ASIDES_|OPENCODE_CONFIG_CONTENT)' > "{fakes}/{name}.env"
pwd -P > "{fakes}/{name}.pwd"
sleep "${{FAKE_SLEEP:-0}}"
""")
    os.chmod(os.path.join(fakes, name), 0o755)
# Codex also has a daemon, reached through `codex app-server proxy` (JSON-RPC over WebSocket), and
# `codex queue`: the fake records the requests and the queued messages.
with open(os.path.join(fakes, "codex"), "w") as f:
    f.write(f"""#!/usr/bin/python3 -I
import json, os, struct, sys, time
fakes = {fakes!r}
args = sys.argv[1:]
def record(name, value):
    with open(os.path.join(fakes, name), "a") as f:
        f.write(json.dumps(value) + "\\n")
if args[:1] == ["--version"]:
    print("codex-cli 0.163.0")
elif args[:2] == ["app-server", "proxy"]:
    if os.environ.get("FAKE_NO_DAEMON"):
        sys.exit(1)
    inp, out = sys.stdin.buffer, sys.stdout.buffer
    while inp.readline() not in (b"\\r\\n", b""):
        pass
    out.write(b"HTTP/1.1 101 Switching Protocols\\r\\nupgrade: websocket\\r\\nconnection: Upgrade\\r\\n\\r\\n")
    out.flush()
    while True:
        head = inp.read(2)
        if len(head) < 2 or head[0] & 0x0f == 8:
            break
        n = head[1] & 0x7f
        if n == 126:
            n = struct.unpack(">H", inp.read(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", inp.read(8))[0]
        mask = inp.read(4)
        message = json.loads(bytes(b ^ mask[i % 4] for i, b in enumerate(inp.read(n))))
        record("codex-daemon.jsonl", message)
        method = message.get("method")
        if "id" not in message:
            continue
        if method == "thread/start":
            count = sum(1 for _ in open(os.path.join(fakes, "codex-daemon.jsonl"))
                        if '"thread/start"' in _)
            result = {{"thread": {{"id": f"fake-thread-{{count}}"}}}}
        else:
            result = {{}}
        data = json.dumps({{"jsonrpc": "2.0", "id": message["id"], "result": result}}).encode()
        out.write(bytes([0x81]) + (bytes([len(data)]) if len(data) < 126 else bytes([126]) + struct.pack(">H", len(data))) + data)
        out.flush()
elif args[:1] == ["queue"]:
    record("codex-queue.jsonl", {{"thread": args[args.index("--thread") + 1], "message": args[args.index("--message") + 1]}})
else:
    with open(os.path.join(fakes, "codex.args"), "w") as f:
        f.write("\\0".join(args))
    with open(os.path.join(fakes, "codex.env"), "w") as f:
        f.write("\\n".join(f"{{k}}={{v}}" for k, v in os.environ.items() if k.startswith("ASIDES_")))
    with open(os.path.join(fakes, "codex.pwd"), "w") as f:
        f.write(os.path.realpath(os.getcwd()))
    time.sleep(float(os.environ.get("FAKE_SLEEP", "0")))
""")


def daemon_calls(method):
    with open(os.path.join(fakes, "codex-daemon.jsonl")) as f:
        return [m.get("params", {}) for m in map(json.loads, f) if m.get("method") == method]


def queued():
    try:
        with open(os.path.join(fakes, "codex-queue.jsonl")) as f:
            return [json.loads(line) for line in f]
    except FileNotFoundError:
        return []


# OpenCode 2 runs sessions in a background service, found through its registration in
# $XDG_STATE_HOME/opencode. Agents start with an empty one (no service: the real one stays out of
# this); the fake service records requests and answers as OpenCode's does.
no_state = tempfile.mkdtemp()


class FakeOpenCode(http.server.BaseHTTPRequestHandler):
    calls = []
    sessions = {}
    auth = "Basic " + base64.b64encode(b"opencode:fake-password").decode()

    def log_message(self, *args):
        pass

    def answer(self):
        if self.headers.get("Authorization") != FakeOpenCode.auth:
            return self.reply(401, {"message": "Authentication required"})
        url = urllib.parse.urlsplit(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length)) if length else None
        FakeOpenCode.calls.append({"method": self.command, "path": url.path,
                                   "query": dict(urllib.parse.parse_qsl(url.query)), "body": body})
        if self.command == "POST" and url.path == "/api/session":
            session = f"ses_fake_{len(FakeOpenCode.sessions) + 1}"
            FakeOpenCode.sessions[session] = body["location"]
            return self.reply(200, {"data": {"id": session, "location": body["location"]}})
        if self.command == "GET" and url.path.startswith("/api/session/"):
            session = url.path.split("/")[3]
            return self.reply(200, {"data": {"id": session, "location": FakeOpenCode.sessions.get(session)}})
        if self.command == "GET" and url.path == "/api/config":
            mine = {"type": "local", "command": ["asides", "mcp"]}
            return self.reply(200, [{"type": "document", "info": {"mcp": {"servers": {"asides": mine}}}}])
        if self.command in ("PUT", "DELETE"):
            return self.reply(204, None)
        return self.reply(200, {"data": []})

    do_GET = do_POST = do_PUT = do_DELETE = answer

    def reply(self, status, value):
        data = b"" if value is None else json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)


def opencode_calls(method, path):
    return [c for c in FakeOpenCode.calls if c["method"] == method and c["path"] == path]


def start(agent, sleep, *args, **extra):
    env = {**os.environ, "PATH": fakes + ":" + os.environ["PATH"], "FAKE_SLEEP": str(sleep),
           "XDG_STATE_HOME": no_state, **extra}
    env.pop("CLAUDECODE", None)
    proc = subprocess.Popen([cli, "-C", repo, agent, *args], stdout=subprocess.PIPE, text=True, env=env)
    wait_until(f"asides {agent} starts the agent", lambda: os.path.exists(os.path.join(fakes, agent + ".pwd")), 20)
    with open(os.path.join(fakes, agent + ".args")) as f:
        argv = f.read().rstrip("\0").split("\0")
    with open(os.path.join(fakes, agent + ".env")) as f:
        env = dict(line.split("=", 1) for line in f.read().splitlines() if "=" in line)
    with open(os.path.join(fakes, agent + ".pwd")) as f:
        cwd = f.read().strip()
    for name in ["args", "env", "pwd"]:
        os.remove(os.path.join(fakes, f"{agent}.{name}"))
    return proc, argv, env, cwd


claude, argv, env, cwd = start("claude", 300, "--model", "opus")
session = env.get("ASIDES_SESSION", "")
check("asides claude starts Claude Code in the repo, listening as a new session",
      cwd == os.path.realpath(repo) and argv[0].startswith("Listen to my asides") and f"--session {session}" in argv[0]
      and session, (cwd, argv[:1], env))
config = json.loads(argv[argv.index("--mcp-config") + 1])
check("with asides' MCP server on its command line, for that session",
      config["mcpServers"]["asides"]["args"] == ["-C", os.path.realpath(repo), "mcp", "--session", session], config)
check("its tools pre-approved, and the user's own arguments passed on",
      "mcp__asides" in argv and argv[-2:] == ["--model", "opus"], argv)
wait_until("the app sends the user's asides to it", lambda: f"asides go to Claude Code · {repo_name}" in status(), 15)
check("it shows as starting until it listens", f"Claude Code · {repo_name}: starting" in status(), status())

# Codex: a thread in its daemon, that asides pushes each aside into.
tree = os.path.join(os.path.realpath(repo), ".asides", "worktrees")
codex, argv, env, cwd = start("codex", 120)
thread = daemon_calls("thread/start")[-1]
codex_session = thread["config"]["mcp_servers"]["asides"]["args"][-1]
check("asides codex starts a thread in Codex's daemon, with the session's MCP server",
      thread["config"]["mcp_servers"]["asides"]["args"][:4] == ["-C", os.path.realpath(repo), "mcp", "--session"]
      and env.get("ASIDES_SESSION") == codex_session, (thread, env))
check("a second session in the repo works in a worktree asides makes",
      thread["cwd"].startswith(tree + "/asides-") and cwd == thread["cwd"], (thread["cwd"], cwd))
check("the thread is named, and its first prompt says asides come as messages",
      daemon_calls("thread/name/set")[-1]["name"].endswith(f"asides {codex_session}")
      and "as a message" in daemon_calls("turn/start")[-1]["input"][0]["text"])
check("the Codex window opens that thread, without -c (that would lock it out)", argv == ["resume", "fake-thread-1"], argv)
wait_until("it becomes the target", lambda: f"asides go to Codex · {repo_name} (asides-{codex_session})" in status(), 15)
before = entries()
simulate("en_bug.wav")
wait_for_entries(before + 1)
time.sleep(2.5)  # the app reads the session records every 2 s
sent = subprocess.run([cli, "-C", repo, "send"], capture_output=True, text=True).stdout
check("in batch mode, Send to Agent sends the batch to the Codex session", sent.startswith("Sent"), sent)
wait_until("the batch is pushed into the thread (codex queue)",
           lambda: any(q["thread"] == "fake-thread-1" and "padding on the summary card" in q["message"] for q in queued()), 120)
check("as a message that says how to resolve it", "resolve_aside" in queued()[-1]["message"])
codex.terminate()
codex.wait(timeout=10)
wait_until("closing the Codex window ends the session and its pusher",
           lambda: f"Codex · {repo_name} (asides-{codex_session})" not in status()
           and "fake-thread-1" not in subprocess.run(["ps", "-axo", "command"], capture_output=True, text=True).stdout, 15)

env = {**os.environ, "PATH": fakes + ":" + os.environ["PATH"]}
background = subprocess.run([cli, "-C", repo, "codex", "--background"], capture_output=True, text=True, env=env, timeout=60)
check("asides codex --background starts the thread and returns", "codex resume fake-thread-2" in background.stdout,
      background.stdout + background.stderr)
record = next(json.load(open(f)) for f in glob.glob(os.path.join(sessions_dir, "*.json"))
              if json.load(open(f)).get("thread") == "fake-thread-2")
check("its session lasts as long as what pushes into the thread", record["background"] and record["pushed"]
      and record["pid"] > 0, record)
wait_until("it becomes the target", lambda: f"asides go to Codex · {repo_name}" in status(), 15)
before = entries()
simulate("ro_bug.wav")
wait_for_entries(before + 1)
time.sleep(2.5)
subprocess.run([cli, "-C", repo, "send"], capture_output=True, text=True)
wait_until("asides reach the thread with no Codex window open",
           lambda: any(q["thread"] == "fake-thread-2" and "Butonul" in q["message"] for q in queued()), 120)
os.kill(record["pid"], 15)  # Stop, from the menu
wait_until("stopping it ends the session", lambda: f"asides go to Claude Code · {repo_name}" in status(), 15)

# OpenCode 2: a session in its background service, that asides pushes each aside into.
service = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeOpenCode)
threading.Thread(target=service.serve_forever, daemon=True).start()
oc_state = tempfile.mkdtemp()
os.makedirs(os.path.join(oc_state, "opencode"))
with open(os.path.join(oc_state, "opencode", "service.json"), "w") as f:
    json.dump({"id": "fake", "version": "2.0.0", "url": f"http://127.0.0.1:{service.server_address[1]}",
               "pid": os.getpid(), "password": "fake-password"}, f)
opencode, argv, env, cwd = start("opencode", 120, XDG_STATE_HOME=oc_state)
created = opencode_calls("POST", "/api/session")[-1]["body"]
server = opencode_calls("PUT", "/api/experimental/mcp/asides")[-1]
oc_session = env.get("ASIDES_SESSION")
check("asides opencode starts a session in OpenCode's service, in the worktree it works in",
      created["location"]["directory"] == cwd and cwd.startswith(tree + "/asides-")
      and created["title"].endswith(f"asides {oc_session}"), (created, cwd))
check("and gives that directory the session's MCP server",
      server["query"].get("location[directory]") == cwd
      and server["body"]["config"]["command"][1:] == ["-C", os.path.realpath(repo), "mcp", "--session", oc_session],
      server)
first = opencode_calls("POST", "/api/session/ses_fake_1/prompt")
check("its first prompt says asides come as messages", first and "as a message" in first[0]["body"]["text"], first)
check("the TUI opens that session from the service", argv == ["--session", "ses_fake_1"]
      and "OPENCODE_CONFIG_CONTENT" not in env, (argv, env))
wait_until("it becomes the target", lambda: f"asides go to OpenCode · {repo_name} (asides-{oc_session})" in status(), 15)
before = entries()
simulate("en_bug.wav", "--image", os.path.join(fixtures, "screenshot.png"), "--at", "6.5")
wait_for_entries(before + 1)
time.sleep(2.5)
sent = subprocess.run([cli, "-C", repo, "send"], capture_output=True, text=True).stdout
check("Send to Agent sends the batch to the OpenCode session", sent.startswith("Sent"), sent)
wait_until("the batch is pushed into the session as its next input",
           lambda: any("padding on the summary card" in c["body"]["text"] and c["body"]["delivery"] == "queue"
                       for c in opencode_calls("POST", "/api/session/ses_fake_1/prompt")[1:]), 120)
check("with the screenshot attached",
      any(f.get("uri", "").startswith("file:///") for c in opencode_calls("POST", "/api/session/ses_fake_1/prompt")
          for f in (c["body"].get("files") or [])), opencode_calls("POST", "/api/session/ses_fake_1/prompt")[-1])
opencode.terminate()
opencode.wait(timeout=10)
mine = {"type": "local", "command": ["asides", "mcp"]}
wait_until("closing the TUI ends the session, and puts the user's own asides server back",
           lambda: f"OpenCode · {repo_name}" not in status()
           and any(c["body"] == {"config": mine} and c["query"].get("location[directory]") == cwd
                   for c in opencode_calls("PUT", "/api/experimental/mcp/asides")), 15)
env = {**os.environ, "PATH": fakes + ":" + os.environ["PATH"], "XDG_STATE_HOME": oc_state}
background = subprocess.run([cli, "-C", repo, "opencode", "--background"], capture_output=True, text=True, env=env,
                            timeout=60)
check("asides opencode --background starts the session and returns", "opencode --session ses_fake_2" in background.stdout,
      background.stdout + background.stderr)
record = next(json.load(open(f)) for f in glob.glob(os.path.join(sessions_dir, "*.json"))
              if json.load(open(f)).get("thread") == "ses_fake_2")
check("its session lasts as long as what pushes into it", record["background"] and record["pushed"] and record["pid"] > 0,
      record)
restored = len(opencode_calls("PUT", "/api/experimental/mcp/asides"))
os.kill(record["pid"], 15)  # Stop, from the menu
wait_until("stopping it ends the session, and puts the user's server back",
           lambda: f"asides go to Claude Code · {repo_name}" in status()
           and opencode_calls("PUT", "/api/experimental/mcp/asides")[restored:], 15)
service.shutdown()

codex, argv, env, cwd = start("codex", 0, "--no-worktree", FAKE_NO_DAEMON="1")
check("without Codex's daemon, Codex waits for asides in a tool call instead",
      "To listen, call the wait_for_asides tool" in argv[-1] and any("mcp_servers.asides.args=" in a for a in argv), argv)
codex.wait(timeout=10)
opencode, argv, env, cwd = start("opencode", 0)
opencode_config = json.loads(env.get("OPENCODE_CONFIG_CONTENT", "{}"))
check("without OpenCode's service, OpenCode waits for asides, its server in its environment; asides makes its worktree",
      cwd.startswith(tree + "/asides-") and "--standalone" in argv
      and "--session" in opencode_config["mcp"]["servers"]["asides"]["command"], (cwd, argv, env))
opencode.wait(timeout=10)
wait_until("ended sessions leave the target to the one still running",
           lambda: f"asides go to Claude Code · {repo_name}" in status() and "Codex" not in status(), 15)
opencode, argv, env, cwd = start("opencode", 0, "--no-worktree")
check("--no-worktree shares the repo", cwd == os.path.realpath(repo), cwd)
check("and the next start removes the worktrees of ended sessions with nothing done in them",
      not os.listdir(tree), os.listdir(tree))
opencode.wait(timeout=10)
claude.terminate()
claude.wait(timeout=10)
wait_until("when the last session ends, asides go to the notes only", lambda: "asides go to notes only" in status(), 15)
shutil.rmtree(fakes)
