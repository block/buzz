import assert from "node:assert/strict";
import test from "node:test";

import { resolveManagedAgentSendAudience } from "./managedAgentSendAudience.ts";

const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const person = "c".repeat(64);

test("DM messages wake managed-agent participants without an explicit mention", () => {
  assert.deepEqual(
    resolveManagedAgentSendAudience({
      channelType: "dm",
      dmParticipantPubkeys: [person, agentA],
      explicitMentionPubkeys: [],
      managedAgentPubkeys: [agentA],
    }),
    [agentA],
  );
});

test("channel messages wake only explicitly mentioned managed agents", () => {
  assert.deepEqual(
    resolveManagedAgentSendAudience({
      channelType: "private",
      dmParticipantPubkeys: [agentA],
      explicitMentionPubkeys: [person, agentB],
      managedAgentPubkeys: [agentA, agentB],
    }),
    [agentB],
  );
});

test("managed-agent audience is normalized and deduplicated", () => {
  assert.deepEqual(
    resolveManagedAgentSendAudience({
      channelType: "dm",
      dmParticipantPubkeys: [agentA.toUpperCase(), agentA],
      explicitMentionPubkeys: [agentA],
      managedAgentPubkeys: [agentA],
    }),
    [agentA],
  );
});
