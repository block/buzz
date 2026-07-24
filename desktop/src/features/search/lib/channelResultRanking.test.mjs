import assert from "node:assert/strict";
import test from "node:test";

import { rankTopbarChannelResults } from "./channelResultRanking.ts";

function makeChannel(name, overrides = {}) {
  return {
    id: `id-${name}`,
    name,
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 1,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: false,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

function names(results) {
  return results.map((channel) => channel.name);
}

test("relevance beats alphabetical order under the cap", () => {
  // The bug: query "buzz" over many buzz-* channels ranked alphabetically
  // could never surface an exact/prefix match buried late in the alphabet.
  const filler = ["buzz-acp", "buzz-bugs", "buzz-ci", "buzz-dev", "buzz-docs"];
  const channels = [
    ...filler.map((name) => makeChannel(name)),
    makeChannel("buzz"),
  ];

  const results = rankTopbarChannelResults({ channels, lowerQuery: "buzz" });
  assert.equal(results.length, 5);
  assert.equal(results[0].name, "buzz", "exact match must rank first");
});

test("separator-tolerant matching: 'buzz security' finds buzz-security", () => {
  const channels = [makeChannel("buzz-security"), makeChannel("general")];
  assert.deepEqual(
    names(rankTopbarChannelResults({ channels, lowerQuery: "buzz security" })),
    ["buzz-security"],
  );
  assert.deepEqual(
    names(rankTopbarChannelResults({ channels, lowerQuery: "buzzsec" })),
    ["buzz-security"],
  );
});

test("caps results at 5 by default", () => {
  const channels = Array.from({ length: 9 }, (_, i) =>
    makeChannel(`chan-${i}`),
  );
  const results = rankTopbarChannelResults({ channels, lowerQuery: "chan" });
  assert.equal(results.length, 5);
});

test("visibility: private non-member hidden, archived only for members", () => {
  const channels = [
    makeChannel("secret", { visibility: "private" }),
    makeChannel("secret-mine", { visibility: "private", isMember: true }),
    makeChannel("secret-old", { archivedAt: "2026-01-01T00:00:00Z" }),
    makeChannel("secret-old-mine", {
      archivedAt: "2026-01-01T00:00:00Z",
      isMember: true,
    }),
  ];
  assert.deepEqual(
    names(rankTopbarChannelResults({ channels, lowerQuery: "secret" })),
    ["secret-mine", "secret-old-mine"],
  );
});

test("label override is scored as the display name", () => {
  const channels = [
    makeChannel("dm-abc123", { channelType: "dm", isMember: true }),
  ];
  const results = rankTopbarChannelResults({
    channels,
    channelLabels: { "id-dm-abc123": "Tyler" },
    lowerQuery: "tyler",
  });
  assert.deepEqual(names(results), ["dm-abc123"]);
});

test("raw name stays searchable when a label overrides it", () => {
  const channels = [makeChannel("release-notes")];
  const results = rankTopbarChannelResults({
    channels,
    channelLabels: { "id-release-notes": "Announcements" },
    lowerQuery: "release",
  });
  assert.deepEqual(names(results), ["release-notes"]);
});

test("description matches rank below name matches", () => {
  const channels = [
    makeChannel("random", { description: "security chatter" }),
    makeChannel("buzz-security"),
  ];
  assert.deepEqual(
    names(rankTopbarChannelResults({ channels, lowerQuery: "security" })),
    ["buzz-security", "random"],
  );
});

test("no match returns empty", () => {
  const channels = [makeChannel("general")];
  assert.deepEqual(
    rankTopbarChannelResults({ channels, lowerQuery: "zzzzqq" }),
    [],
  );
});
