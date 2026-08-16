import assert from "node:assert/strict";
import test from "node:test";

import {
  applyLastMessages,
  canFetchChannelsForIdentity,
  reconcileRefreshedCachedChannel,
  requireFullChannelList,
  upsertCachedChannel,
  upsertCachedChannelMember,
} from "./hooks.ts";

function makeChannel(
  id,
  name,
  channelType = "stream",
  { participantPubkeys = [], participants = [] } = {},
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
    lastMessageAt: null,
    archivedAt: null,
    participants,
    participantPubkeys,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

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
