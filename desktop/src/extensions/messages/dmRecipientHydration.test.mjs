import assert from "node:assert/strict";
import test from "node:test";

import {
  dmRecipientPubkeysNeedHydration,
  resolveHydratedMessageRecipientPubkeys,
  resolveInboxReplyRecipientPubkeys,
  resolveMessageRecipientPubkeys,
} from "./dmRecipientHydration.ts";

function channel(overrides = {}) {
  return {
    id: "dm-1",
    name: "DM",
    channelType: "dm",
    visibility: "private",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: ["OWNER", "AGENT"],
    participantPubkeys: ["owner", "agent"],
    participants: [],
    lastMessageAt: null,
    archivedAt: null,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("a restarted DM with empty participant arrays requires membership hydration", () => {
  assert.equal(
    dmRecipientPubkeysNeedHydration(
      channel({ memberPubkeys: [], participantPubkeys: [] }),
      "owner",
    ),
    true,
  );
});

test("cached authoritative members repair a restarted DM without another relay read", async () => {
  let loadCalls = 0;
  const recipients = await resolveHydratedMessageRecipientPubkeys({
    channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
    senderPubkey: "owner",
    cachedMemberPubkeys: ["OWNER", "AGENT"],
    loadDmMemberPubkeys: async () => {
      loadCalls += 1;
      return [];
    },
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.equal(loadCalls, 0);
});

test("a complete DM keeps the local fast path", async () => {
  let loadCalls = 0;
  const recipients = await resolveHydratedMessageRecipientPubkeys({
    channel: channel(),
    senderPubkey: "owner",
    loadDmMemberPubkeys: async () => {
      loadCalls += 1;
      return [];
    },
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.equal(loadCalls, 0);
});

test("an incomplete restarted DM loads authoritative membership before sending", async () => {
  let loadCalls = 0;
  const recipients = await resolveHydratedMessageRecipientPubkeys({
    channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
    senderPubkey: "owner",
    loadDmMemberPubkeys: async () => {
      loadCalls += 1;
      return ["OWNER", "AGENT"];
    },
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.equal(loadCalls, 1);
});

test("an incomplete group DM hydrates every missing recipient", async () => {
  const recipients = await resolveHydratedMessageRecipientPubkeys({
    channel: channel({
      memberCount: 3,
      memberPubkeys: ["owner", "agent"],
      participantPubkeys: [],
    }),
    senderPubkey: "owner",
    loadDmMemberPubkeys: async () => ["owner", "agent", "third"],
  });

  assert.deepEqual(recipients, ["agent", "third"]);
});

test("an unresolved DM fails instead of publishing an unreachable message", async () => {
  await assert.rejects(
    resolveHydratedMessageRecipientPubkeys({
      channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
      senderPubkey: "owner",
      loadDmMemberPubkeys: async () => ["owner"],
    }),
    /recipients are still loading/i,
  );
});

test("stream messages never trigger DM membership loading", async () => {
  let loadCalls = 0;
  const recipients = await resolveHydratedMessageRecipientPubkeys({
    channel: channel({ channelType: "stream" }),
    senderPubkey: "owner",
    explicitMentions: ["third"],
    loadDmMemberPubkeys: async () => {
      loadCalls += 1;
      return [];
    },
  });

  assert.deepEqual(recipients, ["third"]);
  assert.equal(loadCalls, 0);
});

test("the send hook shares cached channel members before any relay fetch", async () => {
  let fetchCalls = 0;
  const queryClient = {
    getQueryData: () => [{ pubkey: "OWNER" }, { pubkey: "AGENT" }],
    fetchQuery: async () => {
      fetchCalls += 1;
      return [];
    },
  };

  const recipients = await resolveMessageRecipientPubkeys({
    channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
    senderPubkey: "owner",
    queryClient,
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.equal(fetchCalls, 0);
});

test("the send hook forces an authoritative members query for incomplete cache state", async () => {
  let fetchOptions;
  const queryClient = {
    getQueryData: () => undefined,
    fetchQuery: async (options) => {
      fetchOptions = options;
      return [{ pubkey: "OWNER" }, { pubkey: "AGENT" }];
    },
  };

  const recipients = await resolveMessageRecipientPubkeys({
    channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
    senderPubkey: "owner",
    queryClient,
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.deepEqual(fetchOptions.queryKey, ["channels", "dm-1", "members"]);
  assert.equal(fetchOptions.staleTime, 0);
});

test("the inbox composer addresses every DM recipient through the shared send path", async () => {
  let fetchCalls = 0;
  const recipients = await resolveInboxReplyRecipientPubkeys({
    channel: channel({ memberPubkeys: [], participantPubkeys: [] }),
    channelId: "dm-1",
    senderPubkey: "owner",
    queryClient: {
      getQueryData: () => [{ pubkey: "OWNER" }, { pubkey: "AGENT" }],
      fetchQuery: async () => {
        fetchCalls += 1;
        return [];
      },
    },
  });

  assert.deepEqual(recipients, ["agent"]);
  assert.equal(fetchCalls, 0);
});

test("the inbox composer refuses to publish before its channel snapshot is ready", async () => {
  await assert.rejects(
    resolveInboxReplyRecipientPubkeys({
      channel: null,
      channelId: "dm-1",
      senderPubkey: "owner",
      queryClient: {},
    }),
    /channel details are still loading/i,
  );
});

test("the inbox composer refuses to publish before its sender identity is ready", async () => {
  await assert.rejects(
    resolveInboxReplyRecipientPubkeys({
      channel: channel(),
      channelId: "dm-1",
      senderPubkey: null,
      queryClient: {},
    }),
    /identity is still loading/i,
  );
});
