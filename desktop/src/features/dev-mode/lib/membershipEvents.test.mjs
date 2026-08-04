import assert from "node:assert/strict";
import test from "node:test";

import {
  parseMembershipEvent,
  selectMembershipEvents,
} from "./membershipEvents.ts";

const KIND_SYSTEM_MESSAGE = 40099;
const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

let nextId = 0;
function systemEvent(payload, createdAt = 1_000) {
  nextId += 1;
  return {
    id: `sys-${nextId}`,
    kind: KIND_SYSTEM_MESSAGE,
    pubkey: "relay",
    created_at: createdAt,
    content: JSON.stringify(payload),
    tags: [],
  };
}

test("self-join parses as joined", () => {
  const change = parseMembershipEvent(
    systemEvent({ type: "member_joined", actor: ALICE, target: ALICE }),
  );
  assert.equal(change.change, "joined");
  assert.equal(change.member, ALICE);
  assert.equal(change.actor, null);
});

test("join by another member parses as added with actor", () => {
  const change = parseMembershipEvent(
    systemEvent({ type: "member_joined", actor: ALICE, target: BOB }),
  );
  assert.equal(change.change, "added");
  assert.equal(change.member, BOB);
  assert.equal(change.actor, ALICE);
});

test("member_left names the actor as the departing member", () => {
  const change = parseMembershipEvent(
    systemEvent({ type: "member_left", actor: BOB }),
  );
  assert.equal(change.change, "left");
  assert.equal(change.member, BOB);
  assert.equal(change.actor, null);
});

test("member_removed carries the removing actor", () => {
  const change = parseMembershipEvent(
    systemEvent({ type: "member_removed", actor: ALICE, target: BOB }),
  );
  assert.equal(change.change, "removed");
  assert.equal(change.member, BOB);
  assert.equal(change.actor, ALICE);
});

test("non-membership system messages and other kinds are ignored", () => {
  assert.equal(
    parseMembershipEvent(
      systemEvent({ type: "topic_changed", actor: ALICE, topic: "x" }),
    ),
    null,
  );
  assert.equal(
    parseMembershipEvent({
      id: "m1",
      kind: 40002,
      pubkey: ALICE,
      created_at: 1,
      content: "hello",
      tags: [],
    }),
    null,
  );
  assert.equal(
    parseMembershipEvent({
      id: "bad",
      kind: KIND_SYSTEM_MESSAGE,
      pubkey: "relay",
      created_at: 1,
      content: "not json",
      tags: [],
    }),
    null,
  );
});

test("selectMembershipEvents filters and sorts oldest first", () => {
  const events = [
    systemEvent({ type: "member_left", actor: BOB }, 300),
    {
      id: "m1",
      kind: 40002,
      pubkey: ALICE,
      created_at: 150,
      content: "hi",
      tags: [],
    },
    systemEvent({ type: "member_joined", actor: BOB, target: BOB }, 100),
    systemEvent({ type: "channel_created", actor: ALICE }, 200),
  ];
  const changes = selectMembershipEvents(events);
  assert.deepEqual(
    changes.map((change) => change.change),
    ["joined", "left"],
  );
  assert.ok(changes[0].event.created_at < changes[1].event.created_at);
});
