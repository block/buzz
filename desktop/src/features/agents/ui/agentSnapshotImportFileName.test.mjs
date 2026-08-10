import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_SNAPSHOT_IMPORT_ACCEPT,
  isAgentSnapshotCandidateName,
} from "./PersonaCatalogDialog.tsx";

test("agent import accepts ordinary JSON and PNG filenames", () => {
  assert.equal(AGENT_SNAPSHOT_IMPORT_ACCEPT, ".json,.png");
  assert.equal(isAgentSnapshotCandidateName("agent.json"), true);
  assert.equal(isAgentSnapshotCandidateName("analyst.agent.json"), true);
  assert.equal(isAgentSnapshotCandidateName("AGENT.PNG"), true);
});

test("agent import still rejects unrelated filenames before reading them", () => {
  assert.equal(isAgentSnapshotCandidateName("agent.txt"), false);
  assert.equal(isAgentSnapshotCandidateName("agent.json.zip"), false);
});
