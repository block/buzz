import assert from "node:assert/strict";
import test from "node:test";

import {
  revalidateAgentMentionPubkeys,
  AgentMentionAuthorizationError,
} from "./agentMentionRevalidation.ts";

const CURRENT = "a".repeat(64);
const AGENT = "b".repeat(64);
const HUMAN = "c".repeat(64);
const LOCAL_AGENT = "e".repeat(64);

function options() {
  return {
    pubkeys: [HUMAN, AGENT],
    agentPubkeys: new Set([AGENT]),
    currentPubkey: CURRENT,
    eligibilityScope: { type: "channel", channelId: "general" },
    sharedChannelIds: new Set(["general"]),
    refetchManagedAgents: async () => ({ data: [], error: null }),
    fetchRelayAgents: async () => [
      {
        pubkey: AGENT,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
    ],
  };
}

test("relay policy revalidation admits an authorized external agent", async () => {
  assert.deepEqual(await revalidateAgentMentionPubkeys(options()), [
    HUMAN,
    AGENT,
  ]);
});

test("fresh managed evidence survives unrelated relay authorization errors", async () => {
  const result = await revalidateAgentMentionPubkeys({
    ...options(),
    pubkeys: [HUMAN, LOCAL_AGENT],
    agentPubkeys: new Set([LOCAL_AGENT]),
    refetchManagedAgents: async () => ({
      data: [{ pubkey: LOCAL_AGENT }],
      error: null,
    }),
    fetchRelayAgents: async () => {
      throw new Error("relay directory unavailable");
    },
  });
  assert.deepEqual(result, [HUMAN, LOCAL_AGENT]);
});

test("relay-only agents still fail closed when relay discovery fails", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
      ...options(),
      fetchRelayAgents: async () => {
        throw new Error("relay directory unavailable");
      },
    }),
    AgentMentionAuthorizationError,
  );
});

test("mixed evidence cannot silently drop an intended relay recipient", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
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
      fetchRelayAgents: async () => {
        throw new Error("relay directory unavailable");
      },
    }),
    AgentMentionAuthorizationError,
  );
});

test("remote-owned membership does not depend on local runtime discovery", async () => {
  assert.deepEqual(
    await revalidateAgentMentionPubkeys({
      ...options(),
      refetchManagedAgents: async () => ({
        data: undefined,
        error: new Error("local unavailable"),
      }),
      fetchRelayAgents: async () => [
        {
          pubkey: AGENT,
          ownerPubkey: CURRENT,
          respondTo: "owner-only",
          respondToAllowlist: [],
          channelIds: ["general"],
        },
      ],
    }),
    [HUMAN, AGENT],
  );
});

test("stale local data is not authority when its refresh fails", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
      ...options(),
      pubkeys: [HUMAN, LOCAL_AGENT, AGENT],
      agentPubkeys: new Set([LOCAL_AGENT, AGENT]),
      refetchManagedAgents: async () => ({
        data: [{ pubkey: LOCAL_AGENT }],
        error: new Error("local unavailable"),
      }),
    }),
    AgentMentionAuthorizationError,
  );
});

test("owned remote policy revocation and missing membership fail closed", async () => {
  for (const agent of [
    { respondTo: "nobody", channelIds: ["general"] },
    { respondTo: "owner-only", channelIds: [] },
  ]) {
    await assert.rejects(
      revalidateAgentMentionPubkeys({
        ...options(),
        fetchRelayAgents: async () => [
          {
            pubkey: AGENT,
            ownerPubkey: CURRENT,
            respondToAllowlist: [],
            ...agent,
          },
        ],
      }),
      AgentMentionAuthorizationError,
    );
  }
});

for (const type of ["channel", "owned"]) {
  test(`${type}: preparation admits owned nonmembers but publication requires actual membership`, async () => {
    let channelIds = [];
    const opts = {
      ...options(),
      eligibilityScope: { type, channelId: "target" },
      sharedChannelIds: new Set(),
      fetchRelayAgents: async () => [
        {
          pubkey: AGENT,
          ownerPubkey: CURRENT,
          respondTo: "allowlist",
          respondToAllowlist: [],
          channelIds,
        },
      ],
    };
    assert.deepEqual(
      await revalidateAgentMentionPubkeys({ ...opts, phase: "prepare" }),
      [HUMAN, AGENT],
    );
    await assert.rejects(
      revalidateAgentMentionPubkeys(opts),
      AgentMentionAuthorizationError,
    );
    channelIds = ["target"];
    assert.deepEqual(await revalidateAgentMentionPubkeys(opts), [HUMAN, AGENT]);
    channelIds = ["other"];
    await assert.rejects(
      revalidateAgentMentionPubkeys(opts),
      AgentMentionAuthorizationError,
    );
  });
}

test("preparation cannot bypass a fresh owner-policy denial", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
      ...options(),
      phase: "prepare",
      fetchRelayAgents: async () => [
        {
          pubkey: AGENT,
          ownerPubkey: CURRENT,
          respondTo: "nobody",
          respondToAllowlist: [],
          channelIds: [],
        },
      ],
    }),
    AgentMentionAuthorizationError,
  );
});

test("publication cannot authorize a DM that still has no destination", async () => {
  await assert.rejects(
    revalidateAgentMentionPubkeys({
      ...options(),
      eligibilityScope: { type: "owned", channelId: null },
      fetchRelayAgents: async () => [
        {
          pubkey: AGENT,
          ownerPubkey: CURRENT,
          respondTo: "owner-only",
          respondToAllowlist: [],
          channelIds: ["other"],
        },
      ],
    }),
    AgentMentionAuthorizationError,
  );
});
