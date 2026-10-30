import assert from "node:assert/strict";
import test from "node:test";

import {
  findCachedOneToOneDm,
  oneToOneDmPubkeys,
  resolveOneToOneDm,
} from "./oneToOneDm.ts";

const ME = "a".repeat(64);
const ALICE = "b".repeat(64);
const BOB = "c".repeat(64);

function dm(id, participantPubkeys, overrides = {}) {
  return {
    id,
    name: id,
    channelType: "dm",
    participantPubkeys,
    ...overrides,
  };
}

test("an existing 1:1 DM is reused, whatever case the relay used", () => {
  const channels = [
    dm("stream", [ME, ALICE], { channelType: "stream" }),
    dm("group", [ME, ALICE, BOB]),
    dm("alice", [ME, ALICE.toUpperCase()]),
  ];

  assert.equal(findCachedOneToOneDm(channels, ALICE, ME)?.id, "alice");
  assert.equal(
    findCachedOneToOneDm(channels, ALICE.toUpperCase(), ME.toUpperCase())?.id,
    "alice",
  );
});

test("no cached 1:1 DM means the caller must open one", () => {
  // A group DM with the same person is not that person's conversation, and a
  // forward must not land in front of the extra participants.
  assert.equal(
    findCachedOneToOneDm([dm("group", [ME, ALICE, BOB])], ALICE, ME),
    null,
  );
  assert.equal(findCachedOneToOneDm([], ALICE, ME), null);
  assert.equal(findCachedOneToOneDm(undefined, ALICE, ME), null);
  assert.equal(findCachedOneToOneDm([dm("bob", [ME, BOB])], ALICE, ME), null);
});

test("the people a 1:1 DM already covers are listed once", () => {
  const pubkeys = oneToOneDmPubkeys(
    [
      dm("alice", [ME, ALICE]),
      dm("self-note", [ME]),
      dm("group", [ME, ALICE, BOB]),
      dm("stream", [ME, BOB], { channelType: "stream" }),
    ],
    ME,
  );

  assert.deepEqual([...pubkeys], [ALICE]);
  assert.deepEqual([...oneToOneDmPubkeys(undefined, ME)], []);
});

test("resolving a person's DM reuses the cached one and opens one otherwise", async () => {
  const cached = dm("alice", [ME, ALICE]);
  const opened = dm("opened", [ME, BOB]);
  const calls = [];
  const openDm = async (input) => {
    calls.push(input);
    return opened;
  };

  assert.equal(
    (
      await resolveOneToOneDm({
        channels: [cached],
        currentPubkey: ME,
        openDm,
        targetPubkey: ALICE,
      })
    ).id,
    "alice",
  );
  assert.deepEqual(calls, []);

  assert.equal(
    (
      await resolveOneToOneDm({
        channels: [cached],
        currentPubkey: ME,
        openDm,
        targetPubkey: BOB.toUpperCase(),
      })
    ).id,
    "opened",
  );
  assert.deepEqual(calls, [{ pubkeys: [BOB] }]);
});
