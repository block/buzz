import assert from "node:assert/strict";
import test from "node:test";

import {
  reconcileRefreshedCachedChannel,
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
