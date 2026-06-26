import assert from "node:assert/strict";
import test from "node:test";

import { makeDmIngestDecryptor } from "@/features/messages/lib/dmCrypto";
import { mergeMessages } from "@/features/messages/hooks";

// Minimal valid NIP-44 v2 envelope (see messageQueryKeys.test.mjs).
const V2_CIPHERTEXT =
  "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

const DM_CHANNEL = {
  id: "dm-target-channel-id",
  channelType: "dm",
  participantPubkeys: ["a".repeat(64), "b".repeat(64)],
};
const STREAM_CHANNEL = {
  id: "stream-target-channel-id",
  channelType: "stream",
  participantPubkeys: [],
};
const SELF = "a".repeat(64);
const PEER = "b".repeat(64);

function targetEvent(content) {
  return {
    id: "tgt".padEnd(64, "0"),
    pubkey: PEER,
    created_at: 5_000,
    kind: 9,
    tags: [["h", DM_CHANNEL.id]],
    content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

// Mirror useDecryptedTargetMessageEvents + ChannelScreen's resolvedMessages
// reduce: decrypt the target events, then merge them into the (already
// decrypted) current messages exactly as the render path does. The RED form
// skips the decrypt and merges the raw target.
async function resolveRenderedTimeline(
  channel,
  selfPubkey,
  currentMessages,
  targetMessageEvents,
) {
  const decryptIngested = makeDmIngestDecryptor(channel, selfPubkey);
  const decryptedTargets = await decryptIngested(targetMessageEvents);
  return decryptedTargets.reduce(mergeMessages, currentMessages);
}

test("DM route-target event is decrypted before it reaches the rendered timeline, never raw ciphertext", async () => {
  const rendered = await resolveRenderedTimeline(
    DM_CHANNEL,
    SELF,
    [],
    [targetEvent(V2_CIPHERTEXT)],
  );

  assert.equal(rendered.length, 1, "the target row is spliced into the list");
  assert.notEqual(
    rendered[0].content,
    V2_CIPHERTEXT,
    "a DM route-target must not render raw ciphertext",
  );
});

test("DM route-target does not clobber an already-decrypted copy of the same id with ciphertext", async () => {
  // The decrypted cache already holds plaintext-X (e.g. history fetched it).
  const decryptedCopy = {
    ...targetEvent("dinner at 7?"),
    content: "dinner at 7?",
  };

  const rendered = await resolveRenderedTimeline(
    DM_CHANNEL,
    SELF,
    [decryptedCopy],
    [targetEvent(V2_CIPHERTEXT)],
  );

  assert.equal(rendered.length, 1, "id collision keeps a single row");
  assert.notEqual(
    rendered[0].content,
    V2_CIPHERTEXT,
    "the raw target must not clobber the decrypted copy",
  );
});

test("non-DM route-target with a v2-shaped body passes through verbatim", async () => {
  const streamTarget = {
    ...targetEvent(V2_CIPHERTEXT),
    tags: [["h", STREAM_CHANNEL.id]],
  };

  const rendered = await resolveRenderedTimeline(
    STREAM_CHANNEL,
    SELF,
    [],
    [streamTarget],
  );

  assert.equal(
    rendered[0].content,
    V2_CIPHERTEXT,
    "outside a 2-party DM the decryptor is a no-op and content is untouched",
  );
});
