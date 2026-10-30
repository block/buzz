import assert from "node:assert/strict";
import test from "node:test";

import {
  ALREADY_IN_CHANNEL_ERROR,
  findChannelByName,
  groupInvitees,
  NOT_IN_CHANNEL_ERROR,
  parseChannelRefNames,
  planInvite,
  planRemove,
  USE_LEAVE_ERROR,
} from "./membershipPlan.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "c".repeat(64);
const SELF = "d".repeat(64);

test("invite skips people who are already members and dedupes", () => {
  assert.deepEqual(
    planInvite([BOB.toUpperCase(), CAROL, CAROL], [ALICE, BOB]),
    [CAROL],
  );
  assert.throws(
    () => planInvite([ALICE, BOB], [ALICE.toUpperCase(), BOB]),
    new Error(ALREADY_IN_CHANNEL_ERROR),
  );
});

const removeRows = [
  ["keeps only members", [ALICE, CAROL], [ALICE, BOB], [ALICE]],
  ["compares case-insensitively", [ALICE.toUpperCase()], [ALICE], [ALICE]],
];

for (const [label, mentioned, members, expected] of removeRows) {
  test(`remove ${label}`, () => {
    assert.deepEqual(planRemove(mentioned, members, SELF), expected);
  });
}

test("remove refuses yourself and non-members", () => {
  assert.throws(
    () => planRemove([ALICE, SELF.toUpperCase()], [ALICE, SELF], SELF),
    new Error(USE_LEAVE_ERROR),
  );
  assert.throws(
    () => planRemove([CAROL], [ALICE, BOB], SELF),
    new Error(NOT_IN_CHANNEL_ERROR),
  );
});

test("invitees split into attach, bot and member groups", () => {
  const managed = { pubkey: ALICE };
  const groups = groupInvitees(
    [ALICE, BOB, CAROL],
    new Map([[ALICE, managed]]),
    (pubkey) => pubkey === BOB,
  );
  assert.deepEqual(groups, { attach: [managed], bots: [BOB], humans: [CAROL] });
});

const refRows = [
  ["#design", ["design"]],
  ["  #design extra", ["design"]],
  ["#a #b-2 #a_c", ["a", "b-2", "a_c"]],
  ["design", []],
  ["foo#design", []],
  ["(#design)", ["design"]],
];

for (const [args, expected] of refRows) {
  test(`parses channel refs from ${JSON.stringify(args)}`, () => {
    assert.deepEqual(parseChannelRefNames(args), expected);
  });
}

test("channel lookup prefers a joined channel, skips DMs, and ignores case", () => {
  const archived = {
    id: "1",
    name: "Design",
    channelType: "stream",
    isMember: false,
    archivedAt: "2026-01-01T00:00:00Z",
  };
  const open = { ...archived, id: "2", archivedAt: null };
  const joined = { ...archived, id: "3", isMember: true };
  const dm = { ...archived, id: "4", channelType: "dm", name: "design" };

  assert.equal(findChannelByName("design", [archived, open])?.id, "2");
  assert.equal(findChannelByName("DESIGN", [archived, open, joined])?.id, "3");
  assert.equal(findChannelByName("design", [dm]), undefined);
  assert.equal(findChannelByName("design", [archived])?.id, "1");
});
