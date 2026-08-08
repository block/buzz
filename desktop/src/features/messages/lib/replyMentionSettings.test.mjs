import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_REPLY_MENTION_SETTINGS,
  readStoredReplyMentionSettings,
  sanitizeReplyMentionSettings,
} from "./replyMentionSettings.ts";

const PK_A = "aa".repeat(32);
const PK_B = "bb".repeat(32);

test("sanitizeReplyMentionSettings returns defaults for non-objects", () => {
  for (const value of [null, undefined, 42, "nope", []]) {
    assert.deepEqual(sanitizeReplyMentionSettings(value), {
      autoMentionRepliedTo: true,
      mentionPrefixPubkeys: [],
    });
  }
});

test("sanitizeReplyMentionSettings defaults autoMentionRepliedTo when not a boolean", () => {
  const result = sanitizeReplyMentionSettings({
    autoMentionRepliedTo: "yes",
    mentionPrefixPubkeys: [],
  });
  assert.equal(result.autoMentionRepliedTo, true);
});

test("sanitizeReplyMentionSettings preserves an explicit false toggle", () => {
  const result = sanitizeReplyMentionSettings({
    autoMentionRepliedTo: false,
    mentionPrefixPubkeys: [],
  });
  assert.equal(result.autoMentionRepliedTo, false);
});

test("sanitizeReplyMentionSettings filters, lowercases, and dedupes prefix pubkeys", () => {
  const result = sanitizeReplyMentionSettings({
    autoMentionRepliedTo: true,
    mentionPrefixPubkeys: [
      PK_A.toUpperCase(),
      ` ${PK_A} `, // whitespace + duplicate of the first entry
      PK_B,
      "not-a-pubkey",
      123,
      null,
    ],
  });
  assert.deepEqual(result.mentionPrefixPubkeys, [PK_A, PK_B]);
});

test("sanitizeReplyMentionSettings drops malformed prefix pubkeys", () => {
  const result = sanitizeReplyMentionSettings({
    autoMentionRepliedTo: true,
    mentionPrefixPubkeys: ["zz".repeat(32), "short", PK_A.slice(0, 63)],
  });
  assert.deepEqual(result.mentionPrefixPubkeys, []);
});

test("readStoredReplyMentionSettings falls back to defaults outside the browser", () => {
  // node:test has no window.localStorage — the reader must not throw and must
  // return the defaults.
  assert.deepEqual(
    readStoredReplyMentionSettings(PK_A),
    DEFAULT_REPLY_MENTION_SETTINGS,
  );
});
