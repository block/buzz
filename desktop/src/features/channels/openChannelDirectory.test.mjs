import assert from "node:assert/strict";
import test from "node:test";

import { mergeOpenChannelDirectory } from "./openChannelDirectory.ts";

function makeChannel(id, name, channelType = "stream") {
  return {
    id,
    name,
    channelType,
    visibility: channelType === "dm" ? "private" : "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 0,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

test("mergeOpenChannelDirectory_appendsNonMemberOpenChannels", () => {
  const member = makeChannel("general", "General");
  const openOnly = { ...makeChannel("random", "Random"), isMember: false };

  const merged = mergeOpenChannelDirectory([member], [member, openOnly]);

  assert.deepEqual(
    merged.map((channel) => channel.id).sort(),
    ["general", "random"],
    "no non-member open channel may be silently lost",
  );
});

test("mergeOpenChannelDirectory_prefersMemberEntryForSharedId", () => {
  // The member list carries optimistic mutations and poll timestamps, so its
  // entry must win over the directory's snapshot for a shared channel id.
  const memberEntry = { ...makeChannel("general", "General"), memberCount: 9 };
  const directoryEntry = {
    ...makeChannel("general", "General"),
    memberCount: 1,
    isMember: false,
  };

  const merged = mergeOpenChannelDirectory([memberEntry], [directoryEntry]);

  assert.equal(merged.length, 1, "shared id must not duplicate");
  assert.strictEqual(
    merged[0],
    memberEntry,
    "the member entry must win for a shared id",
  );
});

test("mergeOpenChannelDirectory_returnsMemberListWhenDirectoryAbsent", () => {
  const memberList = [makeChannel("general", "General")];

  assert.strictEqual(
    mergeOpenChannelDirectory(memberList, undefined),
    memberList,
    "an un-fetched directory must return the member list untouched",
  );
  assert.strictEqual(
    mergeOpenChannelDirectory(memberList, []),
    memberList,
    "an empty directory must return the member list untouched",
  );
});
