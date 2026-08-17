import assert from "node:assert/strict";
import test from "node:test";

import { revalidateAgentMentionPubkeys } from "./agentMentionRevalidation.ts";

const CURRENT = "a".repeat(64);
const AGENT = "b".repeat(64);
const HUMAN = "c".repeat(64);
const OTHER_OWNER = "d".repeat(64);

function options(refetchOwnerProfiles, overrides = {}) {
  return {
    pubkeys: [HUMAN, AGENT],
    agentPubkeys: new Set([AGENT]),
    currentPubkey: CURRENT,
    eligibilityScope: {
      type: "channel",
      channelId: "general",
      memberPubkeys: new Set([AGENT]),
    },
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
    refetchChannelMembers: async () => ({
      data: [{ pubkey: AGENT }],
      error: null,
    }),
    refetchOwnerProfiles,
    ...overrides,
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

test("channel revalidation drops an agent removed after selection", async () => {
  const result = await revalidateAgentMentionPubkeys(
    options(
      async () => ({
        profiles: { [AGENT]: { ownerPubkey: CURRENT } },
        missing: [],
      }),
      { refetchChannelMembers: async () => ({ data: [], error: null }) },
    ),
  );

  assert.deepEqual(result, [HUMAN]);
});

test("channel revalidation fails closed when the fresh roster fails", async () => {
  const result = await revalidateAgentMentionPubkeys(
    options(
      async () => ({
        profiles: { [AGENT]: { ownerPubkey: CURRENT } },
        missing: [],
      }),
      {
        refetchChannelMembers: async () => ({
          data: [{ pubkey: AGENT }],
          error: new Error("relay unavailable"),
        }),
      },
    ),
  );

  assert.deepEqual(result, [HUMAN]);
});

test("invite preflight revalidates policy before requiring membership", async () => {
  let rosterReads = 0;
  const result = await revalidateAgentMentionPubkeys(
    options(
      async () => ({
        profiles: { [AGENT]: { ownerPubkey: CURRENT } },
        missing: [],
      }),
      {
        eligibilityScope: {
          type: "channel",
          channelId: "general",
          memberPubkeys: new Set(),
        },
        requireChannelMembership: false,
        refetchChannelMembers: async () => {
          rosterReads += 1;
          return { data: [], error: null };
        },
      },
    ),
  );

  assert.equal(rosterReads, 0);
  assert.deepEqual(result, [HUMAN, AGENT]);
});

test("fresh roster admits an authorized agent with stale directory channels", async () => {
  const result = await revalidateAgentMentionPubkeys(
    options(
      async () => ({
        profiles: { [AGENT]: { ownerPubkey: CURRENT } },
        missing: [],
      }),
      {
        refetchRelayAgents: async () => ({
          data: [
            {
              pubkey: AGENT,
              respondTo: "anyone",
              respondToAllowlist: [],
              channelIds: [],
            },
          ],
          error: null,
        }),
      },
    ),
  );

  assert.deepEqual(result, [HUMAN, AGENT]);
});
