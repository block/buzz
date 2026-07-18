import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelMentionTags,
  channelMentionModes,
  getChannelMentionAudienceLimitError,
  hasChannelMentionForPubkey,
  isChannelWideMentionEvent,
} from "./channelMentions.ts";

const SELF = "a".repeat(64);
const ONLINE = "b".repeat(64);
const AWAY = "c".repeat(64);
const OFFLINE = "d".repeat(64);

test("channelMentionModes recognizes reserved mentions outside code", () => {
  assert.deepEqual(channelMentionModes("hi @everyone and @here"), [
    "everyone",
    "here",
  ]);
  assert.deepEqual(channelMentionModes("`@everyone`"), []);
});

test("@everyone snapshots every other channel member", () => {
  assert.deepEqual(
    buildChannelMentionTags({
      memberPubkeys: [SELF, ONLINE, AWAY, ONLINE.toUpperCase()],
      selfPubkey: SELF,
      text: "Heads up @everyone",
    }),
    [
      ["buzz-audience-ref", "everyone"],
      ["p", ONLINE, "", "buzz:audience:everyone"],
      ["p", AWAY, "", "buzz:audience:everyone"],
    ],
  );
});

test("@here snapshots only online members", () => {
  const tags = buildChannelMentionTags({
    memberPubkeys: [SELF, ONLINE, AWAY, OFFLINE],
    presence: { [ONLINE]: "online", [AWAY]: "away", [OFFLINE]: "offline" },
    selfPubkey: SELF,
    text: "Heads up @here",
  });

  assert.deepEqual(tags, [
    ["buzz-audience-ref", "here"],
    ["p", ONLINE, "", "buzz:audience:here"],
  ]);
  assert.equal(hasChannelMentionForPubkey(tags, ONLINE.toUpperCase()), true);
  assert.equal(hasChannelMentionForPubkey(tags, AWAY), false);
  assert.equal(isChannelWideMentionEvent(tags), true);
  assert.equal(
    isChannelWideMentionEvent([
      ["p", ONLINE, "", "buzz:audience:here", "unexpected"],
    ]),
    false,
  );
});

test("an unchanged channel mention edit preserves rendering without renotifying", () => {
  assert.deepEqual(
    buildChannelMentionTags({
      memberPubkeys: [SELF, ONLINE],
      originalText: "hello @everyone",
      selfPubkey: SELF,
      text: "hello @everyone!",
    }),
    [["buzz-audience-ref", "everyone"]],
  );
});

test("@everyone supersedes @here notification recipients", () => {
  assert.deepEqual(
    buildChannelMentionTags({
      memberPubkeys: [SELF, ONLINE, AWAY],
      presence: { [ONLINE]: "online", [AWAY]: "away" },
      selfPubkey: SELF,
      text: "@here @everyone",
    }),
    [
      ["buzz-audience-ref", "everyone"],
      ["buzz-audience-ref", "here"],
      ["p", ONLINE, "", "buzz:audience:everyone"],
      ["p", AWAY, "", "buzz:audience:everyone"],
    ],
  );
});

test("@everyone preserves standard p-tag delivery beyond the direct mention cap", () => {
  const members = Array.from({ length: 51 }, (_, index) =>
    (index + 1).toString(16).padStart(64, "0"),
  );
  const tags = buildChannelMentionTags({
    memberPubkeys: [SELF, ...members],
    selfPubkey: SELF,
    text: "Heads up @everyone",
  });

  assert.equal(tags.length, 52);
  assert.equal(tags.filter((tag) => tag[0] === "p").length, 51);
  assert.deepEqual(tags.at(-1), [
    "p",
    members.at(-1),
    "",
    "buzz:audience:everyone",
  ]);
});

test("channel-wide mention limit accepts 4,000 recipients and rejects 4,001", () => {
  assert.equal(getChannelMentionAudienceLimitError(4_000), null);
  assert.equal(
    getChannelMentionAudienceLimitError(4_001),
    "Channel-wide mentions support up to 4,000 recipients.",
  );
});
