import json
import sys
import os

mode = os.environ["CATALOG_FIXTURE_MODE"]
options = [
    {"id": "model", "category": "model", "type": "select", "name": "Model",
     "currentValue": "model-a", "options": [
         {"value": "model-a", "name": "Model A"},
         {"value": "model-b", "name": "Model B"}]},
    {"id": "effort", "category": "thought_level", "type": "select", "name": "Effort",
     "currentValue": "medium", "options": [{"value": "medium", "name": "Medium"}]},
]
for line in sys.stdin:
    req = json.loads(line)
    if "id" not in req:
        continue
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {},
                  "agentInfo": {"name": "catalog-fixture", "version": "1"}}
    elif method == "session/new":
        result = {"sessionId": "fixture", "configOptions": options}
    elif method == "session/set_config_option":
        if mode == "reject":
            print(json.dumps({"jsonrpc": "2.0", "id": req["id"],
                              "error": {"code": -32602, "message": "model retired"}}), flush=True)
            continue
        options[0]["currentValue"] = req["params"]["value"]
        options[1]["currentValue"] = "high"
        options[1]["options"] = [{"value": "high", "name": "High"}]
        result = {} if mode == "no-options" else {"configOptions": options}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": req["id"], "result": result}), flush=True)
