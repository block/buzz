import assert from "node:assert/strict";
import test from "node:test";

import {
  enumerateProjectEvents,
  buildProjectsFromFetcher,
} from "./projectEnumeration.ts";

function relayEvent(id, createdAt) {
  return {
    id: id.repeat(64),
    kind: 30617,
    pubkey: "a".repeat(64),
    created_at: createdAt,
    content: "",
    tags: [["d", id]],
  };
}

function fetcherFor(events) {
  return async ({ limit, since, until }) =>
    events
      .filter(
        (event) =>
          (since === undefined || event.created_at >= since) &&
          (until === undefined || event.created_at <= until),
      )
      .sort(
        (left, right) =>
          right.created_at - left.created_at || left.id.localeCompare(right.id),
      )
      .slice(0, limit);
}

test("enumerateProjectEvents drains a tied boundary second before advancing", async () => {
  const events = [
    relayEvent("a", 1_000),
    relayEvent("b", 900),
    relayEvent("c", 900),
    relayEvent("d", 800),
  ];

  const result = await enumerateProjectEvents(fetcherFor(events), [30617], 3);

  assert.deepEqual(
    result.map((event) => event.id).sort(),
    events.map((event) => event.id).sort(),
  );
});

test("enumerateProjectEvents refuses to present a truncated boundary as complete", async () => {
  const events = [
    relayEvent("a", 1_000),
    relayEvent("b", 1_000),
    relayEvent("c", 1_000),
  ];

  await assert.rejects(
    enumerateProjectEvents(fetcherFor(events), [30617], 2),
    /cannot exhaustively enumerate/,
  );
});

// ── Tombstone fetch-failure gate ─────────────────────────────────────────────
//
// `buildProjectsFromFetcher` (and therefore `fetchProjects`) must throw rather
// than returning a project list when the kind:5 tombstone enumeration rejects.
// A silent empty-set substitution would resurrect every deleted head served by
// a history-retaining relay.

test("buildProjectsFromFetcher throws when kind-5 tombstone enumeration rejects", async () => {
  const OWNER = "a".repeat(64);
  const DTAG = "relay";

  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 200,
    content: "",
    tags: [["d", DTAG]],
  };
  const repoEvent = {
    id: "r".repeat(64),
    kind: 30617,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [
      ["d", DTAG],
      ["name", DTAG],
    ],
  };

  // Fetcher: succeeds for project/repo kinds, rejects for kind:5 (tombstones).
  const fetchExhaustively = async (kinds) => {
    if (kinds.includes(5)) {
      throw new Error("relay unavailable");
    }
    if (kinds.includes(30621)) return [projectEvent];
    if (kinds.includes(30617)) return [repoEvent];
    return [];
  };

  await assert.rejects(
    buildProjectsFromFetcher(fetchExhaustively),
    /Could not fetch project deletion records/,
    "tombstone fetch failure must propagate as a throw, not an empty deletion set",
  );
});

test("buildProjectsFromFetcher does not throw when tombstone enumeration succeeds with an empty result", async () => {
  // Verify the happy path: no tombstones → returns projects without error.
  const OWNER = "a".repeat(64);

  const fetchExhaustively = async (kinds) => {
    if (kinds.includes(5)) return []; // no deletions
    if (kinds.includes(30621)) return []; // no explicit projects
    if (kinds.includes(30617))
      return [
        {
          id: "r".repeat(64),
          kind: 30617,
          pubkey: OWNER,
          created_at: 100,
          content: "",
          tags: [
            ["d", "relay"],
            ["name", "relay"],
          ],
        },
      ];
    return [];
  };

  const projects = await buildProjectsFromFetcher(fetchExhaustively);
  // Legacy project (unclaimed repo) should appear.
  assert.ok(Array.isArray(projects), "must return a project array");
  assert.equal(
    projects.some((p) => p.legacy),
    true,
    "unclaimed repo must appear as legacy project",
  );
});
