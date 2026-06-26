import assert from "node:assert/strict";
import test from "node:test";

import { resolveTargetRenderEvents } from "@/features/messages/useDecryptedTargetMessageEvents";

// useDecryptedTargetMessageEvents decides what the rendered timeline sees for
// deep-link / search-hit target events. The dangerous branch is a DM whose
// identity has not resolved yet (cold-start selfPubkey === undefined): the hook
// must HOLD BACK (render nothing) until decrypt is possible, never pass the raw
// NIP-44 v2 ciphertext target through to the render merge. Unlike the cache
// path, these targets are component state merged directly onto resolvedMessages
// with no [...,null] vs [...,pubkey] bucket-orphaning to catch a raw write.

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

function rawTarget() {
  return {
    id: "tgt".padEnd(64, "0"),
    pubkey: "b".repeat(64),
    created_at: 5_000,
    kind: 9,
    tags: [["h", DM_CHANNEL.id]],
    content: V2_CIPHERTEXT,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

test("DM with unresolved identity holds back the raw target instead of rendering ciphertext", () => {
  // selfPubkey undefined -> decrypt not yet possible -> decryptedEvents still [].
  const rendered = resolveTargetRenderEvents(DM_CHANNEL, [rawTarget()], []);

  assert.deepEqual(
    rendered,
    [],
    "a DM must hold back its targets until identity resolves; raw ciphertext must never reach the render path",
  );
});

test("DM with resolved identity renders the decrypted targets", () => {
  const decrypted = [{ ...rawTarget(), content: "dinner at 7?" }];
  const rendered = resolveTargetRenderEvents(
    DM_CHANNEL,
    [rawTarget()],
    decrypted,
  );

  assert.deepEqual(
    rendered,
    decrypted,
    "once identity resolves the hook surfaces the decrypted target set",
  );
});

test("non-DM target passes through synchronously with no held-back frame", () => {
  const streamTarget = { ...rawTarget(), tags: [["h", STREAM_CHANNEL.id]] };
  const rendered = resolveTargetRenderEvents(
    STREAM_CHANNEL,
    [streamTarget],
    [],
  );

  assert.deepEqual(
    rendered,
    [streamTarget],
    "outside a DM there is nothing to decrypt, so the target splices on first paint",
  );
});

test("group DM (>2 participants) is not peer-encrypted and passes through", () => {
  const groupDm = {
    id: "group-dm-channel-id",
    channelType: "dm",
    participantPubkeys: ["a".repeat(64), "b".repeat(64), "c".repeat(64)],
  };
  const groupTarget = { ...rawTarget(), tags: [["h", groupDm.id]] };
  const rendered = resolveTargetRenderEvents(groupDm, [groupTarget], []);

  assert.deepEqual(
    rendered,
    [groupTarget],
    "a group DM is not 2-party peer-encrypted, so holding it back would drop its targets",
  );
});
