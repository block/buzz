import assert from "node:assert/strict";
import test from "node:test";

import { parseDraftPayload, serializeDraftPayload } from "./draftPayload.ts";

const pubkey = "a".repeat(64);
const draft = {
  content: "cross-device draft",
  selectionStart: 0,
  selectionEnd: 18,
  channelId: "550e8400-e29b-41d4-a716-446655440000",
  createdAt: "2026-07-13T00:00:00.000Z",
  updatedAt: "2026-07-13T00:01:00.000Z",
  pendingImeta: [],
  spoileredAttachmentUrls: [],
  status: "active",
};

test("test_draft_payload_round_trip_recovers_thread_context", () => {
  const plaintext = serializeDraftPayload(
    "thread:root-event",
    draft,
    pubkey,
    100,
  );
  const decoded = parseDraftPayload(plaintext, pubkey, 9, draft.updatedAt);

  assert.ok(decoded);
  assert.equal(decoded.draftKey, "thread:root-event");
  assert.equal(decoded.draft.content, draft.content);
  assert.equal(decoded.draft.channelId, draft.channelId);
});

test("test_draft_payload_foreign_author_is_rejected", () => {
  const plaintext = serializeDraftPayload(
    draft.channelId,
    draft,
    "b".repeat(64),
    100,
  );
  assert.equal(parseDraftPayload(plaintext, pubkey, 9, draft.updatedAt), null);
});
