"""Deterministic ACP peer for the real run CLI integration tests. No model access."""
import json
import os
import subprocess
import sys
import time

mode = os.environ.get("TASK_AGENT_MODE", "normal")
log = open(os.environ["TASK_AGENT_LOG"], "a", buffering=1)
log.write(json.dumps({"pid": os.getpid(), "relay": os.environ.get("BUZZ_RELAY_URL"),
                      "key": os.environ.get("BUZZ_PRIVATE_KEY"),
                      "gitKeyfile": subprocess.check_output(["git", "config", "nostr.keyfile"], text=True).strip()}) + "\n")
if mode.startswith("descendant"):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
    log.write(json.dumps({"descendant": child.pid}) + "\n")

options = [
    {"id": "model", "name": "Model", "category": "model", "type": "select",
     "currentValue": "old", "options": [{"value": "old", "name": "Old"}, {"value": "chosen", "name": "Chosen"}]},
    {"id": "effort", "name": "Effort", "category": "thought_level", "type": "select",
     "currentValue": "low", "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]},
    {"id": "mode", "name": "Mode", "type": "select", "currentValue": "default",
     "options": [{"value": "bypassPermissions", "name": "Bypass"}, {"value": "default", "name": "Default"}]},
]
prompt_id = None
for line in sys.stdin:
    msg = json.loads(line)
    log.write(json.dumps(msg) + "\n")
    method = msg.get("method")
    result = {}
    if method == "initialize":
        if mode in ("startup-hang", "descendant-startup"):
            time.sleep(120)
        result = {"protocolVersion": int(os.environ.get("TASK_AGENT_PROTOCOL", "2")),
                  "agentInfo": {"name": "task-test"}, "agentCapabilities": {}}
    elif method == "session/new":
        if mode == "session-hang":
            time.sleep(120)
        result = {"sessionId": "fresh-" + str(os.getpid()), "configOptions": options, "modes": {"availableModes": [{"id": "bypassPermissions"}]}}
    elif method == "session/set_config_option":
        result = {"configOptions": options}
    elif method == "session/prompt":
        prompt_id = msg["id"]
        if mode in ("turn-hang", "descendant-turn", "ignore-cancel"):
            continue
        if mode == "permission":
            print(json.dumps({"jsonrpc": "2.0", "id": "permission", "method": "session/request_permission", "params": {
                "sessionId": "fresh-" + str(os.getpid()), "toolCall": {"toolCallId": "tool-1", "title": "Test tool"},
                "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}]}}), flush=True)
            continue
        if mode == "error":
            print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": "SECRET_TASK_TEXT"}}), flush=True)
            continue
        result = {"stopReason": os.environ.get("TASK_STOP_REASON", "end_turn")}
    elif method == "session/cancel":
        if mode == "ignore-cancel":
            continue
        print(json.dumps({"jsonrpc": "2.0", "id": prompt_id, "result": {"stopReason": "end_turn"}}), flush=True)
        continue
    elif msg.get("id") == "permission":
        print(json.dumps({"jsonrpc": "2.0", "id": prompt_id, "result": {"stopReason": "end_turn"}}), flush=True)
        continue
    if "id" in msg:
        print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
