import assert from "node:assert/strict";
import test from "node:test";

import { catalogTeamsFromPublications } from "./teamCatalogRelay.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

// Relay paging, signature verification, head selection, and content parsing
// now live in `team_catalog.rs` and are covered by `team_catalog_tests.rs`.
// This suite exercises only the renderer's remaining job: linking a verified
// publication to a local team and deciding ownership and sort order.

function publication(overrides = {}) {
  return {
    eventId: "event-1",
    ownerPubkey: ALICE,
    teamDTag: "squad",
    name: "Review Squad",
    description: null,
    instructions: null,
    members: [
      {
        memberKey: "reviewer",
        displayName: "Relay Reviewer",
        systemPrompt: "Review changes.",
        avatarUrl: null,
        runtime: "goose",
        model: "claude",
        provider: null,
      },
    ],
    ...overrides,
  };
}

function localTeam(overrides = {}) {
  return {
    id: "local-1",
    name: "Review Squad",
    description: null,
    instructions: null,
    personaIds: [],
    isBuiltin: false,
    shared: false,
    catalogSource: null,
    sourceDir: null,
    isSymlink: false,
    symlinkTarget: null,
    version: null,
    createdAt: "2026-07-30T00:00:00.000Z",
    updatedAt: "2026-07-30T00:00:00.000Z",
    ...overrides,
  };
}

test("test_own_publication_resolves_to_the_local_team_by_id", () => {
  const own = localTeam({ id: "squad", shared: true });

  const teams = catalogTeamsFromPublications([publication()], [own], ALICE);

  assert.equal(teams[0].isOwn, true);
  assert.equal(teams[0].localTeam.id, "squad");
});

// The duplicate-add bug: a copy carries a fresh local id, so only the stored
// coordinate links it back to the publication it came from.
test("test_added_foreign_entry_resolves_to_its_local_copy", () => {
  const copy = localTeam({
    id: "a-fresh-uuid",
    catalogSource: { ownerPubkey: ALICE, teamDTag: "squad" },
  });

  const teams = catalogTeamsFromPublications([publication()], [copy], BOB);

  assert.equal(teams[0].isOwn, false);
  assert.equal(teams[0].localTeam.id, "a-fresh-uuid");
});

test("test_foreign_entry_with_no_local_copy_has_no_local_team", () => {
  // A same-named local team with no provenance is a different team.
  const unrelated = localTeam({ id: "unrelated" });

  const teams = catalogTeamsFromPublications([publication()], [unrelated], BOB);

  assert.equal(teams[0].localTeam, null);
});

// Provenance is per-owner: the same d-tag under a different publisher is a
// different team, so a copy of Alice's must not mask Bob's entry.
test("test_catalog_source_match_is_scoped_to_the_publishing_owner", () => {
  const copyOfAlices = localTeam({
    id: "copy-of-alices",
    catalogSource: { ownerPubkey: ALICE, teamDTag: "squad" },
  });

  const teams = catalogTeamsFromPublications(
    [publication({ ownerPubkey: BOB, teamDTag: "bob-team" })],
    [copyOfAlices],
    ALICE,
  );

  assert.equal(teams[0].localTeam, null);
});

// An own team's `d`-tag is its local id, so an id match under another
// publisher's coordinate must not read as already-added.
test("test_local_id_match_under_a_foreign_owner_is_not_a_local_copy", () => {
  const sameId = localTeam({ id: "squad" });

  const teams = catalogTeamsFromPublications(
    [publication({ ownerPubkey: BOB, teamDTag: "squad" })],
    [sameId],
    ALICE,
  );

  assert.equal(teams[0].isOwn, false);
  assert.equal(teams[0].localTeam, null);
});

test("test_identity_pubkey_case_does_not_change_ownership", () => {
  const teams = catalogTeamsFromPublications(
    [publication()],
    [],
    ALICE.toUpperCase(),
  );

  assert.equal(teams[0].isOwn, true);
});

test("test_catalog_entries_are_sorted_by_name", () => {
  const teams = catalogTeamsFromPublications(
    [
      publication({ teamDTag: "zed", name: "Zed Squad" }),
      publication({ teamDTag: "ace", name: "Ace Squad" }),
    ],
    [],
    BOB,
  );

  assert.deepEqual(
    teams.map((team) => team.name),
    ["Ace Squad", "Zed Squad"],
  );
});
