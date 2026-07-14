import assert from "node:assert/strict";
import test from "node:test";

import { classifyAgentManagementSender } from "./agentManagementBuffer.ts";

const AGENT = "a".repeat(64);

test("buffers a draft until managed-agent ownership data resolves", () => {
  assert.equal(classifyAgentManagementSender(undefined, AGENT), "buffer");
  assert.equal(
    classifyAgentManagementSender([{ pubkey: AGENT }], AGENT),
    "accept",
  );
});

test("rejects a buffered draft when the loaded agent list does not own it", () => {
  assert.equal(classifyAgentManagementSender(undefined, AGENT), "buffer");
  assert.equal(
    classifyAgentManagementSender([{ pubkey: "b".repeat(64) }], AGENT),
    "reject",
  );
});
