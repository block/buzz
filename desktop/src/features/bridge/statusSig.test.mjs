import test from "node:test";
import assert from "node:assert/strict";

test("agentStatusSig is order-independent and null without a snapshot", async () => {
  const { agentStatusSig } = await import("./useBridgeAgents.ts");
  assert.equal(agentStatusSig(null), null);
  const a = agentStatusSig({ agents: [{ pubkey: "B", status: "running" }, { pubkey: "a", status: "stopped" }] });
  assert.equal(a, agentStatusSig({ agents: [{ pubkey: "a", status: "stopped" }, { pubkey: "b", status: "running" }] }));
  assert.notEqual(a, agentStatusSig({ agents: [{ pubkey: "a", status: "running" }, { pubkey: "b", status: "running" }] }));
});
