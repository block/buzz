import assert from "node:assert/strict";
import test from "node:test";

import { getSendToChannelSemantics } from "./sendToChannelSemantics.ts";

const SOURCE = "a".repeat(64);
const SIGNER = "b".repeat(64);
const MENTION = "c".repeat(64);

test("send-to-channel preserves supported message semantics", () => {
  const imeta = ["imeta", "url https://relay.example/media/file.png"];
  const emoji = ["emoji", "party", "https://relay.example/party.png"];
  const mention = ["mention", MENTION];
  const preview = ["link-preview", "none"];

  assert.deepEqual(
    getSendToChannelSemantics({
      pubkey: SOURCE,
      signerPubkey: SIGNER,
      tags: [
        ["h", "channel-id"],
        ["e", "thread-root", "", "reply"],
        ["p", SOURCE],
        ["p", SIGNER.toUpperCase()],
        ["p", MENTION.toUpperCase()],
        ["p", MENTION],
        ["p", "not-a-pubkey"],
        imeta,
        emoji,
        mention,
        preview,
        ["client", "source-only-marker"],
      ],
    }),
    {
      mentionPubkeys: [MENTION],
      semanticTags: [imeta, emoji, mention, preview],
    },
  );
});

test("edited messages recompute effective mention recipients from the body", () => {
  const ADDED = "d".repeat(64);
  const profiles = {
    [MENTION]: { displayName: "Alice" },
    [ADDED]: { displayName: "Bob" },
  };

  assert.deepEqual(
    getSendToChannelSemantics(
      {
        body: "Now pinging @Bob",
        edited: true,
        pubkey: SOURCE,
        tags: [
          ["p", MENTION],
          ["p", ADDED],
        ],
      },
      profiles,
    ),
    { mentionPubkeys: [ADDED], semanticTags: [] },
  );
});

test("send-to-channel canonicalizes suppressed link previews", () => {
  const snapshot = ["link-preview", "https://example.com", "snapshot"];
  const suppression = ["link-preview", "none"];

  assert.deepEqual(
    getSendToChannelSemantics({
      pubkey: SOURCE,
      tags: [snapshot, suppression],
    }),
    { mentionPubkeys: [], semanticTags: [suppression] },
  );
});

test("send-to-channel handles messages without semantic tags", () => {
  assert.deepEqual(
    getSendToChannelSemantics({ pubkey: SOURCE, tags: undefined }),
    { mentionPubkeys: [], semanticTags: [] },
  );
});
