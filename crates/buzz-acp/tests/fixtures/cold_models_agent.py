"""Cold-start ACP model probe fixture; no provider, relay, credentials or tools."""
import json
import sys
import time

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        # Exceeds the old ten-second production timeout.
        time.sleep(11)
        result = {"protocolVersion": 2, "agentInfo": {"name": "cold-probe"},
                  "agentCapabilities": {}}
    elif method == "session/new":
        result = {"sessionId": "cold-probe-session", "models": {
            "currentModelId": "cold-model",
            "availableModels": [{"modelId": "cold-model", "name": "Cold model"}]}}
    else:
        result = {}
    if "id" in request:
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
