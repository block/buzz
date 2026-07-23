import assert from "node:assert/strict";
import test from "node:test";

import {
  agentDefinitionConfigScope,
  resolveAgentDialogGlobalConfig,
} from "./useAgentDialogDefaults.ts";

const persistedConfig = {
  env_vars: { SHARED: "kept" },
  provider: "relay-mesh",
  model: "auto",
  preferred_runtime: "buzz-agent",
};

test("create-mode defaults mask values owned by a hidden harness", () => {
  assert.deepEqual(
    resolveAgentDialogGlobalConfig(persistedConfig, "implicit", ["buzz-agent"]),
    {
      env_vars: { SHARED: "kept" },
      provider: null,
      model: null,
      preferred_runtime: null,
    },
  );
});

test("existing edit defaults preserve values owned by a hidden harness", () => {
  assert.equal(
    resolveAgentDialogGlobalConfig(persistedConfig, "existing", ["buzz-agent"]),
    persistedConfig,
  );
});

test("definition dialogs select config scope from create versus edit values", () => {
  assert.equal(agentDefinitionConfigScope(null), "implicit");
  assert.equal(agentDefinitionConfigScope({ displayName: "New" }), "implicit");
  assert.equal(
    agentDefinitionConfigScope({ id: "existing", displayName: "Existing" }),
    "existing",
  );
});
