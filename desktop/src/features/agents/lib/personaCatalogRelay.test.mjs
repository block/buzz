import assert from "node:assert/strict";
import test from "node:test";

import { catalogPersonasFromPublications } from "./personaCatalogRelay.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

function publication(overrides = {}) {
  return {
    eventId: "event-1",
    ownerPubkey: ALICE,
    sourcePersonaId: "reviewer",
    createdAt: 1,
    agent: {
      displayName: "Relay Reviewer",
      avatarUrl: null,
      systemPrompt: "Review changes.",
      runtime: null,
      model: null,
      provider: null,
      namePool: [],
      respondTo: null,
      parallelism: null,
    },
    ...overrides,
  };
}

test("a pending local share does not appear before relay confirmation", () => {
  const localPersona = {
    id: "local-reviewer",
    displayName: "Local Reviewer",
    avatarUrl: null,
    systemPrompt: "Review local changes.",
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: true,
    sourceTeam: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-07-26T00:00:00.000Z",
    updatedAt: "2026-07-26T00:00:00.000Z",
  };

  const personas = catalogPersonasFromPublications([], [localPersona], ALICE);
  assert.deepEqual(personas, []);
});

function localPersona(overrides = {}) {
  return {
    id: "local-1",
    displayName: "Relay Reviewer",
    avatarUrl: null,
    systemPrompt: "Review changes.",
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    sourceTeam: null,
    catalogSource: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-07-26T00:00:00.000Z",
    updatedAt: "2026-07-26T00:00:00.000Z",
    ...overrides,
  };
}

// The duplicate-add bug: a copy of Alice's entry carries a fresh local UUID, so
// matching by id finds nothing and the catalog offers "Add" again. Only the
// stored catalogSource coordinate links the copy back to the publication.
test("test_added_foreign_catalog_entry_keeps_publisher_identity_and_local_selection", () => {
  const publisherAvatar = "https://relay.example/publisher.png";
  const publications = [
    publication({
      agent: { ...publication().agent, avatarUrl: publisherAvatar },
    }),
  ];
  const copy = localPersona({
    id: "a-fresh-uuid",
    displayName: "Locally Renamed Reviewer",
    avatarUrl: "https://relay.example/local-copy.png",
    systemPrompt: "Locally edited instructions.",
    catalogSource: { ownerPubkey: ALICE, personaId: "reviewer" },
  });

  const personas = catalogPersonasFromPublications(publications, [copy], BOB);

  assert.equal(personas.length, 1);
  assert.equal(
    personas[0].id,
    "a-fresh-uuid",
    "the projection must retain the existing local copy's linkage id",
  );
  assert.equal(
    personas[0].isActive,
    true,
    "an added foreign entry must read as already selected",
  );
  assert.equal(personas[0].displayName, "Relay Reviewer");
  assert.equal(personas[0].avatarUrl, publisherAvatar);
  assert.equal(personas[0].systemPrompt, "Review changes.");
});

test("test_foreign_entry_with_no_local_copy_stays_unselected", () => {
  const publications = [publication()];
  // A same-named local persona with no provenance is a different agent.
  const unrelated = localPersona({ id: "unrelated" });

  const personas = catalogPersonasFromPublications(
    publications,
    [unrelated],
    BOB,
  );

  assert.equal(personas[0].id, `catalog:${ALICE}:reviewer`);
  assert.equal(personas[0].isActive, false);
});

// Provenance is per-owner: the same d-tag under a different publisher is a
// different agent, so a copy of Alice's must not mask Bob's entry.
test("test_catalog_source_match_is_scoped_to_the_publishing_owner", () => {
  const publications = [publication({ ownerPubkey: BOB })];
  const copyOfAlices = localPersona({
    id: "copy-of-alices",
    catalogSource: { ownerPubkey: ALICE, personaId: "reviewer" },
  });

  const personas = catalogPersonasFromPublications(
    publications,
    [copyOfAlices],
    ALICE,
  );

  assert.equal(personas[0].id, `catalog:${BOB}:reviewer`);
  assert.equal(personas[0].isActive, false);
});

test("test_own_publication_still_resolves_by_local_id", () => {
  const publications = [publication()];
  const own = localPersona({ id: "reviewer", shared: true });

  const personas = catalogPersonasFromPublications(publications, [own], ALICE);

  assert.equal(personas[0].id, "reviewer");
  assert.equal(personas[0].catalogSource.isOwn, true);
});
