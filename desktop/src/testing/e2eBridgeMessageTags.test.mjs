import assert from "node:assert/strict";
import test from "node:test";

import { mockSendMessageExtraTags } from "./e2eBridgeMessageTags.ts";

const IMETA = ["imeta", "url https://relay.example/a.png"];
const EMOJI = ["emoji", "party", "https://relay.example/party.png"];
const MENTION = ["mention", "a".repeat(64)];
const LINK_PREVIEW = ["link-preview", "snapshot", "https://example.com"];
const SENT_FROM_THREAD = ["buzz:sent-from-thread", "b".repeat(64)];
const FORWARDED_FROM = [
  "buzz:forwarded-from",
  "c".repeat(64),
  "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
  "d".repeat(64),
];

test("the mock send echoes every independently validated tag kind", () => {
  assert.deepEqual(
    mockSendMessageExtraTags({
      mediaTags: [IMETA],
      emojiTags: [EMOJI],
      mentionTags: [MENTION],
      linkPreviewTags: [LINK_PREVIEW],
      sentFromThreadTag: SENT_FROM_THREAD,
      forwardedFromTag: FORWARDED_FROM,
      suppressLinkPreviews: true,
    }),
    [
      IMETA,
      EMOJI,
      MENTION,
      LINK_PREVIEW,
      SENT_FROM_THREAD,
      FORWARDED_FROM,
      ["link-preview", "none"],
    ],
  );
});

test("a forwarded copy keeps its provenance on the stored E2E event", () => {
  // Without this the mock drops the tag the real command signs, and the
  // "Forwarded from …" attribution is unreachable from any E2E spec.
  assert.deepEqual(
    mockSendMessageExtraTags({ forwardedFromTag: FORWARDED_FROM }),
    [FORWARDED_FROM],
  );
});

test("an ordinary send carries no extra tags", () => {
  assert.deepEqual(mockSendMessageExtraTags({}), []);
  assert.deepEqual(
    mockSendMessageExtraTags({
      mediaTags: null,
      emojiTags: null,
      mentionTags: null,
      linkPreviewTags: null,
      sentFromThreadTag: null,
      forwardedFromTag: null,
    }),
    [],
  );
});
