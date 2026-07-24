import assert from "node:assert/strict";
import test from "node:test";

import { expandGroupMentions } from "./groupMentionExpansion.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "c".repeat(64);

function group(overrides = {}) {
  return {
    id: "group-id",
    handle: "ios-team",
    name: "iOS Team",
    description: "",
    creator: ALICE,
    memberPubkeys: [ALICE, BOB, CAROL],
    defaultChannelIds: [],
    ...overrides,
  };
}

test("group expansion intersects members with channel membership", () => {
  assert.deepEqual(
    expandGroupMentions({
      individualMentionPubkeys: [],
      groups: [group()],
      channelMemberPubkeys: [ALICE, CAROL],
    }),
    {
      mentionPubkeys: [ALICE, CAROL],
      markerTags: [["group", "group-id", "ios-team"]],
    },
  );
});

test("group expansion deduplicates individual and overlapping group members", () => {
  const result = expandGroupMentions({
    individualMentionPubkeys: [ALICE],
    groups: [
      group(),
      group({
        id: "other-id",
        handle: "mobile",
        memberPubkeys: [BOB, CAROL],
      }),
      group(),
    ],
    channelMemberPubkeys: [ALICE, BOB],
  });

  assert.deepEqual(result.mentionPubkeys, [ALICE, BOB]);
  assert.deepEqual(result.markerTags, [
    ["group", "group-id", "ios-team"],
    ["group", "other-id", "mobile"],
  ]);
});
