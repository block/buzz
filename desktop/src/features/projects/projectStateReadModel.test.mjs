import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import { buildProjectReadModels } from "./projectModels.ts";

const RELAY_SECRET = new Uint8Array(32).fill(9);
const RELAY_PUBKEY = getPublicKey(RELAY_SECRET);
const OWNER = "a".repeat(64);
const IDENTITY_ID = "b".repeat(64);
const COORDINATE = `30621:${OWNER}:buzz`;
const RELATED = "22222222-2222-4222-8222-222222222222";

function identityEvent() {
  return {
    id: IDENTITY_ID,
    sig: "owner-signature-not-reverified-by-this-read-model",
    kind: 30621,
    pubkey: OWNER,
    created_at: 200,
    content: "",
    tags: [
      ["d", "buzz"],
      ["name", "Owner name"],
    ],
  };
}

function projection({ deleted = false, identityId = IDENTITY_ID } = {}) {
  return finalizeEvent(
    {
      kind: 30623,
      created_at: 201,
      content: JSON.stringify({
        v: 1,
        deleted,
        project_tags: deleted
          ? []
          : [
              ["d", "buzz"],
              ["name", "Effective name"],
              ["buzz-related-channel", RELATED],
            ],
      }),
      tags: [
        ["d", "c".repeat(64)],
        ["a", COORDINATE],
        ["rev", "2"],
        ["e", identityId, "", "identity"],
        ["e", "d".repeat(64), "", "change"],
      ],
    },
    RELAY_SECRET,
  );
}

test("read model uses verified effective tags while preserving owner identity fields", () => {
  const [project] = buildProjectReadModels({
    projectEvents: [identityEvent()],
    projectStateEvents: [projection()],
    repositoryEvents: [],
    relayPubkey: RELAY_PUBKEY,
  });
  assert.equal(project.name, "Effective name");
  assert.equal(project.owner, OWNER);
  assert.equal(project.createdAt, 200);
  assert.equal(project.id, COORDINATE);
  assert.deepEqual(project.relatedChannelIds, [RELATED]);
});

test("read model suppresses matching deleted state and falls back for a stale identity marker", () => {
  assert.deepEqual(
    buildProjectReadModels({
      projectEvents: [identityEvent()],
      projectStateEvents: [projection({ deleted: true })],
      repositoryEvents: [],
      relayPubkey: RELAY_PUBKEY,
    }),
    [],
  );

  const [fallback] = buildProjectReadModels({
    projectEvents: [identityEvent()],
    projectStateEvents: [projection({ identityId: "e".repeat(64) })],
    repositoryEvents: [],
    relayPubkey: RELAY_PUBKEY,
  });
  assert.equal(fallback.name, "Owner name");
  assert.deepEqual(fallback.relatedChannelIds, []);
});
