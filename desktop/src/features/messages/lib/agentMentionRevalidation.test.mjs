import assert from "node:assert/strict";
import test from "node:test";

import { revalidateAgentMentionPubkeys } from "./agentMentionRevalidation.ts";

const CURRENT = "a".repeat(64);
const AGENT = "b".repeat(64);
const HUMAN = "c".repeat(64);
const OTHER_OWNER = "d".repeat(64);
const LOCAL_AGENT = "e".repeat(64);

function options(refetchOwnerProfiles) {
  return {
    pubkeys: [HUMAN, AGENT],
    agentPubkeys: new Set([AGENT]),
    currentPubkey: CURRENT,
    eligibilityScope: { type: "channel", channelId: "general" },
    sharedChannelIds: new Set(["general"]),
    ownerOnly: true,
    ownerPolicyError: null,
    refetchManagedAgents: async () => ({ data: [], error: null }),
    refetchRelayAgents: async () => ({
      data: [
        {
          pubkey: AGENT,
          respondTo: "anyone",
          respondToAllowlist: [],
          channelIds: ["general"],
        },
      ],
      error: null,
    }),
    refetchOwnerProfiles,
  };
}

test("owner-only revalidation admits an agent only from a fresh same-owner proof", async () => {
  const requested = [];
  const result = await revalidateAgentMentionPubkeys(
    options(async (pubkeys) => {
      requested.push(...pubkeys);
      return {
        profiles: { [AGENT]: { ownerPubkey: CURRENT } },
        missing: [],
      };
    }),
  );

  assert.deepEqual(requested, [AGENT]);
  assert.deepEqual(result, [HUMAN, AGENT]);
});

test("fresh managed evidence survives unrelated relay authorization errors", async () => {
  const result = await revalidateAgentMentionPubkeys({
    ...options(async () => {
      throw new Error("owner profiles unavailable");
    }),
    pubkeys: [HUMAN, LOCAL_AGENT],
    agentPubkeys: new Set([LOCAL_AGENT]),
    refetchManagedAgents: async () => ({
      data: [{ pubkey: LOCAL_AGENT }],
      error: null,
    }),
    refetchRelayAgents: async () => ({
      data: undefined,
      error: new Error("relay directory unavailable"),
    }),
  });

  assert.deepEqual(result, [HUMAN, LOCAL_AGENT]);
});

test("relay-only agents still fail closed when relay discovery fails", async () => {
  const result = await revalidateAgentMentionPubkeys({
    ...options(async () => ({
      profiles: { [AGENT]: { ownerPubkey: CURRENT } },
      missing: [],
    })),
    refetchRelayAgents: async () => ({
      data: undefined,
      error: new Error("relay directory unavailable"),
    }),
  });

  assert.deepEqual(result, [HUMAN]);
});

test("mixed evidence preserves only fresh managed agents and humans", async () => {
  const result = await revalidateAgentMentionPubkeys({
    ...options(async () => ({
      profiles: { [AGENT]: { ownerPubkey: CURRENT } },
      missing: [LOCAL_AGENT],
    })),
    pubkeys: [HUMAN, LOCAL_AGENT, AGENT],
    agentPubkeys: new Set([LOCAL_AGENT, AGENT]),
    refetchManagedAgents: async () => ({
      data: [{ pubkey: LOCAL_AGENT }],
      error: null,
    }),
    refetchRelayAgents: async () => ({
      data: undefined,
      error: new Error("relay directory unavailable"),
    }),
  });

  assert.deepEqual(result, [HUMAN, LOCAL_AGENT]);
});

for (const [name, refetchOwnerProfiles] of [
  ["revoked owner proof", async () => ({ profiles: {}, missing: [AGENT] })],
  [
    "changed owner proof",
    async () => ({
      profiles: { [AGENT]: { ownerPubkey: OTHER_OWNER } },
      missing: [],
    }),
  ],
  [
    "owner profile query error",
    async () => {
      throw new Error("relay unavailable");
    },
  ],
]) {
  test(`owner-only revalidation fails closed on ${name}`, async () => {
    assert.deepEqual(
      await revalidateAgentMentionPubkeys(options(refetchOwnerProfiles)),
      [HUMAN],
    );
  });
}
