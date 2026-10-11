import assert from "node:assert/strict";
import test from "node:test";

import {
  agentLibraryQueryTerms,
  filterAgentLibrary,
  isAgentLibraryFilterActive,
  isPlainEscapeKey,
  matchesAgentLibraryQuery,
  resolveAgentLibraryStatus,
} from "./agentLibraryFilter.ts";

const notArchived = () => false;
const noPresence = () => undefined;
// Presence unknown (relay disconnected or snapshot not loaded yet).
const offline = { isArchived: notArchived, getAvailability: noPresence };

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Scout",
    description: null,
    systemPrompt: "You research codebases.",
    model: null,
    provider: null,
    runtime: null,
    ...overrides,
  };
}

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "scout-1",
    personaId: "persona-1",
    status: "stopped",
    systemPrompt: null,
    model: null,
    provider: null,
    runtime: null,
    ...overrides,
  };
}

test("status resolution covers every lifecycle combination", () => {
  const cases = [
    { agents: [], expected: "not_started" },
    { agents: [agent({ status: "stopped" })], expected: "stopped" },
    { agents: [agent({ status: "not_deployed" })], expected: "stopped" },
    { agents: [agent({ status: "running" })], expected: "running" },
    // A deployment receipt without confirmed presence is not running.
    { agents: [agent({ status: "deployed" })], expected: "stopped" },
    {
      agents: [agent({ status: "stopped" }), agent({ status: "running" })],
      expected: "running",
    },
  ];
  for (const { agents, expected } of cases) {
    assert.equal(resolveAgentLibraryStatus(agents, offline), expected);
  }
});

test("relay presence makes a stopped card running, as the card face shows", () => {
  const rows = [
    { status: "stopped", presence: "online", expected: "running" },
    { status: "stopped", presence: "away", expected: "running" },
    { status: "stopped", presence: "offline", expected: "stopped" },
    { status: "stopped", presence: undefined, expected: "stopped" },
    { status: "not_deployed", presence: "online", expected: "running" },
    { status: "running", presence: "offline", expected: "running" },
    { status: "running", presence: undefined, expected: "running" },
    // `deployed` is a retained receipt: a shut-down remote agent keeps it
    // while the relay reports offline, and the card dot shows Offline.
    { status: "deployed", presence: "offline", expected: "stopped" },
    { status: "deployed", presence: undefined, expected: "stopped" },
    { status: "deployed", presence: "online", expected: "running" },
    { status: "deployed", presence: "away", expected: "running" },
  ];
  for (const { status, presence, expected } of rows) {
    assert.equal(
      resolveAgentLibraryStatus([agent({ status })], {
        isArchived: notArchived,
        getAvailability: () => presence,
      }),
      expected,
      `${status} + ${presence}`,
    );
  }
});

test("a persona card's bucket follows the instance its face shows", () => {
  // pickProfileAgent prefers a lifecycle-active instance, then name order.
  // The card renders alpha's avatar and Start button, so beta's presence
  // must not move the card onto the Running chip.
  const alpha = agent({ pubkey: "1".repeat(64), name: "alpha" });
  const beta = agent({ pubkey: "2".repeat(64), name: "beta" });
  const presence = (lookup) => ({
    isArchived: notArchived,
    getAvailability: (pubkey) => lookup[pubkey],
  });
  assert.equal(
    resolveAgentLibraryStatus(
      [alpha, beta],
      presence({ [beta.pubkey]: "online", [alpha.pubkey]: "offline" }),
    ),
    "stopped",
  );
  assert.equal(
    resolveAgentLibraryStatus(
      [alpha, beta],
      presence({ [beta.pubkey]: "offline", [alpha.pubkey]: "online" }),
    ),
    "running",
  );
  // Both lifecycle-active: alpha (deployed, offline) wins by name and the
  // card shows its Offline dot, so the bucket is Stopped even though beta
  // runs locally.
  assert.equal(
    resolveAgentLibraryStatus(
      [
        { ...alpha, status: "deployed" },
        { ...beta, status: "running" },
      ],
      presence({ [alpha.pubkey]: "offline" }),
    ),
    "stopped",
  );
  // The active instance wins the pick over a stopped sibling.
  assert.equal(
    resolveAgentLibraryStatus(
      [
        { ...alpha, status: "stopped" },
        { ...beta, status: "running" },
      ],
      offline,
    ),
    "running",
  );
});

test("archived instances never count toward a card's status", () => {
  const archivedPubkey = "b".repeat(64);
  const isArchived = (pubkey) => pubkey === archivedPubkey;
  const context = { isArchived, getAvailability: () => "online" };
  assert.equal(
    resolveAgentLibraryStatus(
      [agent({ pubkey: archivedPubkey, status: "running" })],
      { isArchived, getAvailability: noPresence },
    ),
    "not_started",
  );
  // Presence of an archived identity is never consulted either.
  assert.equal(
    resolveAgentLibraryStatus(
      [agent({ pubkey: archivedPubkey, status: "stopped" })],
      context,
    ),
    "not_started",
  );
  assert.equal(
    resolveAgentLibraryStatus(
      [
        agent({ pubkey: archivedPubkey, status: "running" }),
        agent({ status: "stopped" }),
      ],
      { isArchived, getAvailability: noPresence },
    ),
    "stopped",
  );
});

test("query terms are lowercased, whitespace-split, and empty-safe", () => {
  assert.deepEqual(agentLibraryQueryTerms("  Code  Review\n"), [
    "code",
    "review",
  ]);
  assert.deepEqual(agentLibraryQueryTerms("   "), []);
});

test("every term must match somewhere across the fields", () => {
  const fields = ["Scout", null, "You research codebases.", undefined];
  assert.equal(matchesAgentLibraryQuery([], fields), true);
  assert.equal(matchesAgentLibraryQuery(["research"], fields), true);
  assert.equal(matchesAgentLibraryQuery(["scout", "codebases"], fields), true);
  assert.equal(matchesAgentLibraryQuery(["scout", "deploy"], fields), false);
  assert.equal(matchesAgentLibraryQuery(["SCOUT"], ["scout"]), false);
});

test("search reaches persona and instance system prompts", () => {
  const cards = {
    groups: [
      {
        persona: persona({ systemPrompt: "Review every pull request." }),
        agents: [],
      },
      {
        persona: persona({
          id: "persona-2",
          displayName: "Ralph",
          systemPrompt: "Write release notes.",
        }),
        agents: [
          agent({
            personaId: "persona-2",
            systemPrompt: "Override: triage incoming bugs.",
          }),
        ],
      },
    ],
    ungrouped: [
      agent({
        pubkey: "c".repeat(64),
        personaId: null,
        name: "legacy",
        systemPrompt: "Summarize standup threads.",
      }),
    ],
    unknown: [],
  };

  const byPersonaPrompt = filterAgentLibrary(
    cards,
    { query: "pull request", status: "all" },
    offline,
  );
  assert.deepEqual(
    byPersonaPrompt.groups.map((group) => group.persona.displayName),
    ["Scout"],
  );
  assert.equal(byPersonaPrompt.ungrouped.length, 0);

  const byInstancePrompt = filterAgentLibrary(
    cards,
    { query: "triage", status: "all" },
    offline,
  );
  assert.deepEqual(
    byInstancePrompt.groups.map((group) => group.persona.displayName),
    ["Ralph"],
  );

  const byStandalonePrompt = filterAgentLibrary(
    cards,
    { query: "standup", status: "all" },
    offline,
  );
  assert.equal(byStandalonePrompt.groups.length, 0);
  assert.deepEqual(
    byStandalonePrompt.ungrouped.map((candidate) => candidate.name),
    ["legacy"],
  );
});

test("search stays within what the box promises: names, descriptions, instructions", () => {
  const cards = {
    groups: [
      {
        persona: persona({
          description: "Ships release notes.",
          model: "claude-opus-4-5",
          provider: "anthropic",
          runtime: "claude",
        }),
        agents: [],
      },
    ],
    ungrouped: [],
    unknown: [],
  };
  const hit = (query) =>
    filterAgentLibrary(cards, { query, status: "all" }, offline).groups.length;
  assert.equal(hit("release notes"), 1);
  assert.equal(hit("scout"), 1);
  // Raw model/provider/runtime ids are outside what the box promises (a card
  // without a description paints a formatted model label instead), and
  // matching them would light up every card on a runtime for "claude".
  assert.equal(hit("claude"), 0);
  assert.equal(hit("anthropic"), 0);
});

test("archived instance text is not searchable", () => {
  const archivedPubkey = "d".repeat(64);
  const cards = {
    groups: [
      {
        persona: persona(),
        agents: [
          agent({
            pubkey: archivedPubkey,
            systemPrompt: "Secret archived override.",
          }),
        ],
      },
    ],
    ungrouped: [],
    unknown: [],
  };
  const result = filterAgentLibrary(
    cards,
    { query: "archived", status: "all" },
    {
      isArchived: (pubkey) => pubkey === archivedPubkey,
      getAvailability: noPresence,
    },
  );
  assert.equal(result.groups.length, 0);
});

test("status chips narrow the library and counts follow the query", () => {
  const cards = {
    groups: [
      { persona: persona(), agents: [agent({ status: "running" })] },
      {
        persona: persona({ id: "persona-2", displayName: "Ralph" }),
        agents: [agent({ personaId: "persona-2", status: "stopped" })],
      },
      {
        persona: persona({ id: "persona-3", displayName: "Reviewer" }),
        agents: [],
      },
    ],
    ungrouped: [
      agent({
        pubkey: "e".repeat(64),
        personaId: null,
        name: "custom-runner",
        status: "running",
      }),
    ],
    unknown: [
      agent({
        pubkey: "f".repeat(64),
        personaId: "missing",
        name: "orphan",
        status: "stopped",
      }),
    ],
  };

  const all = filterAgentLibrary(cards, { query: "", status: "all" }, offline);
  assert.deepEqual(all.counts, {
    all: 5,
    running: 2,
    stopped: 2,
    not_started: 1,
  });

  const running = filterAgentLibrary(
    cards,
    { query: "", status: "running" },
    offline,
  );
  assert.deepEqual(
    running.groups.map((group) => group.persona.displayName),
    ["Scout"],
  );
  assert.deepEqual(
    running.ungrouped.map((candidate) => candidate.name),
    ["custom-runner"],
  );
  assert.equal(running.unknown.length, 0);

  const notStarted = filterAgentLibrary(
    cards,
    { query: "", status: "not_started" },
    offline,
  );
  assert.deepEqual(
    notStarted.groups.map((group) => group.persona.displayName),
    ["Reviewer"],
  );
  assert.equal(notStarted.ungrouped.length, 0);
  assert.equal(notStarted.unknown.length, 0);

  // Counts ignore the selected chip (every fixture name contains "r")...
  const stoppedR = filterAgentLibrary(
    cards,
    { query: "r", status: "stopped" },
    offline,
  );
  assert.deepEqual(stoppedR.counts, {
    all: 5,
    running: 2,
    stopped: 2,
    not_started: 1,
  });
  // ...but follow the query, so a chip never advertises cards the search box
  // has already excluded.
  const queried = filterAgentLibrary(
    cards,
    { query: "orphan", status: "stopped" },
    offline,
  );
  assert.deepEqual(queried.counts, {
    all: 1,
    running: 0,
    stopped: 1,
    not_started: 0,
  });
  assert.deepEqual(
    queried.unknown.map((candidate) => candidate.name),
    ["orphan"],
  );
});

test("filter activity ignores whitespace-only queries", () => {
  assert.equal(isAgentLibraryFilterActive({ query: "", status: "all" }), false);
  assert.equal(
    isAgentLibraryFilterActive({ query: "   ", status: "all" }),
    false,
  );
  assert.equal(isAgentLibraryFilterActive({ query: "x", status: "all" }), true);
  assert.equal(
    isAgentLibraryFilterActive({ query: "", status: "running" }),
    true,
  );
});

test("only an unmodified Escape clears the search box", () => {
  const none = {
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
  };
  assert.equal(isPlainEscapeKey({ key: "Escape", ...none }), true);
  assert.equal(isPlainEscapeKey({ key: "Enter", ...none }), false);
  for (const modifier of ["altKey", "ctrlKey", "metaKey", "shiftKey"]) {
    assert.equal(
      isPlainEscapeKey({ key: "Escape", ...none, [modifier]: true }),
      false,
      modifier,
    );
  }
});
