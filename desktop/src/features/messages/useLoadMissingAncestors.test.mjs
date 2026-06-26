import assert from "node:assert/strict";
import test from "node:test";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import {
  decryptIngestedContent,
  makeDmIngestDecryptor,
} from "@/features/messages/lib/dmCrypto";
import { mergeMessages } from "@/features/messages/hooks";

// base64(0x02 + 98 zero bytes) — minimal valid NIP-44 v2 envelope, so
// looksLikeNip44V2 treats it as an encrypted DM body to decrypt, not legacy
// plaintext. Matches the fixture in messageQueryKeys.test.mjs.
const V2_CIPHERTEXT =
  "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

const DM_CHANNEL = {
  id: "dm-channel-id",
  channelType: "dm",
  participantPubkeys: ["a".repeat(64), "b".repeat(64)],
};
const SELF = "a".repeat(64);
const PEER = "b".repeat(64);

function ancestorEvent(content) {
  return {
    id: "anc".padEnd(64, "0"),
    pubkey: PEER,
    created_at: 4_000,
    kind: 9,
    tags: [["h", DM_CHANNEL.id]],
    content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

// Mirror useLoadMissingAncestors' fetched-ancestor cache write: decrypt the
// fetched event, then mergeMessages into the channel cache. RED form (no
// decrypt) writes the raw event; the fix routes it through the decryptor.
async function loadAncestorIntoCache(store, channel, selfPubkey, event) {
  const decryptIngested = makeDmIngestDecryptor(channel, selfPubkey);
  const key = JSON.stringify(channelMessagesKey(channel.id, selfPubkey));
  const [decrypted] = await decryptIngested([event]);
  const current = store.get(key) ?? [];
  store.set(key, mergeMessages(current, decrypted));
  return key;
}

test("missing DM ancestor is decrypted before it lands in the rendered cache, never raw ciphertext", async () => {
  const store = new Map();

  // A fetched ancestor whose body is valid v2 ciphertext. Routed through the
  // ingest decryptor with a resolved identity, the rendered cache must NOT end
  // holding the raw ciphertext.
  const key = await loadAncestorIntoCache(
    store,
    DM_CHANNEL,
    SELF,
    ancestorEvent(V2_CIPHERTEXT),
  );

  const cached = store.get(key);
  assert.equal(cached.length, 1, "the ancestor is cached");
  assert.notEqual(
    cached[0].content,
    V2_CIPHERTEXT,
    "raw NIP-44 v2 ciphertext must never be written into the rendered DM cache",
  );
});

test("decryptIngestedContent turns a valid-v2 ancestor body into the decrypted plaintext", async () => {
  // Independent proof that the decryptor TRANSFORMS valid-v2 ciphertext (not a
  // no-op passthrough), with an injected decrypt standing in for Tauri NIP-44.
  const content = await decryptIngestedContent(
    ancestorEvent(V2_CIPHERTEXT),
    PEER,
    async () => "decrypted ancestor body",
  );
  assert.equal(content, "decrypted ancestor body");
});

test("missing ancestor in a non-DM channel is passed through unchanged", async () => {
  const store = new Map();
  const streamChannel = {
    id: "stream-channel-id",
    channelType: "stream",
    participantPubkeys: [],
  };
  // Outside a 2-party DM the decryptor is an identity no-op: a v2-shaped body
  // here is NOT an encrypted DM, so it must pass through verbatim.
  const key = await loadAncestorIntoCache(store, streamChannel, SELF, {
    ...ancestorEvent(V2_CIPHERTEXT),
    tags: [["h", streamChannel.id]],
  });
  assert.equal(store.get(key)[0].content, V2_CIPHERTEXT);
});
