import assert from "node:assert/strict";
import test from "node:test";

import { addProjectRepository } from "./useAddProjectRepository.ts";

// ── Partial-publish retry heals through the real mutation ───────────────────
//
// Round-3 review finding: the resume-aware dominated-write guard is only
// meaningful if a retry can actually reach it. After a partial publish (the
// project head landed, the repository event did not), the live head's
// created_at is necessarily NEWER than the cached snapshot — the failed
// mutation never updated the cache. A template-builder unit test cannot catch
// an early unconditional guard throwing first, so these tests run the whole
// exported mutation with stubbed relay/publish seams.

const OWNER = "a".repeat(64);
const ACCESS_CHANNEL = "11111111-1111-4111-8111-111111111111";
const EXISTING_ADDRESS = `30617:${OWNER}:desktop`;
const NEW_ADDRESS = `30617:${OWNER}:mobile`;

function makeProject(overrides = {}) {
  return {
    id: `30621:${OWNER}:platform`,
    createdAt: 100,
    dtag: "platform",
    owner: OWNER,
    accessChannelId: ACCESS_CHANNEL,
    repositoryAddresses: [EXISTING_ADDRESS],
    repositories: [],
    ...overrides,
  };
}

/** Live head as left by a partial publish: newer than the cached snapshot
 * (created_at 101 > 100) and already referencing the new coordinate. */
function makeDanglingLiveHead() {
  return {
    id: "f".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 101,
    content: "",
    tags: [
      ["d", "platform"],
      ["buzz-channel", ACCESS_CHANNEL],
      ["a", EXISTING_ADDRESS],
      ["a", NEW_ADDRESS],
    ],
    sig: "0".repeat(128),
  };
}

function makeRepositoryEvent(createdAt) {
  return {
    id: "b".repeat(64),
    kind: 30617,
    pubkey: OWNER,
    created_at: createdAt,
    content: "",
    tags: [
      ["d", "mobile"],
      ["name", "Mobile"],
      ["buzz-channel", ACCESS_CHANNEL],
    ],
    sig: "0".repeat(128),
  };
}

/** fetchEvents stub: live project head exists (newer than cache), and no
 * repository head exists at the new coordinate — the dangling-member state. */
function makeFetchEvents(liveHead) {
  return async (filter) => {
    if (filter.kinds?.includes(30621)) return [liveHead];
    return [];
  };
}

test("retry after a partial publish heals the dangling member instead of throwing (local path)", async () => {
  const published = [];
  const result = await addProjectRepository(
    {
      accessChannelId: ACCESS_CHANNEL,
      name: "Mobile",
      project: makeProject(),
    },
    {
      fetchEvents: makeFetchEvents(makeDanglingLiveHead()),
      publishOwnerAnnouncement: async (input) => {
        published.push(input);
        return {
          event: makeRepositoryEvent(input.createdAt),
          publicationError: null,
        };
      },
    },
  );

  // The heal publishes ONLY the missing repository event — republishing the
  // project head would advance created_at for nothing.
  assert.deepEqual(
    published.map((input) => input.kind),
    [30617],
    "resume must publish exactly one repository event and no project head",
  );
  assert.equal(result.repository.dtag, "mobile");
  assert.ok(
    result.project.repositoryAddresses.includes(NEW_ADDRESS),
    "the healed project must list the repaired coordinate",
  );
});

test("retry after a partial publish heals through the owner-agent path", async () => {
  const batches = [];
  const result = await addProjectRepository(
    {
      accessChannelId: ACCESS_CHANNEL,
      name: "Mobile",
      ownerControlAgentPubkey: "c".repeat(64),
      project: makeProject(),
    },
    {
      fetchEvents: makeFetchEvents(makeDanglingLiveHead()),
      publishOwnedAgentAnnouncements: async (_agent, templates) => {
        batches.push(templates);
        return templates.map((template) =>
          makeRepositoryEvent(template.createdAt),
        );
      },
    },
  );

  assert.equal(batches.length, 1);
  assert.deepEqual(
    batches[0].map((template) => template.kind),
    [30617],
    "resume must send only the missing repository event to the owner agent",
  );
  assert.equal(result.repository.dtag, "mobile");
});

test("a genuinely dominated write (no dangling member) still throws", async () => {
  // Live head is newer than the cache but does NOT reference the coordinate:
  // another session advanced the project. The guard must still block the
  // write instead of clobbering it.
  const liveHead = {
    ...makeDanglingLiveHead(),
    tags: [
      ["d", "platform"],
      ["buzz-channel", ACCESS_CHANNEL],
      ["a", EXISTING_ADDRESS],
    ],
  };

  await assert.rejects(
    addProjectRepository(
      {
        accessChannelId: ACCESS_CHANNEL,
        name: "Mobile",
        project: makeProject(),
      },
      {
        fetchEvents: makeFetchEvents(liveHead),
        publishOwnerAnnouncement: async () => {
          throw new Error("must not publish past the dominated-write guard");
        },
      },
    ),
    /updated by another session/,
  );
});

test("a dangling member with an unmoved head still resumes", async () => {
  // Crash-recovery shape: the partial state can also be observed with a
  // cache that already caught up (created_at equal). Resume must not depend
  // on the head being newer.
  const liveHead = { ...makeDanglingLiveHead(), created_at: 100 };
  const published = [];

  const result = await addProjectRepository(
    {
      accessChannelId: ACCESS_CHANNEL,
      name: "Mobile",
      project: makeProject(),
    },
    {
      fetchEvents: makeFetchEvents(liveHead),
      publishOwnerAnnouncement: async (input) => {
        published.push(input);
        return {
          event: makeRepositoryEvent(input.createdAt),
          publicationError: null,
        };
      },
    },
  );

  assert.deepEqual(
    published.map((input) => input.kind),
    [30617],
  );
  assert.equal(result.repository.dtag, "mobile");
});
