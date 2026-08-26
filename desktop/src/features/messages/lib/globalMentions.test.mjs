import assert from "node:assert/strict";
import { test } from "node:test";

import {
  MENTION_SCOPE_CHANNEL,
  MENTION_SCOPE_HERE,
  MENTION_SCOPE_TAG,
  detectMentionScope,
  mentionScopeOf,
  shouldNotifyForMentionScope,
} from "./globalMentions.mjs";

// --- detecting a scope written in the message text ---

test("@channel and @here are detected in the composed text", () => {
  assert.equal(detectMentionScope("@channel standup moved"), "channel");
  assert.equal(detectMentionScope("heads up @here"), "here");
  assert.equal(detectMentionScope("mid @here sentence works"), "here");
});

test("detection is case-insensitive", () => {
  assert.equal(detectMentionScope("@Channel please read"), "channel");
  assert.equal(detectMentionScope("@HERE"), "here");
});

test("@channel wins when both appear", () => {
  // The wider audience wins: resolving this the other way would silently drop
  // people the author plainly meant to reach.
  assert.equal(detectMentionScope("@here and @channel"), "channel");
});

test("ordinary text is not a broadcast", () => {
  assert.equal(detectMentionScope("nothing to see"), null);
  assert.equal(detectMentionScope(""), null);
  assert.equal(detectMentionScope(undefined), null);
});

test("lookalikes do not broadcast to the whole channel", () => {
  // The expensive false positive: an address or path silently paging everyone.
  assert.equal(detectMentionScope("mail support@channel-ops.example"), null);
  assert.equal(detectMentionScope("see docs/@here-ish"), null);
  assert.equal(detectMentionScope("@channels"), null);
  assert.equal(detectMentionScope("@herewith"), null);
  assert.equal(detectMentionScope("email me@here.com"), null);
});

test("a username mention is not a broadcast", () => {
  assert.equal(detectMentionScope("@priya can you look"), null);
});

test("trailing punctuation still counts as the end of the word", () => {
  assert.equal(detectMentionScope("please read this @here."), "here");
  assert.equal(detectMentionScope("@channel, standup moved"), "channel");
  assert.equal(detectMentionScope("(@here)"), "here");
});

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
