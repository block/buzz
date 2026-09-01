import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectRelatedChannelRevisionTemplate,
  publishProjectRelatedChannelRevision,
  removeProjectRelatedChannel,
} from "./projectRelatedChannelRevision.ts";

const HOME = "11111111-1111-4111-8111-111111111111";
const RELATED = "22222222-2222-4222-8222-222222222222";
const OWNER = "a".repeat(64);

function project(overrides = {}) {
  return {
    baseRevisionId: "a".repeat(64),
    effectiveRevisionId: "b".repeat(64),
    legacy: false,
    projectAddress: `30621:${OWNER}:buzz`,
    projectChannelId: HOME,
    relatedChannelIds: [],
    ...overrides,
  };
}

test("buildProjectRelatedChannelRevisionTemplate builds add and remove CAS operations", () => {
  assert.deepEqual(
    buildProjectRelatedChannelRevisionTemplate(
      project(),
      RELATED,
      "add-related-channel",
    ),
    {
      kind: 47001,
      content: "",
      tags: [
        ["a", `30621:${OWNER}:buzz`],
        ["base", "a".repeat(64)],
        ["e", "b".repeat(64)],
        ["op", "add-related-channel"],
        ["channel", RELATED],
        ["buzz-related-channel", RELATED],
      ],
    },
  );
  assert.equal(
    buildProjectRelatedChannelRevisionTemplate(
      project({ relatedChannelIds: [RELATED] }),
      RELATED,
      "remove-related-channel",
    ).tags[3][1],
    "remove-related-channel",
  );
});

test("buildProjectRelatedChannelRevisionTemplate rejects stale and invalid mutations", () => {
  assert.throws(
    () =>
      buildProjectRelatedChannelRevisionTemplate(
        project({ effectiveRevisionId: undefined }),
        RELATED,
        "add-related-channel",
      ),
    /Refresh/,
  );
  assert.throws(
    () =>
      buildProjectRelatedChannelRevisionTemplate(
        project(),
        HOME,
        "add-related-channel",
      ),
    /home channel/,
  );
  assert.throws(
    () =>
      buildProjectRelatedChannelRevisionTemplate(
        project({ relatedChannelIds: [RELATED] }),
        RELATED,
        "add-related-channel",
      ),
    /already related/,
  );
});

test("removeProjectRelatedChannel publishes a remove revision and advances local state", async () => {
  const calls = [];
  const updated = await removeProjectRelatedChannel(
    project({ createdAt: 200, relatedChannelIds: [RELATED] }),
    RELATED,
    {
      signEvent: async (template) => ({
        ...template,
        id: "c".repeat(64),
        pubkey: OWNER,
        created_at: 123,
      }),
      publishEvent: async (...args) => calls.push(args),
    },
  );

  assert.equal(calls.length, 1);
  assert.equal(calls[0][0].tags[3][1], "remove-related-channel");
  assert.deepEqual(updated.relatedChannelIds, []);
  assert.equal(updated.effectiveRevisionId, "c".repeat(64));
  assert.equal(updated.createdAt, 200);
});

test("publishProjectRelatedChannelRevision can sign as a locally managed owner", async () => {
  const calls = [];
  const event = await publishProjectRelatedChannelRevision(
    project(),
    RELATED,
    "add-related-channel",
    {
      publishOwnerAnnouncement: async (input) => {
        calls.push(input);
        return {
          event: {
            ...input,
            id: "c".repeat(64),
            pubkey: OWNER,
            created_at: 123,
          },
          publicationError: null,
        };
      },
    },
    { signAsManagedOwner: true },
  );

  assert.equal(calls[0].targetOwner, OWNER);
  assert.equal(calls[0].kind, 47001);
  assert.equal(event.pubkey, OWNER);
});

test("publishProjectRelatedChannelRevision can delegate signing to a controlled owner agent", async () => {
  const calls = [];
  const event = await publishProjectRelatedChannelRevision(
    project(),
    RELATED,
    "add-related-channel",
    {
      publishOwnedAgentAnnouncements: async (...args) => {
        calls.push(args);
        return [
          {
            ...args[1][0],
            id: "c".repeat(64),
            pubkey: OWNER,
            created_at: 123,
          },
        ];
      },
    },
    { ownerControlAgentPubkey: OWNER },
  );

  assert.equal(calls[0][0], OWNER);
  assert.equal(calls[0][1][0].kind, 47001);
  assert.equal(event.pubkey, OWNER);
});
