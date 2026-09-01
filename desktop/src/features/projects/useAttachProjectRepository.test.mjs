import assert from "node:assert/strict";
import test from "node:test";

import { attachProjectRepository } from "./useAttachProjectRepository.ts";

const OWNER = "a".repeat(64);
const CACHED_BASE = "e".repeat(64);

function project() {
  return {
    baseRevisionId: CACHED_BASE,
    createdAt: 100,
    dtag: "platform",
    effectiveRevisionId: CACHED_BASE,
    id: `30621:${OWNER}:platform`,
    owner: OWNER,
    projectAddress: `30621:${OWNER}:platform`,
    relatedChannelIds: [],
  };
}

const repository = {
  id: `${OWNER}:mobile`,
  dtag: "mobile",
  owner: OWNER,
  repoAddress: `30617:${OWNER}:mobile`,
};

test("attachProjectRepository rejects an equal-timestamp base replacement", async () => {
  let revisionHeadFetches = 0;
  await assert.rejects(
    attachProjectRepository(
      { project: project(), repository },
      {
        fetchEvents: async () => [
          {
            id: "d".repeat(64),
            kind: 30621,
            pubkey: OWNER,
            created_at: 100,
            content: "",
            tags: [["d", "platform"]],
            sig: "0".repeat(128),
          },
        ],
        fetchProjectRevisionHeads: async () => {
          revisionHeadFetches += 1;
          return [];
        },
      },
    ),
    /updated by another session/,
  );
  assert.equal(revisionHeadFetches, 1);
});
