import assert from "node:assert/strict";
import test from "node:test";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { makeDmIngestDecryptor } from "@/features/messages/lib/dmCrypto";
import { mergeTimelineCacheMessages } from "@/features/messages/hooks";

// Minimal valid NIP-44 v2 envelope (see messageQueryKeys.test.mjs).
const V2_CIPHERTEXT_LIVE =
  "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

const DM_CHANNEL_LIVE = {
  id: "dm-live-channel-id",
  channelType: "dm",
  participantPubkeys: ["a".repeat(64), "b".repeat(64)],
};
const SELF_LIVE = "a".repeat(64);
const PEER_LIVE = "b".repeat(64);

function liveDmEvent(content) {
  return {
    id: "live".padEnd(64, "0"),
    pubkey: PEER_LIVE,
    created_at: 6_000,
    kind: 9,
    tags: [["h", DM_CHANNEL_LIVE.id]],
    content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

// Mirror useLiveChannelUpdates' handleIncomingMessage timeline-cache write:
// decrypt via makeDmIngestDecryptor, then merge under the `if (!current)`
// guard. RED form (no decrypt) merges the raw event.
async function liveTimelineWrite(store, dmChannel, currentPubkey, event) {
  const key = JSON.stringify(
    channelMessagesKey(event.tags[0][1], currentPubkey),
  );
  const [decrypted] = await makeDmIngestDecryptor(
    dmChannel,
    currentPubkey,
  )([event]);
  const current = store.get(key);
  if (!current) {
    return key;
  }
  store.set(key, mergeTimelineCacheMessages(current, decrypted));
  return key;
}

test("live dual-write decrypts a DM event so it cannot clobber the decrypted copy with ciphertext", async () => {
  const store = new Map();
  const key = JSON.stringify(channelMessagesKey(DM_CHANNEL_LIVE.id, SELF_LIVE));

  // The decrypting useChannelSubscription seeds the bucket with plaintext-X.
  store.set(key, [{ ...liveDmEvent("dinner at 7?"), content: "dinner at 7?" }]);

  // The live dual-write then fires for the SAME event id, carrying raw
  // ciphertext. On the id collision the last writer wins — so without
  // decryption this would replace plaintext-X with ciphertext-X.
  await liveTimelineWrite(
    store,
    DM_CHANNEL_LIVE,
    SELF_LIVE,
    liveDmEvent(V2_CIPHERTEXT_LIVE),
  );

  const cached = store.get(key);
  assert.equal(cached.length, 1, "id collision keeps a single row");
  assert.notEqual(
    cached[0].content,
    V2_CIPHERTEXT_LIVE,
    "the live dual-write must not clobber plaintext with raw ciphertext",
  );
});

test("live dual-write never SEEDS an absent DM bucket (guard preserved)", async () => {
  const store = new Map();
  // Bucket not yet seeded by the decrypting path: the guard returns early, so
  // even a ciphertext event must not create a bucket here.
  const key = await liveTimelineWrite(
    store,
    DM_CHANNEL_LIVE,
    SELF_LIVE,
    liveDmEvent(V2_CIPHERTEXT_LIVE),
  );
  assert.equal(
    store.has(key),
    false,
    "an absent bucket is never seeded by the dual-write",
  );
});
