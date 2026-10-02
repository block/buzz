import assert from "node:assert/strict";
import test from "node:test";
import { resolveProfileActivityAgent } from "./profileActivityAgent.ts";

const input = {
  effectivePubkey: "a".repeat(64),
  isBot: true,
  managedAgent: undefined,
  profile: { displayName: "Scout" },
  relayAgent: undefined,
  viewerIsOwner: true,
};

for (const [status, expected] of [
  ["unknown", "unknown"],
  ["online", "deployed"],
  ["away", "deployed"],
  ["offline", "stopped"],
]) {
  test(`profile activity preserves relay ${status} evidence`, () => {
    const agent = resolveProfileActivityAgent({
      ...input,
      relayAgent: { status, name: "Relay Scout" },
    });
    assert.equal(agent.status, expected);
    assert.equal(agent.pubkey, input.effectivePubkey);
  });
}

test("profile activity cannot infer liveness from ownership alone", () => {
  assert.equal(resolveProfileActivityAgent(input).status, "unknown");
  assert.equal(
    resolveProfileActivityAgent({ ...input, viewerIsOwner: false }),
    null,
  );
});

test("local managed runtime status still takes precedence", () => {
  assert.equal(
    resolveProfileActivityAgent({
      ...input,
      managedAgent: {
        pubkey: input.effectivePubkey,
        name: "Local Scout",
        status: "running",
      },
      relayAgent: { status: "unknown" },
    }).status,
    "running",
  );
});
