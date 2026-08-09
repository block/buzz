import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  fetchTeamCatalogPublications,
  teamCatalogPublicationsFromEvents,
  teamEventIsShared,
} from "./teamCatalogRelay.ts";

const OWNER = "a".repeat(64);

function eventId(label) {
  return createHash("sha256").update(label).digest("hex");
}

function teamEvent({
  id = "team",
  createdAt = 1,
  owner = OWNER,
  dTag = "review-team",
  shared = true,
  sharedTag,
  content,
  kind = 30178,
} = {}) {
  return {
    id: eventId(id),
    pubkey: owner,
    created_at: createdAt,
    kind,
    tags: [
      ["d", dTag],
      ...(shared
        ? [sharedTag ?? ["shared", "true"]]
        : sharedTag
          ? [sharedTag]
          : []),
    ],
    content:
      content ??
      JSON.stringify({
        v: 1,
        name: "Review Team",
        members: [
          {
            member_key: "persona-1",
            display_name: "Reviewer",
            system_prompt: "private prompt must not escape the projection",
            provider: "private-provider",
          },
        ],
      }),
    sig: "f".repeat(128),
  };
}

test("strict shared v1 parsing returns only safe team projection fields", () => {
  const [publication] = teamCatalogPublicationsFromEvents([teamEvent({})]);
  assert.equal(publication.name, "Review Team");
  assert.equal(publication.memberCount, 1);
  assert.deepEqual(publication.memberKeys, ["persona-1"]);
  assert.equal("systemPrompt" in publication, false);
  assert.equal("provider" in publication, false);
});

test("malformed tags, wrong kind, and wrong schema are rejected", () => {
  const exact = teamEvent({ id: "exact" });
  assert.equal(teamEventIsShared(exact), true);

  for (const [index, sharedTag] of [
    ["shared"],
    ["shared", "false"],
    ["shared", "true", "extra"],
  ].entries()) {
    const malformed = teamEvent({
      id: `shared-${index}`,
      shared: false,
      sharedTag,
    });
    assert.equal(teamEventIsShared(malformed), false);
    assert.deepEqual(teamCatalogPublicationsFromEvents([malformed]), []);
  }

  const duplicateD = teamEvent({ id: "duplicate-d" });
  duplicateD.tags.push(["d", "another"]);
  assert.deepEqual(teamCatalogPublicationsFromEvents([duplicateD]), []);

  const duplicateShared = teamEvent({ id: "duplicate-shared" });
  duplicateShared.tags.push(["shared", "true"]);
  assert.deepEqual(teamCatalogPublicationsFromEvents([duplicateShared]), []);

  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      teamEvent({ id: "wrong-kind", kind: 30176 }),
      teamEvent({
        id: "wrong-version",
        content: '{"v":2,"name":"Future","members":[]}',
      }),
    ]),
    [],
  );
});

test("duplicate member projections are rejected", () => {
  const content = JSON.stringify({
    v: 1,
    name: "Unsafe Team",
    members: [
      { member_key: "same", display_name: "One" },
      { member_key: "same", display_name: "Again" },
    ],
  });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([teamEvent({ content })]),
    [],
  );
});

test("bounded names, member keys, and content are enforced", () => {
  const oversizedName = JSON.stringify({
    v: 1,
    name: "x".repeat(257),
    members: [],
  });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      teamEvent({ id: "oversized-name", content: oversizedName }),
    ]),
    [],
  );

  const oversizedMemberKey = JSON.stringify({
    v: 1,
    name: "Bounded Team",
    members: [{ member_key: "x".repeat(129), display_name: "Worker" }],
  });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      teamEvent({ id: "oversized-member-key", content: oversizedMemberKey }),
    ]),
    [],
  );

  const oversizedContent = JSON.stringify({
    v: 1,
    name: "Bounded Team",
    description: "x".repeat(4_097),
    members: [],
  });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      teamEvent({ id: "oversized-content-field", content: oversizedContent }),
    ]),
    [],
  );
});

test("display text rejects control and bidi characters", () => {
  const content = JSON.stringify({
    v: 1,
    name: "Safe\u202eTeam",
    members: [],
  });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      teamEvent({ id: "bidi-name", content }),
    ]),
    [],
  );
});

test("one owner's large heads cannot exhaust validation for another owner", () => {
  const largeContent = JSON.stringify({
    v: 1,
    name: "Large Team",
    members: [],
    padding: "x".repeat(190_000),
  });
  const noisyOwner = "b".repeat(64);
  const noisyHeads = Array.from({ length: 24 }, (_, index) =>
    teamEvent({
      id: `large-${index}`,
      owner: noisyOwner,
      dTag: `large-${index}`,
      createdAt: 10_000 - index,
      content: largeContent,
    }),
  );
  const otherOwnerHead = teamEvent({
    id: "other-owner",
    owner: "c".repeat(64),
    dTag: "other-owner-team",
    createdAt: 1,
  });

  assert.equal(
    teamCatalogPublicationsFromEvents([...noisyHeads, otherOwnerHead]).some(
      (publication) => publication.ownerPubkey === "c".repeat(64),
    ),
    true,
  );
});

test("newer invalid or unshared head hides older shared head", () => {
  const older = teamEvent({ id: "older", createdAt: 1 });
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      older,
      teamEvent({ id: "unshared", createdAt: 2, shared: false }),
    ]),
    [],
  );
  assert.deepEqual(
    teamCatalogPublicationsFromEvents([
      older,
      teamEvent({ id: "invalid", createdAt: 2, content: "{}" }),
    ]),
    [],
  );
});

test("full catalog pages are read past the relay limit and event ids are deduped", async (t) => {
  t.after(() => mock.restoreAll());
  const first = Array.from({ length: 500 }, (_, index) =>
    teamEvent({
      id: `page-${index}`,
      createdAt: 1_000 - index,
      dTag: `team-${index}`,
    }),
  );
  const second = [
    first.at(-1),
    teamEvent({ id: "older", createdAt: 1, dTag: "older-team" }),
  ];
  const filters = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    filters.push(filter);
    return Promise.resolve(filters.length === 1 ? first : second);
  });

  const publications = await fetchTeamCatalogPublications();
  assert.equal(publications.length, 501);
  assert.deepEqual(filters, [
    { kinds: [30178], limit: 500 },
    { kinds: [30178], limit: 500, until: 501 },
  ]);
});
