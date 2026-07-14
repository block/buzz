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

test("upsertCachedChannelMember_recordsDmParticipantBeforeRefetch", () => {
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
  const updatedDm = channels?.[0];

  assert.deepEqual(updatedDm?.participantPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.deepEqual(updatedDm?.participants, ["charlie", "owner", "Fizz"]);
  assert.deepEqual(updatedDm?.memberPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.equal(updatedDm?.memberCount, 3);
});

test("upsertCachedChannelMember_doesNotDuplicateExistingParticipant", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const fizzPubkey = "FIZZ-PUBKEY";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey, fizzPubkey],
    participants: ["charlie", "owner", "Fizz"],
  });

  const channels = upsertCachedChannelMember([openedDm], openedDm.id, {
    membershipAdded: false,
    name: "Fizz duplicate",
    pubkey: fizzPubkey.toLowerCase(),
  });

  assert.deepEqual(channels, [openedDm]);
});

test("reconcileRefreshedCachedChannel_preservesOptimisticParticipantAcrossStaleRefresh", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const fizzPubkey = "fizz-pubkey";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey],
    participants: ["charlie", "owner"],
  });
  const cachedBeforeRefresh = upsertCachedChannelMember(
    [openedDm],
    openedDm.id,
    {
      membershipAdded: true,
      name: "Fizz",
      pubkey: fizzPubkey,
    },
  )?.[0];

  const reconciled = reconcileRefreshedCachedChannel(
    [openedDm],
    openedDm,
    cachedBeforeRefresh,
  );

  assert.deepEqual(reconciled[0].participantPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.deepEqual(reconciled[0].participants, ["charlie", "owner", "Fizz"]);
  assert.deepEqual(reconciled[0].memberPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.equal(reconciled[0].memberCount, 3);
});

test("upsertCachedChannel_preservesParticipantsAddedAfterDmOpened", () => {
  const charliePubkey = "charlie-pubkey";
  const ownerPubkey = "owner-pubkey";
  const fizzPubkey = "fizz-pubkey";
  const openedDm = makeChannel("new-dm", "DM", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey],
    participants: ["charlie", "owner"],
  });
  const postAttachDm = makeChannel("new-dm", "Group DM (3)", "dm", {
    participantPubkeys: [charliePubkey, ownerPubkey, fizzPubkey],
    participants: ["charlie", "owner", "Fizz"],
  });

  const repairedChannels = upsertCachedChannel([postAttachDm], openedDm, {
    preserveCachedDmParticipants: true,
  });
  const repairedDm = repairedChannels[0];

  assert.equal(repairedDm.name, openedDm.name);
  assert.deepEqual(repairedDm.participantPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.deepEqual(repairedDm.participants, ["charlie", "owner", "Fizz"]);
  assert.deepEqual(repairedDm.memberPubkeys, [
    charliePubkey,
    ownerPubkey,
    fizzPubkey,
  ]);
  assert.equal(repairedDm.memberCount, 3);
});
