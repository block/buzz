import assert from "node:assert/strict";
import test from "node:test";

import {
  assertCanSendMessageToChannel,
  canSendMessageToChannel,
} from "./canSendToChannel.ts";

const CURRENT = "a".repeat(64);
const OWNED_AGENT = "b".repeat(64);
const OTHER_PERSON = "c".repeat(64);
const OTHER_AGENT = "d".repeat(64);

const message = (pubkey) => ({ kind: 9, pubkey });
const profiles = {
  [OWNED_AGENT]: { isAgent: true, ownerPubkey: CURRENT },
  [OTHER_AGENT]: { isAgent: true, ownerPubkey: OTHER_PERSON },
};

test("send-to-channel permits self-authored messages", () => {
  assert.equal(
    canSendMessageToChannel(message(CURRENT), CURRENT, profiles),
    true,
  );
});

test("send-to-channel permits messages from an agent owned by the viewer", () => {
  assert.equal(
    canSendMessageToChannel(message(OWNED_AGENT), CURRENT, profiles),
    true,
  );
});

test("send-to-channel rejects specialized message kinds", () => {
  const diffMessage = { ...message(CURRENT), kind: 40008 };

  assert.equal(canSendMessageToChannel(diffMessage, CURRENT, profiles), false);
  assert.throws(
    () => assertCanSendMessageToChannel(diffMessage, CURRENT, profiles),
    /Only ordinary channel messages/,
  );
});

test("send-to-channel rejects pending messages", () => {
  const pendingMessage = { ...message(CURRENT), pending: true };

  assert.equal(
    canSendMessageToChannel(pendingMessage, CURRENT, profiles),
    false,
  );
  assert.throws(
    () => assertCanSendMessageToChannel(pendingMessage, CURRENT, profiles),
    /finish sending first/,
  );
});

test("send-to-channel rejects third-party people and agents", () => {
  assert.equal(
    canSendMessageToChannel(message(OTHER_PERSON), CURRENT, profiles),
    false,
  );
  assert.equal(
    canSendMessageToChannel(message(OTHER_AGENT), CURRENT, profiles),
    false,
  );
  assert.throws(
    () =>
      assertCanSendMessageToChannel(message(OTHER_AGENT), CURRENT, profiles),
    /only send your own or your agents' messages/,
  );
});
