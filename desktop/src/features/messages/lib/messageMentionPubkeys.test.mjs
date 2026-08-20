import assert from "node:assert/strict";
import test from "node:test";

import { messageMentionPubkeys } from "./messageMentionPubkeys.ts";

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

test("plain DM messages p-tag every recipient except the sender", () => {
  assert.deepEqual(messageMentionPubkeys(channel(), "owner"), ["agent"]);
});

test("DM recipients and explicit mentions are normalized and deduplicated", () => {
  assert.deepEqual(
    messageMentionPubkeys(channel(), "OWNER", ["AGENT", "third"]),
    ["agent", "third"],
  );
});

test("stream messages preserve explicit-mention semantics", () => {
  assert.deepEqual(
    messageMentionPubkeys(
      channel({ channelType: "stream", memberPubkeys: ["owner", "agent"] }),
      "owner",
      [],
    ),
    [],
  );
});

test("group DM (3+ members) plain messages p-tag every other participant", () => {
  const groupDm = channel({
    memberCount: 4,
    memberPubkeys: ["owner", "fizz", "honey", "bumble"],
    participantPubkeys: ["owner", "fizz", "honey", "bumble"],
  });
  assert.deepEqual(
    messageMentionPubkeys(groupDm, "owner").sort(),
    ["bumble", "fizz", "honey"].sort(),
  );
});

test("group DM still notifies everyone when memberPubkeys is stale/empty but participantPubkeys holds", () => {
  // Regresses a real gap: `memberPubkeys` comes from a separate, best-effort
  // relay query (desktop/src-tauri/src/commands/channels.rs) that silently
  // defaults to empty on failure/timeout, with no retry or surfaced error.
  // `participantPubkeys` is parsed synchronously from the channel's own
  // metadata event tags, so it should keep this working even when the
  // member-count query comes back empty.
  const groupDm = channel({
    memberCount: 4,
    memberPubkeys: [],
    participantPubkeys: ["owner", "fizz", "honey", "bumble"],
  });
  assert.deepEqual(
    messageMentionPubkeys(groupDm, "owner").sort(),
    ["bumble", "fizz", "honey"].sort(),
  );
});

test("KNOWN GAP: group DM silently drops every recipient when both pubkey sources are empty", () => {
  // Documents the actual failure mode behind the incident where a message in
  // a 4-person group DM went out with zero `p` tags and nobody was notified.
  // `participantPubkeys` is parsed once from the channel's metadata event at
  // creation time (nostr_convert.rs) and is never refreshed — if a member is
  // added to the DM after creation and the separate `memberPubkeys` relay
  // query (channels.rs) hasn't caught up yet (or fails), both sources can be
  // incomplete/empty at send time, and this function has no fallback.
  // This test intentionally asserts the current (broken) behavior so it goes
  // red the moment a real fix (e.g. refreshing participantPubkeys, or
  // retrying/erroring instead of silently defaulting memberPubkeys to empty)
  // lands — flip the assertion to the non-empty recipient list at that point.
  const groupDm = channel({
    memberCount: 4,
    memberPubkeys: [],
    participantPubkeys: [],
  });
  assert.deepEqual(messageMentionPubkeys(groupDm, "owner"), []);
});
