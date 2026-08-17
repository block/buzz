import assert from "node:assert/strict";
import { test } from "node:test";

import {
  CHANNEL_MENTION_ADMIN_THRESHOLD,
  MENTION_SCOPE_CHANNEL,
  MENTION_SCOPE_HERE,
  MENTION_SCOPE_TAG,
  canUseMentionScope,
  mentionScopeOf,
  mentionScopeTag,
  resolveMentionAudience,
  shouldNotifyForMentionScope,
} from "./globalMentions.mjs";

// --- reading and writing the marker ---

test("the scope is read off the tag", () => {
  assert.equal(
    mentionScopeOf([
      ["h", "channel-1"],
      [MENTION_SCOPE_TAG, "channel"],
    ]),
    MENTION_SCOPE_CHANNEL,
  );
  assert.equal(
    mentionScopeOf([[MENTION_SCOPE_TAG, "here"]]),
    MENTION_SCOPE_HERE,
  );
});

test("an ordinary message has no scope", () => {
  assert.equal(mentionScopeOf([["h", "channel-1"]]), null);
  assert.equal(mentionScopeOf([]), null);
  assert.equal(mentionScopeOf(undefined), null);
});

test("an unrecognised scope value is ignored, not honoured", () => {
  // A future or malicious client could write anything here.
  assert.equal(mentionScopeOf([[MENTION_SCOPE_TAG, "everyone"]]), null);
  assert.equal(mentionScopeOf([[MENTION_SCOPE_TAG, ""]]), null);
  assert.equal(mentionScopeOf([[MENTION_SCOPE_TAG]]), null);
});

test("the existing broadcast tag is not mistaken for a scope", () => {
  // `broadcast` already means "thread reply echoed to the channel".
  assert.equal(mentionScopeOf([["broadcast", "1"]]), null);
});

test("only valid scopes produce a tag", () => {
  assert.deepEqual(mentionScopeTag("channel"), [MENTION_SCOPE_TAG, "channel"]);
  assert.equal(mentionScopeTag("everyone"), null);
});

// --- the permission gate ---

test("a small channel lets anyone use @channel", () => {
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: 5,
      role: "member",
    }),
    true,
  );
});

test("a large channel restricts @channel to admins by default", () => {
  const large = CHANNEL_MENTION_ADMIN_THRESHOLD + 1;
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: large,
      role: "member",
    }),
    false,
  );
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: large,
      role: "admin",
    }),
    true,
  );
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: large,
      role: "owner",
    }),
    true,
  );
});

test("the threshold itself is still open", () => {
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: CHANNEL_MENTION_ADMIN_THRESHOLD,
      role: "member",
    }),
    true,
  );
});

test("@here is not gated by size", () => {
  // Its reach is bounded by who is actually online, so it needs no size rule.
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_HERE,
      memberCount: 500,
      role: "member",
    }),
    true,
  );
});

test("an admin override beats the size default, both ways", () => {
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: 500,
      role: "member",
      override: "everyone",
    }),
    true,
  );
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: 3,
      role: "member",
      override: "admins",
    }),
    false,
  );
});

test("the two scopes gate independently", () => {
  // Discord's mistake was one permission for both, so an admin could not leave
  // the polite one open while restricting the loud one.
  const large = CHANNEL_MENTION_ADMIN_THRESHOLD + 1;
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      memberCount: large,
      role: "member",
    }),
    false,
  );
  assert.equal(
    canUseMentionScope({
      scope: MENTION_SCOPE_HERE,
      memberCount: large,
      role: "member",
    }),
    true,
  );
});

test("guests and bots do not count as privileged", () => {
  const large = CHANNEL_MENTION_ADMIN_THRESHOLD + 1;
  for (const role of ["guest", "bot", "member", undefined]) {
    assert.equal(
      canUseMentionScope({
        scope: MENTION_SCOPE_CHANNEL,
        memberCount: large,
        role,
      }),
      false,
    );
  }
});

// --- who gets notified ---

test("the author is never notified of their own broadcast", () => {
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      isAuthor: true,
    }),
    false,
  );
});

test("@here reaches only people who are online", () => {
  for (const [presence, expected] of [
    ["online", true],
    ["away", false],
    ["offline", false],
  ]) {
    assert.equal(
      shouldNotifyForMentionScope({ scope: MENTION_SCOPE_HERE, presence }),
      expected,
      `presence ${presence}`,
    );
  }
});

test("@channel reaches people regardless of presence", () => {
  for (const presence of ["online", "away", "offline"]) {
    assert.equal(
      shouldNotifyForMentionScope({ scope: MENTION_SCOPE_CHANNEL, presence }),
      true,
    );
  }
});

test("@channel pierces mute, @here does not", () => {
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      isMuted: true,
    }),
    true,
  );
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_HERE,
      isMuted: true,
      presence: "online",
    }),
    false,
  );
});

test("the per-user opt-out restores silence under mute", () => {
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      isMuted: true,
      allowChannelMentionWhileMuted: false,
    }),
    false,
  );
  // Opting out only matters while muted — it is not a blanket mute.
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      isMuted: false,
      allowChannelMentionWhileMuted: false,
    }),
    true,
  );
});

test("non-members are never notified", () => {
  assert.equal(
    shouldNotifyForMentionScope({
      scope: MENTION_SCOPE_CHANNEL,
      isMember: false,
    }),
    false,
  );
});

// --- audience resolution ---

test("@channel resolves to every member except the author", () => {
  assert.deepEqual(
    resolveMentionAudience({
      scope: MENTION_SCOPE_CHANNEL,
      members: ["alice", "bob", "carol"],
      authorPubkey: "alice",
    }),
    ["bob", "carol"],
  );
});

test("@here resolves to online members only", () => {
  assert.deepEqual(
    resolveMentionAudience({
      scope: MENTION_SCOPE_HERE,
      members: ["alice", "bob", "carol"],
      authorPubkey: "alice",
      presenceByPubkey: new Map([
        ["bob", "online"],
        ["carol", "away"],
      ]),
    }),
    ["bob"],
  );
});

test("a member with unknown presence is treated as offline for @here", () => {
  // Absent presence must not be read as present, or @here becomes @channel.
  assert.deepEqual(
    resolveMentionAudience({
      scope: MENTION_SCOPE_HERE,
      members: ["bob"],
      presenceByPubkey: new Map(),
    }),
    [],
  );
});

test("audience honours mute and the opt-out per person", () => {
  const audience = resolveMentionAudience({
    scope: MENTION_SCOPE_CHANNEL,
    members: ["quiet", "muted-but-listening", "opted-out"],
    mutedBy: new Set(["muted-but-listening", "opted-out"]),
    optedOutOfMutedChannelMentions: new Set(["opted-out"]),
  });
  assert.deepEqual(audience, ["quiet", "muted-but-listening"]);
});

test("resolution is bounded by current membership, not the event", () => {
  // The whole reason for a marker rather than N `p` tags: someone who joined
  // after the message was sent is still in the audience.
  assert.deepEqual(
    resolveMentionAudience({
      scope: MENTION_SCOPE_CHANNEL,
      members: ["original", "joined-later"],
    }),
    ["original", "joined-later"],
  );
});

test("empty and undefined inputs resolve to nobody rather than throwing", () => {
  assert.deepEqual(
    resolveMentionAudience({
      scope: MENTION_SCOPE_CHANNEL,
      members: undefined,
    }),
    [],
  );
  assert.deepEqual(
    resolveMentionAudience({ scope: "everyone", members: ["alice"] }),
    [],
  );
});
