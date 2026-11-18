# A stand-in for Resolve's MCP server (ResolveMCP) speaking MCP over stdio, for connection tests.
# A script's text picks what happens: "HANG" never answers, "DIE" exits, "ERROR" fails like a traceback;
# any other script answers {"result": {length, timeout}}. FAKE_NO_RUN_SCRIPT=1 leaves run_script out.
# The Python twin of apps/desktop/server/resolve/fixtures/fakeResolveMcp.mjs.
import json
import os
import sys

calls = 0


def send(message):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", **message}) + "\n")
    sys.stdout.flush()


def text(id, value):
    send({"id": id, "result": {"content": [{"type": "text", "text": value}]}})


while True:
    line = sys.stdin.readline()
    if not line:
        break
    message = json.loads(line)
    if "id" not in message:
        continue
    method = message.get("method")
    name = (message.get("params") or {}).get("name")
    if method == "initialize":
        send({"id": message["id"], "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "0"}}})
    elif method == "tools/list":
        tools = [{"name": "get_resolve_status"}] + ([] if os.environ.get("FAKE_NO_RUN_SCRIPT") else [{"name": "run_script"}])
        send({"id": message["id"], "result": {"tools": tools}})
    elif name == "get_resolve_status":
        calls += 1
        text(message["id"], json.dumps({"running": True, "calls": calls, "pid": os.getpid()}))
    elif name == "run_script":
        arguments = message["params"]["arguments"]
        script = arguments["script"]
        if "HANG" in script:
            continue
        if "DIE" in script:
            sys.exit(3)
        if "ERROR" in script:
            text(message["id"], json.dumps({"error": 'Traceback (most recent call last):\n  File "<script>"\nRefused: Not on this timeline.'}))
        else:
            text(message["id"], json.dumps({"result": {"length": len(script), "timeout": arguments.get("timeout")}}))
