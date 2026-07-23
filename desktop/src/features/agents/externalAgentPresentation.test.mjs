import assert from "node:assert/strict";
import test from "node:test";

import { applyExternalAgentPresentations } from "./externalAgentPresentation.ts";

const alice = {
  pubkey: "A".repeat(64),
  name: "Alice",
  avatarUrl: null,
  ownerPubkey: "b".repeat(64),
  agentType: "hermes",
  channels: ["identity"],
  channelIds: ["identity-id"],
  capabilities: [],
  status: "online",
  respondTo: "anyone",
  respondToAllowlist: [],
};

test("applies owner presentation name and avatar without changing runtime data", () => {
  const [presented] = applyExternalAgentPresentations([alice], {
    ["a".repeat(64)]: {
      displayName: "ALICE",
      avatarUrl: "https://example.com/alice.png",
    },
  });

  assert.equal(presented.name, "ALICE");
  assert.equal(presented.avatarUrl, "https://example.com/alice.png");
  assert.equal(presented.agentType, "hermes");
  assert.deepEqual(presented.channels, ["identity"]);
  assert.equal(presented.ownerPubkey, alice.ownerPubkey);
});

test("leaves agents unchanged when no presentation exists", () => {
  const [presented] = applyExternalAgentPresentations([alice], {});
  assert.equal(presented, alice);
});
