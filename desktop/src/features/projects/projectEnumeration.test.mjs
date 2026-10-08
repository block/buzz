import assert from "node:assert/strict";
import test from "node:test";

import {
  enumerateProjectEvents,
  buildProjectHomeFromFetcher,
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

test("buildProjectHomeFromFetcher scopes startup lookup to the active channel", async () => {
  const owner = "a".repeat(64);
  const channelId = "11111111-1111-4111-8111-111111111111";
  const repositoryAddress = `30617:${owner}:relay`;
  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: owner,
    created_at: 200,
    content: "",
    tags: [
      ["d", "relay"],
      ["name", "Relay"],
      ["buzz-channel", channelId],
      ["a", repositoryAddress],
    ],
  };
  const repositoryEvent = {
    id: "r".repeat(64),
    kind: 30617,
    pubkey: owner,
    created_at: 100,
    content: "",
    tags: [
      ["d", "relay"],
      ["name", "Relay"],
      ["buzz-channel", channelId],
    ],
  };
  const calls = [];
  const fetchExhaustively = async (kinds, extraFilter) => {
    calls.push({ kinds, extraFilter });
    if (kinds.includes(30621)) return [projectEvent];
    if (kinds.includes(30617)) return [repositoryEvent];
    return [];
  };

  const project = await buildProjectHomeFromFetcher(
    fetchExhaustively,
    channelId,
    { viewerPubkey: owner },
  );

  assert.equal(project?.projectChannelId, channelId);
  assert.deepEqual(calls[0], {
    kinds: [30621],
    extraFilter: { "#buzz-channel": [channelId] },
  });
  assert.deepEqual(calls[1], {
    kinds: [30617],
    extraFilter: { "#buzz-channel": [channelId] },
  });
  assert.deepEqual(calls[2], {
    kinds: [5],
    extraFilter: { "#a": [`30621:${owner}:relay`, repositoryAddress] },
  });
});

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

// ── Tombstone scoping ────────────────────────────────────────────────────────
//
// Kind:5 is the app-wide NIP-09 deletion kind (every deleted chat message is
// one), so the tombstone fetch must be scoped server-side with `#a` filters
// on the announcement coordinates rather than crawling the relay's entire
// deletion history.

test("buildProjectsFromFetcher scopes the tombstone fetch to announcement coordinates", async () => {
  const OWNER = "a".repeat(64);

  const projectEvent = {
    id: "p".repeat(64),
    kind: 30621,
    pubkey: OWNER,
    created_at: 200,
    content: "",
    tags: [["d", "proj"]],
  };
  const repoEvent = {
    id: "r".repeat(64),
    kind: 30617,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [
      ["d", "repo"],
      ["name", "repo"],
    ],
  };

  const deletionCalls = [];
  const fetchExhaustively = async (kinds, extraFilter) => {
    if (kinds.includes(5)) {
      deletionCalls.push(extraFilter);
      return [];
    }
    if (kinds.includes(30621)) return [projectEvent];
    if (kinds.includes(30617)) return [repoEvent];
    return [];
  };

  await buildProjectsFromFetcher(fetchExhaustively);

  assert.equal(deletionCalls.length, 1, "one scoped tombstone query");
  assert.deepEqual(deletionCalls[0], {
    "#a": [`30621:${OWNER}:proj`, `30617:${OWNER}:repo`],
  });
});

test("buildProjectsFromFetcher skips the tombstone fetch when there are no announcements", async () => {
  let deletionQueried = false;
  const fetchExhaustively = async (kinds) => {
    if (kinds.includes(5)) deletionQueried = true;
    return [];
  };

  const projects = await buildProjectsFromFetcher(fetchExhaustively);

  assert.deepEqual(projects, []);
  assert.equal(
    deletionQueried,
    false,
    "no coordinates means no tombstone query at all",
  );
});

test("buildProjectsFromFetcher still suppresses deleted heads via the scoped fetch", async () => {
  const OWNER = "a".repeat(64);

  const repoEvent = {
    id: "r".repeat(64),
    kind: 30617,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [
      ["d", "repo"],
      ["name", "repo"],
    ],
  };
  const deletionEvent = {
    id: "5".repeat(64),
    kind: 5,
    pubkey: OWNER,
    created_at: 150,
    content: "",
    tags: [["a", `30617:${OWNER}:repo`]],
  };

  const fetchExhaustively = async (kinds, extraFilter) => {
    if (kinds.includes(5)) {
      return extraFilter?.["#a"]?.includes(`30617:${OWNER}:repo`)
        ? [deletionEvent]
        : [];
    }
    if (kinds.includes(30617)) return [repoEvent];
    return [];
  };

  const projects = await buildProjectsFromFetcher(fetchExhaustively);
  assert.deepEqual(projects, [], "deleted repo must not surface as a project");
});

test("enumerateProjectEvents stops paginating once its signal aborts", async () => {
  // 3 full pages of 2 → without an abort the enumeration would fetch all of
  // them plus boundary drains. Abort after the first page: the loop must
  // throw before requesting another page.
  const events = [
    relayEvent("a", 1_000),
    relayEvent("b", 900),
    relayEvent("c", 800),
    relayEvent("d", 700),
    relayEvent("e", 600),
    relayEvent("f", 500),
  ];
  const controller = new AbortController();
  let pageFetches = 0;
  const fetchPage = async (filter) => {
    pageFetches += 1;
    const page = await fetcherFor(events)(filter);
    controller.abort();
    return page;
  };

  await assert.rejects(
    enumerateProjectEvents(fetchPage, [30617], 2, undefined, controller.signal),
    (error) => error.name === "AbortError",
  );
  assert.equal(pageFetches, 1);
});
