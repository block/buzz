import assert from "node:assert/strict";
import test from "node:test";

import {
  CATCH_UP_LOOKBACK_SECONDS,
  describeCatchUpCoverage,
  eligibleCatchUpRepoAddresses,
  extractCatchUpTaskReferences,
  isCatchUpConfigured,
  isCatchUpConflictError,
  isCatchUpSourceChangedError,
  MAX_CATCH_UP_TASK_REFERENCES,
  resolveCatchUpBaseline,
  resolveCatchUpRange,
  resolveCatchUpSourceState,
  selectCatchUpAssignedTasks,
  snapshotCitedBodies,
  validateCatchUpSettings,
} from "./threadCatchUp.ts";

const ROOT = "f".repeat(64);
const ME = "deadbeef".repeat(8);
const ALICE = "a".repeat(64);
const OWNER = `${"0".repeat(63)}1`;
const NOW = 2_000_000;
const HOUR = 3_600;
const id = (n) => n.toString(16).padStart(64, "0");

function snapshot(overrides = {}) {
  return {
    channelId: "channel-1",
    rootEventId: ROOT,
    channelReadAt: null,
    channelReady: true,
    threadReadAt: null,
    threadReady: true,
    replyReadAt: new Map(),
    repliesReady: true,
    ...overrides,
  };
}

const replies = [
  { id: ROOT, createdAt: NOW - 10 * HOUR, pubkey: ALICE },
  { id: id(1), createdAt: NOW - 5 * HOUR, pubkey: ALICE },
  { id: id(2), createdAt: NOW - 3 * HOUR, pubkey: ME },
  { id: id(3), createdAt: NOW - 2 * HOUR, pubkey: ALICE },
  { id: id(4), createdAt: NOW - HOUR, pubkey: ALICE },
];

const baselineCases = [
  ["no snapshot is unresolved", null, { kind: "unresolved" }],
  [
    "channel read state not ready is unresolved",
    snapshot({ channelReady: false, channelReadAt: NOW - HOUR }),
    { kind: "unresolved" },
  ],
  [
    "thread read state not ready is unresolved",
    snapshot({ threadReady: false }),
    { kind: "unresolved" },
  ],
  [
    "reply markers captured before readiness are unresolved",
    snapshot({ repliesReady: false }),
    { kind: "unresolved" },
  ],
  ["ready with no markers is absent", snapshot(), { kind: "absent" }],
  [
    "markers past the read-state horizon are expired",
    snapshot({ channelReadAt: NOW - 8 * 24 * HOUR }),
    { kind: "expired", previousReadAt: NOW - 8 * 24 * HOUR },
  ],
  [
    "channel frontier starts at the first unread reply from someone else",
    snapshot({ channelReadAt: NOW - 4 * HOUR }),
    {
      kind: "marker",
      since: NOW - 2 * HOUR,
      previousReadAt: NOW - 4 * HOUR,
      unreadCount: 2,
    },
  ],
  [
    "own replies never count as unread",
    snapshot({ threadReadAt: NOW - 4 * HOUR }),
    {
      kind: "marker",
      since: NOW - 2 * HOUR,
      previousReadAt: NOW - 4 * HOUR,
      unreadCount: 2,
    },
  ],
  [
    "per-reply markers from collapsed branches are honored",
    snapshot({
      channelReadAt: NOW - 6 * HOUR,
      replyReadAt: new Map([
        [id(1), NOW - 5 * HOUR],
        [id(3), NOW - 2 * HOUR],
      ]),
    }),
    {
      kind: "marker",
      since: NOW - HOUR,
      previousReadAt: NOW - 2 * HOUR,
      unreadCount: 1,
    },
  ],
  [
    "everything read before the visit is caught up",
    snapshot({ channelReadAt: NOW - 30 }),
    { kind: "caught-up", previousReadAt: NOW - 30 },
  ],
];

for (const [name, input, expected] of baselineCases) {
  test(`baseline: ${name}`, () => {
    assert.deepEqual(
      resolveCatchUpBaseline({
        snapshot: input,
        replies,
        currentPubkey: ME.toUpperCase(),
        nowSeconds: NOW,
      }),
      expected,
    );
  });
}

test("range: explicit lookbacks use the request time, baseline needs a marker", () => {
  assert.deepEqual(
    resolveCatchUpRange(
      { kind: "lookback", lookback: "day" },
      { kind: "absent" },
      NOW,
    ),
    { since: NOW - CATCH_UP_LOOKBACK_SECONDS.day, until: NOW },
  );
  assert.deepEqual(
    resolveCatchUpRange(
      { kind: "lookback", lookback: "week" },
      { kind: "unresolved" },
      NOW,
    ),
    { since: NOW - 7 * 24 * HOUR, until: NOW },
  );
  assert.equal(
    resolveCatchUpRange(
      { kind: "baseline" },
      { kind: "expired", previousReadAt: 1 },
      NOW,
    ),
    null,
  );
  assert.deepEqual(
    resolveCatchUpRange(
      { kind: "baseline" },
      { kind: "marker", since: 42, previousReadAt: 40, unreadCount: 1 },
      NOW,
    ),
    { since: 42, until: NOW },
  );
});

const coverageCases = [
  [
    { messageCount: 4, inputMessageCount: 4, truncated: false },
    "Read 4 retrieved messages",
  ],
  [
    { messageCount: 1, inputMessageCount: 1, truncated: false },
    "Read 1 retrieved message",
  ],
  [
    { messageCount: 300, inputMessageCount: 200, truncated: true },
    "Partial: read 200 of 300 retrieved messages",
  ],
  [
    { messageCount: 9, inputMessageCount: 7, truncated: false },
    "Partial: read 7 of 9 retrieved messages",
  ],
];

for (const [coverage, expected] of coverageCases) {
  test(`coverage: ${expected}`, () => {
    assert.equal(
      describeCatchUpCoverage({ ...coverage, warnings: [] }),
      expected,
    );
  });
}

const settingsCases = [
  [
    {
      baseUrl: "http://127.0.0.1:11434/v1",
      model: "llama3",
      confirmedLocal: true,
    },
    null,
  ],
  [
    { baseUrl: "http://[::1]:8080/v1", model: "qwen", confirmedLocal: true },
    null,
  ],
  [
    { baseUrl: "https://127.0.0.1:11434/v1", model: "m", confirmedLocal: true },
    /http:\/\//,
  ],
  [
    { baseUrl: "http://localhost:11434/v1", model: "m", confirmedLocal: true },
    /numeric loopback/,
  ],
  [
    { baseUrl: "http://10.0.0.2:11434", model: "m", confirmedLocal: true },
    /numeric loopback/,
  ],
  [
    { baseUrl: "http://127.0.0.256", model: "m", confirmedLocal: true },
    /full endpoint/,
  ],
  [
    { baseUrl: "http://127.example.com", model: "m", confirmedLocal: true },
    /numeric loopback/,
  ],
  [
    { baseUrl: "http://user:pw@127.0.0.1", model: "m", confirmedLocal: true },
    /credentials/,
  ],
  [
    { baseUrl: "http://127.0.0.1/v1?key=x", model: "m", confirmedLocal: true },
    /query/,
  ],
  [{ baseUrl: "file:///tmp/model", model: "m", confirmedLocal: true }, /http/],
  [
    { baseUrl: "127.0.0.1:11434", model: "m", confirmedLocal: true },
    /full endpoint|http/,
  ],
  [{ baseUrl: "http://127.0.0.1", model: " ", confirmedLocal: true }, /model/],
  [
    { baseUrl: "http://127.0.0.1", model: "m", confirmedLocal: false },
    /Confirm/,
  ],
];

for (const [settings, expected] of settingsCases) {
  test(`settings: ${settings.baseUrl} ${settings.model} ${settings.confirmedLocal}`, () => {
    const error = validateCatchUpSettings(settings);
    if (expected === null) {
      assert.equal(error, null);
      assert.equal(isCatchUpConfigured(settings), true);
    } else {
      assert.match(error ?? "", expected);
      assert.equal(isCatchUpConfigured(settings), false);
    }
  });
}

test("errors: native prefixes, not substrings, select the recovery path", () => {
  const cases = [
    ["conflict: recap changed", true, false],
    ["source_changed: a cited message was edited", false, true],
    ["source_changed", false, true],
    ["relay said conflict: no", false, false],
    ["failed: source_changed upstream", false, false],
    ["source_changedx", false, false],
  ];
  for (const [message, conflict, sourceChanged] of cases) {
    const error = new Error(message);
    assert.equal(isCatchUpConflictError(error), conflict, message);
    assert.equal(isCatchUpSourceChangedError(error), sourceChanged, message);
  }
  assert.equal(isCatchUpSourceChangedError("source_changed"), false);
});

test("sources: snapshot detects edits and unloaded messages", () => {
  const recap = {
    points: [
      { id: "p1", category: "decision", text: "x", sourceIds: [id(1), id(2)] },
    ],
  };
  const initial = new Map([
    [id(1), "first"],
    [id(2), "second"],
  ]);
  const frozen = snapshotCitedBodies(recap, initial);
  const later = new Map([[id(1), "first (edited)"]]);
  assert.equal(resolveCatchUpSourceState(id(1), later, frozen), "changed");
  assert.equal(resolveCatchUpSourceState(id(2), later, frozen), "unloaded");
  assert.equal(resolveCatchUpSourceState(id(1), initial, frozen), "current");
  assert.equal(resolveCatchUpSourceState(id(1), later, null), "current");
});

const link = (issue, dtag = "relay", owner = OWNER) =>
  `buzz://issue?id=${issue}&owner=${owner}&d=${dtag}`;

test("task references: only strict links in recap sources, deduped in order", () => {
  const messages = [
    { id: id(1), body: `Please take ${link(id(10))}.` },
    { id: id(2), body: `Not cited ${link(id(11))}` },
    { id: id(3), body: `again ${link(id(10))}, plus (${link(id(12), "web")})` },
    { id: id(4), body: "buzz://issue?id=nothex&owner=x&d=relay" },
    { id: id(5), body: `buzz://pr?id=${id(13)}&owner=${OWNER}&d=relay` },
    { id: id(6), body: "Assigned to you: the deploy" },
  ];
  const allowed = new Set([id(1), id(3), id(4), id(5), id(6)]);
  assert.deepEqual(extractCatchUpTaskReferences(messages, allowed), [
    {
      issueId: id(10),
      owner: OWNER,
      dtag: "relay",
      repoAddress: `30617:${OWNER}:relay`,
      sourceIds: [id(1), id(3)],
    },
    {
      issueId: id(12),
      owner: OWNER,
      dtag: "web",
      repoAddress: `30617:${OWNER}:web`,
      sourceIds: [id(3)],
    },
  ]);
});

test("task references: capped", () => {
  const body = Array.from(
    { length: MAX_CATCH_UP_TASK_REFERENCES + 5 },
    (_, i) => link(id(100 + i)),
  ).join(" ");
  const refs = extractCatchUpTaskReferences(
    [{ id: id(1), body }],
    new Set([id(1)]),
  );
  assert.equal(refs.length, MAX_CATCH_UP_TASK_REFERENCES);
});

function task(overrides = {}) {
  return {
    id: id(10),
    repoAddress: `30617:${OWNER}:relay`,
    assignees: [ME],
    status: "In Progress",
    ...overrides,
  };
}

const reference = {
  issueId: id(10),
  owner: OWNER,
  dtag: "relay",
  repoAddress: `30617:${OWNER}:relay`,
  sourceIds: [id(1)],
};

const assignedCases = [
  ["assigned, open, eligible repo", task(), true],
  ["not found", null, false],
  ["assigned to someone else", task({ assignees: [ALICE] }), false],
  ["done", task({ status: "Done" }), false],
  ["closed", task({ status: "Closed" }), false],
  [
    "repo coordinate mismatch",
    task({ repoAddress: `30617:${OWNER}:web` }),
    false,
  ],
  ["different issue id", task({ id: id(11) }), false],
  [
    "repo identifier differs by case",
    task({ repoAddress: `30617:${OWNER}:Relay` }),
    false,
  ],
  [
    "repo owner hex differs only by case",
    task({ repoAddress: `30617:${OWNER.toUpperCase()}:relay` }),
    true,
  ],
];

for (const [name, value, expected] of assignedCases) {
  test(`assigned: ${name}`, () => {
    const selected = selectCatchUpAssignedTasks({
      references: [reference],
      tasks: new Map([[id(10), value]]),
      eligibleRepoAddresses: new Set([`30617:${OWNER}:relay`]),
      currentPubkey: ME.toUpperCase(),
    });
    assert.equal(selected.length, expected ? 1 : 0);
  });
}

test("assigned: repo outside the reader's projects is excluded", () => {
  assert.deepEqual(
    selectCatchUpAssignedTasks({
      references: [reference],
      tasks: new Map([[id(10), task()]]),
      eligibleRepoAddresses: eligibleCatchUpRepoAddresses([
        { repositories: [{ repoAddress: `30617:${OWNER}:web` }] },
      ]),
      currentPubkey: ME,
    }),
    [],
  );
});
