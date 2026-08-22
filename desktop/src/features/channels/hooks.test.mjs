import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";

import {
  applyLastMessages,
  canFetchChannelsForIdentity,
  channelsQueryKey,
  reconcileRefreshedCachedChannel,
  refreshChannelsQuery,
  requireFullChannelList,
  upsertCachedChannel,
  upsertCachedChannelMember,
} from "./hooks.ts";

function makeChannel(
  id,
  name,
  channelType = "stream",
  { participantPubkeys = [], participants = [], lastMessageAt = null } = {},
) {
  return {
    id,
    name,
    channelType,
    visibility: channelType === "dm" ? "private" : "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: participantPubkeys.length,
    memberPubkeys: [...participantPubkeys],
    lastMessageAt,
    archivedAt: null,
    participants,
    participantPubkeys,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function makeRefreshHarness({ cachedHash = "hash-1" } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const start = makeChannel("general", "General", "stream", {
    lastMessageAt: "2026-01-01T00:00:00.000Z",
  });
  queryClient.setQueryData(channelsQueryKey, [start]);
  const request = deferred();
  const calls = [];
  const fetchChannels = (knownHash) => {
    calls.push(knownHash);
    return request.promise;
  };
  const initialSnapshotPair = cachedHash
    ? { channels: [start], hash: cachedHash }
    : null;

  return {
    calls,
    fetchChannels,
    initialSnapshotPair,
    queryClient,
    request,
    start,
  };
}

function setDisplayedRecency(queryClient, lastMessageAt) {
  queryClient.setQueryData(channelsQueryKey, (channels) =>
    channels.map((channel) =>
      channel.id === "general" ? { ...channel, lastMessageAt } : channel,
    ),
  );
}

function refreshWithHarness(harness, fetchChannels = harness.fetchChannels) {
  return harness.queryClient.fetchQuery({
    queryKey: channelsQueryKey,
    queryFn: () =>
      refreshChannelsQuery({
        queryClient: harness.queryClient,
        initialSnapshotPair: harness.initialSnapshotPair,
        relayUrl: null,
        ownerPubkey: null,
        fetchChannels,
      }),
  });
}

const T1 = "2026-01-01T00:01:00.000Z";
const T2 = "2026-01-01T00:02:00.000Z";

test("refreshChannelsQuery preserves a live update through matching not-modified settlement", async () => {
  const harness = makeRefreshHarness();
  const refresh = refreshWithHarness(harness);

  assert.deepEqual(harness.calls, ["hash-1"]);
  setDisplayedRecency(harness.queryClient, T2);
  harness.request.resolve({
    hash: "hash-1",
    channels: null,
    lastMessages: { general: T1 },
  });

  const result = await refresh;
  assert.equal(result[0].lastMessageAt, T2);
  assert.equal(
    harness.queryClient.getQueryData(channelsQueryKey)[0].lastMessageAt,
    T2,
  );
});

test("refreshChannelsQuery preserves a live update through authoritative full-list settlement", async () => {
  const harness = makeRefreshHarness({ cachedHash: null });
  const refresh = refreshWithHarness(harness);

  assert.deepEqual(harness.calls, [null]);
  setDisplayedRecency(harness.queryClient, T2);
  harness.request.resolve({
    hash: "hash-2",
    channels: [makeChannel("general", "General")],
    lastMessages: { general: T1 },
  });

  const result = await refresh;
  assert.equal(result[0].lastMessageAt, T2);
  assert.equal(
    harness.queryClient.getQueryData(channelsQueryKey)[0].lastMessageAt,
    T2,
  );
});

test("refreshChannelsQuery preserves a live update through mismatched not-modified retry", async () => {
  const harness = makeRefreshHarness();
  const retry = deferred();
  const fetchChannels = (knownHash) => {
    harness.calls.push(knownHash);
    return harness.calls.length === 1
      ? Promise.resolve({
          hash: "mismatched-hash",
          channels: null,
          lastMessages: {},
        })
      : retry.promise;
  };
  const refresh = refreshWithHarness(harness, fetchChannels);

  await Promise.resolve();
  assert.deepEqual(harness.calls, ["hash-1", null]);
  setDisplayedRecency(harness.queryClient, T2);
  retry.resolve({
    hash: "hash-2",
    channels: [makeChannel("general", "General")],
    lastMessages: { general: T1 },
  });

  const result = await refresh;
  assert.equal(result[0].lastMessageAt, T2);
  assert.equal(
    harness.queryClient.getQueryData(channelsQueryKey)[0].lastMessageAt,
    T2,
  );
});

test("refreshChannelsQuery clears unchanged recency on authoritative absence", async () => {
  const harness = makeRefreshHarness();
  const refresh = refreshWithHarness(harness);

  harness.request.resolve({
    hash: "hash-1",
    channels: null,
    lastMessages: {},
  });

  const result = await refresh;
  assert.equal(result[0].lastMessageAt, null);
});

test("upsertCachedChannel_reseedsOpenedDmAfterStaleRefetch", () => {
  const staleChannels = [makeChannel("general", "General")];
  const openedDm = makeChannel("new-dm", "Alice", "dm");

  const repairedChannels = upsertCachedChannel(staleChannels, openedDm);

  assert.strictEqual(
    repairedChannels.find((channel) => channel.id === openedDm.id),
    openedDm,
    "the route must be able to resolve the exact relay-returned DM",
  );
});

test("upsertCachedChannel_replacesExistingChannelWithoutDuplicates", () => {
  const staleDm = makeChannel("new-dm", "Old name", "dm");
  const openedDm = makeChannel("new-dm", "Alice", "dm");

  const repairedChannels = upsertCachedChannel([staleDm], openedDm);

  assert.deepEqual(repairedChannels, [openedDm]);
});

test("upsertCachedChannelMember_doesNotDecorateImmutableDmSource", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const fizzPubkey = "fizz-pubkey";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey],
    participants: ["charlie", "owner"],
  });

  const channels = upsertCachedChannelMember([openedDm], openedDm.id, {
    membershipAdded: true,
    name: "Fizz",
    pubkey: fizzPubkey,
  });
  assert.deepEqual(channels, [openedDm]);
});

test("upsertCachedChannelMember_recordsStreamMemberBeforeRefetch", () => {
  const fizzPubkey = "fizz-pubkey";
  const channel = makeChannel("general", "General");

  const channels = upsertCachedChannelMember([channel], channel.id, {
    membershipAdded: true,
    name: "Fizz",
    pubkey: fizzPubkey,
  });

  assert.deepEqual(channels?.[0].memberPubkeys, [fizzPubkey]);
  assert.equal(channels?.[0].memberCount, 1);
});

test("reconcileRefreshedCachedChannel_restoresOpenedDmAfterStaleRefresh", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const fizzPubkey = "fizz-pubkey";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey],
    participants: ["charlie", "owner"],
  });
  const expandedDm = makeChannel("expanded-dm", "Group DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey, fizzPubkey],
    participants: ["charlie", "owner", "Fizz"],
  });

  const reconciled = reconcileRefreshedCachedChannel([openedDm], expandedDm);

  assert.deepEqual(reconciled[1].participantPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.deepEqual(reconciled[0], openedDm);
});

test("identity failure enables a hashless live channel fetch", () => {
  assert.equal(canFetchChannelsForIdentity(null, false), false);
  assert.equal(canFetchChannelsForIdentity("owner-pubkey", false), true);
  assert.equal(canFetchChannelsForIdentity(null, true), true);
});

test("hashless retry rejects null channels before persistence", () => {
  const channels = [makeChannel("general", "General")];
  assert.strictEqual(requireFullChannelList(channels), channels);
  assert.throws(
    () => requireFullChannelList(null),
    /no list for a hashless request/,
  );
});

// ── applyLastMessages ─────────────────────────────────────────────────────────

test("applyLastMessages_preservesReferenceWhenTimestampUnchanged", () => {
  const channel = makeChannel("general", "General");
  channel.lastMessageAt = "2026-01-01T00:00:00Z";

  const result = applyLastMessages([channel], {
    general: "2026-01-01T00:00:00Z",
  });

  // Must be the same object reference — structural sharing avoids re-renders.
  assert.strictEqual(
    result[0],
    channel,
    "reference must be preserved when lastMessageAt is unchanged",
  );
});

test("applyLastMessages_createsNewObjectWhenTimestampChanges", () => {
  const channel = makeChannel("general", "General");
  channel.lastMessageAt = "2026-01-01T00:00:00Z";

  const result = applyLastMessages([channel], {
    general: "2026-06-15T12:00:00Z",
  });

  assert.notStrictEqual(
    result[0],
    channel,
    "must create a new object when timestamp changes",
  );
  assert.equal(result[0].lastMessageAt, "2026-06-15T12:00:00Z");
});

test("applyLastMessages_setsNullWhenChannelAbsentFromMap", () => {
  const channel = makeChannel("general", "General");
  channel.lastMessageAt = "2026-01-01T00:00:00Z";

  const result = applyLastMessages([channel], {});

  assert.notStrictEqual(result[0], channel);
  assert.equal(result[0].lastMessageAt, null);
});

test("applyLastMessages_preservesReferenceWhenBothNull", () => {
  const channel = makeChannel("general", "General");
  // lastMessageAt defaults to null in makeChannel

  const result = applyLastMessages([channel], {});

  assert.strictEqual(result[0], channel, "null→null must preserve reference");
});

test("reconcileRefreshedCachedChannel_preservesRefreshedDmRecency", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey],
    participants: ["charlie", "owner"],
  });
  const refreshedDm = {
    ...openedDm,
    lastMessageAt: "2026-07-14T11:21:26Z",
    name: "Group DM (3)",
  };

  const reconciled = reconcileRefreshedCachedChannel([refreshedDm], openedDm);

  assert.equal(reconciled[0].lastMessageAt, refreshedDm.lastMessageAt);
  assert.equal(reconciled[0].name, refreshedDm.name);
  assert.deepEqual(reconciled[0].participantPubkeys, [
    charliePubkey,
    ownerPubkey,
  ]);
});

test("invalidateChannelMembersRosters dedupes and targets member keys", async () => {
  const { invalidateChannelMembersRosters } = await import(
    "./rosterFreshness.ts"
  );
  const invalidated = [];
  const queryClient = {
    invalidateQueries: async ({ queryKey }) => {
      invalidated.push(queryKey);
    },
  };

  await invalidateChannelMembersRosters(queryClient, ["ch-a", "ch-b", "ch-a"]);

  assert.deepEqual(invalidated, [
    ["channels", "ch-a", "members"],
    ["channels", "ch-b", "members"],
  ]);
});
