"""Drives `asides mcp` the way a coding agent does, for scripts/e2e.sh.

Usage: e2e-mcp.py <asides CLI> <repo> <fixtures dir> <config.toml> <listeners dir> <another repo>
"""

import json
import os
import queue
import re
import subprocess
import sys
import threading
import time
import urllib.request

cli, repo, fixtures, config_path, listeners, other = sys.argv[1:7]


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
    def __init__(self):
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


def simulate(wav):
    subprocess.run([cli, "-C", repo, "simulate", os.path.join(fixtures, wav)], check=True, capture_output=True)


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
init = client.call("initialize", {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "e2e-agent", "version": "1"},
})
client.send({"method": "notifications/initialized"})
check("initialize: server info and protocol version",
      init["serverInfo"]["name"] == "asides" and init["protocolVersion"] == "2025-06-18", init)
tools = {t["name"]: t for t in client.call("tools/list")["tools"]}
expected = ["acknowledge_aside", "bind_asides", "dismiss_aside", "get_asides", "get_pending_asides",
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

# Bound elsewhere: the wait says so at once, and bind_asides points the app here.
subprocess.run([cli, "-C", other], check=True, capture_output=True)
started = time.time()
elsewhere = text(client.call("tools/call", tool("wait_for_asides", timeout_seconds=60), timeout=10))
check("waiting while Asides is bound elsewhere returns at once",
      time.time() - started < 5 and "bind_asides" in elsewhere and os.path.basename(other) in elsewhere, elsewhere)
bound = text(client.call("tools/call", tool("bind_asides"), timeout=30))
check("bind_asides binds the app to the agent's repo", bound.startswith("Asides is now bound to"), bound)
status = subprocess.run([cli, "-C", repo, "status"], check=True, capture_output=True, text=True).stdout
check("as the app confirms", re.search(rf"bound to .*/{re.escape(os.path.basename(repo))}$", status, re.M), status)
check("binding again is a no-op", "already bound" in text(client.call("tools/call", tool("bind_asides"), timeout=30)))

# Each aside, as soon as it's transcribed.
before = entries()
waiting = client.request("tools/call", tool("wait_for_asides", timeout_seconds=180))
time.sleep(1.5)
record = os.path.join(listeners, f"{client.proc.pid}.json")
with open(record) as f:
    listener = json.load(f)
check("listening shows in the app's listener records", listener["waiting"] and listener["client"] == "e2e-agent", listener)
simulate("en_bug.wav")
answer = client.response(waiting, 180)
result = answer and answer["result"]
check("a new recording reaches the waiting agent", result and "padding on the summary card" in text(result),
      result and text(result))
check("as one new aside", text(result).startswith("A new aside from the user."), text(result))
check("nothing is delivered twice",
      "No new asides yet" in text(client.call("tools/call", tool("wait_for_asides", timeout_seconds=2), timeout=10)))

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
