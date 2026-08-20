import assert from "node:assert/strict";
import test from "node:test";

import {
  buildPrompt,
  heuristicActions,
  parseLlmActions,
  summarizeFibre,
} from "./classify.mjs";

const OPEN = [
  {
    id: "f-open",
    kind: "ask",
    title: "Run triage scripts",
    summary: "Vlad asked for the scripts",
    score: 80,
    people: [{ pubkey: "vlad", label: "Vlad" }],
    artifacts: [
      {
        eventId: "root-1",
        threadRootId: "root-1",
        authorLabel: "Vlad",
      },
    ],
    channelName: "hack-project-mesh",
  },
];

const MENTION = {
  eventId: "m1",
  channelId: "c1",
  channelName: "hack-project-mesh",
  threadRootId: null,
  authorPubkey: "vlad",
  authorLabel: "Vlad",
  content: "@jacob can you run the triage scripts before the next build",
  createdAt: 1,
  isDm: false,
  isMention: true,
  isSelf: false,
};

test("heuristic skips short acknowledgements", () => {
  const actions = heuristicActions(
    [{ ...MENTION, eventId: "ack", content: "ok" }],
    [],
    { events: new Map(), authors: new Map(), channels: new Map(), threads: new Map(), examples: [] },
  );
  assert.deepEqual(actions, [{ type: "skip", eventId: "ack" }]);
});

test("heuristic creates an ask for a qualifying mention", () => {
  const actions = heuristicActions([MENTION], [], {
    events: new Map(),
    authors: new Map(),
    channels: new Map(),
    threads: new Map(),
    examples: [],
  });
  assert.equal(actions.length, 1);
  assert.equal(actions[0].type, "create");
  assert.equal(actions[0].kind, "ask");
  assert.deepEqual(actions[0].eventIds, ["m1"]);
  assert.ok(actions[0].score >= 50);
});

test("heuristic updates an open fibre that shares a thread root", () => {
  const actions = heuristicActions(
    [{ ...MENTION, eventId: "m2", threadRootId: "root-1", content: "the scripts are in the repo" }],
    OPEN,
    {
      events: new Map(),
      authors: new Map(),
      channels: new Map(),
      threads: new Map(),
      examples: [],
    },
  );
  assert.deepEqual(actions, [
    { type: "update", fibreId: "f-open", eventIds: ["m2"] },
  ]);
});

test("heuristic skips a message already attached to an open fibre", () => {
  const actions = heuristicActions(
    [{ ...MENTION, eventId: "root-1", threadRootId: "root-1" }],
    OPEN,
    {
      events: new Map(),
      authors: new Map(),
      channels: new Map(),
      threads: new Map(),
      examples: [],
    },
  );
  assert.deepEqual(actions, [{ type: "skip", eventId: "root-1" }]);
});

test("parseLlmActions drops unknown kinds onto fyi and ignores stale fibre ids", () => {
  const actions = parseLlmActions(
    {
      actions: [
        { type: "create", kind: "not-a-kind", title: "x", eventIds: ["m1"], score: 150 },
        { type: "update", fibreId: "missing", eventIds: ["m1"] },
        { type: "update", fibreId: "f-open", eventIds: ["m2"] },
        { type: "skip", eventId: "m3" },
      ],
    },
    OPEN,
  );
  assert.equal(actions[0].kind, "fyi");
  assert.equal(actions[0].score, 100);
  assert.equal(actions[1].type, "update");
  assert.equal(actions[1].fibreId, "f-open");
  assert.equal(actions[2].type, "skip");
});

test("buildPrompt includes open fibres and new messages", () => {
  const prompt = buildPrompt([MENTION], OPEN, { examples: [] });
  assert.match(prompt, /f-open/);
  assert.match(prompt, /Run triage scripts/);
  assert.match(prompt, /m1/);
  assert.match(prompt, /incomplete/);
});

test("summarizeFibre is compact enough for the prompt", () => {
  const summary = summarizeFibre(OPEN[0]);
  assert.deepEqual(summary.eventIds, ["root-1"]);
  assert.deepEqual(summary.people, ["Vlad"]);
});
