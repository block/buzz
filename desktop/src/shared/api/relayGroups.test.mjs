import assert from "node:assert/strict";
import test from "node:test";

import {
  groupsFromSnapshotEvents,
  parseUserGroupSnapshot,
} from "./relayGroups.ts";

function event(overrides = {}) {
  return {
    id: "event-id",
    pubkey: "f".repeat(64),
    created_at: 1,
    kind: 39100,
    tags: [
      ["d", "f75f743f-178c-49cf-9632-21eea0bb67dc"],
      ["handle", "ios-team"],
      ["name", "iOS Team"],
      ["description", "Mobile builders"],
      ["creator", "A".repeat(64)],
      ["p", "B".repeat(64)],
      ["p", "b".repeat(64)],
      ["channel", "channel-one"],
    ],
    content: "",
    sig: "signature",
    ...overrides,
  };
}

test("parseUserGroupSnapshot parses and deduplicates relay state tags", () => {
  assert.deepEqual(parseUserGroupSnapshot(event()), {
    id: "f75f743f-178c-49cf-9632-21eea0bb67dc",
    handle: "ios-team",
    name: "iOS Team",
    description: "Mobile builders",
    creator: "a".repeat(64),
    memberPubkeys: ["b".repeat(64)],
    defaultChannelIds: ["channel-one"],
  });
});

test("parseUserGroupSnapshot ignores tombstones and malformed state", () => {
  assert.equal(
    parseUserGroupSnapshot(
      event({
        tags: [["d", "f75f743f-178c-49cf-9632-21eea0bb67dc"], ["deleted"]],
      }),
    ),
    null,
  );
  assert.equal(parseUserGroupSnapshot(event({ tags: [["d", "id"]] })), null);
});

test("groupsFromSnapshotEvents keeps the newest state for each id", () => {
  const deleted = event({
    id: "new",
    created_at: 2,
    tags: [["d", "f75f743f-178c-49cf-9632-21eea0bb67dc"], ["deleted"]],
  });
  assert.deepEqual(groupsFromSnapshotEvents([event(), deleted]), []);
});
