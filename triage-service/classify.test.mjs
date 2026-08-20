import assert from "node:assert/strict";
import test from "node:test";

import {
  buildPrompt,
  constrainActions,
  heuristicActions,
  limitSummary,
  narrativeSummary,
  parseLlmActions,
  summarizeFibre,
} from "./classify.mjs";

const EMPTY_LESSONS = {
  events: new Map(),
  authors: new Map(),
  channels: new Map(),
  threads: new Map(),
  examples: [],
};

const OPEN = [
  {
    id: "f-open",
    kind: "ask",
    title: "Run triage scripts",
    summary: "Vlad asked for the scripts",
    score: 80,
    channelId: "c1",
    channelName: "hack-project-mesh",
    isDm: false,
    people: [{ pubkey: "vlad", label: "Vlad" }],
    artifacts: [
      {
        eventId: "root-1",
        channelId: "c1",
        threadRootId: "root-1",
        authorLabel: "Vlad",
      },
    ],
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

test("heuristic skips short acknowledgements that are not mentions or DMs", () => {
  const actions = heuristicActions(
    [
      {
        ...MENTION,
        eventId: "ack",
        content: "ok",
        isMention: false,
      },
    ],
    [],
    EMPTY_LESSONS,
  );
  assert.deepEqual(actions, [{ type: "skip", eventId: "ack" }]);
});

test("heuristic never skips a mention, even a short ack", () => {
  const actions = heuristicActions(
    [{ ...MENTION, eventId: "ack", content: "ok" }],
    [],
    EMPTY_LESSONS,
  );
  assert.equal(actions.length, 1);
  assert.equal(actions[0].type, "create");
  assert.deepEqual(actions[0].eventIds, ["ack"]);
});

test("heuristic never skips a DM", () => {
  const actions = heuristicActions(
    [
      {
        eventId: "dm1",
        channelId: "dm-1",
        channelName: "Fizz",
        threadRootId: null,
        authorPubkey: "fizz",
        authorLabel: "Fizz",
        content: "got it",
        isDm: true,
        isMention: false,
        isSelf: false,
      },
    ],
    [],
    EMPTY_LESSONS,
  );
  assert.equal(actions[0].type, "create");
  assert.equal(actions[0].kind, "fyi");
});

test("heuristic creates an ask for a qualifying mention", () => {
  const actions = heuristicActions([MENTION], [], EMPTY_LESSONS);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].type, "create");
  assert.equal(actions[0].kind, "ask");
  assert.deepEqual(actions[0].eventIds, ["m1"]);
  assert.ok(actions[0].score >= 50);
  assert.match(
    actions[0].summary,
    /^Vlad asked in #hack-project-mesh: @jacob can you run the triage scripts/,
  );
});

test("heuristic updates an open fibre for a same-channel mention in that thread", () => {
  const actions = heuristicActions(
    [
      {
        ...MENTION,
        eventId: "m2",
        threadRootId: "root-1",
        content: "the scripts are in the repo",
      },
    ],
    OPEN,
    EMPTY_LESSONS,
  );
  assert.deepEqual(actions, [
    { type: "update", fibreId: "f-open", eventIds: ["m2"] },
  ]);
});

test("heuristic does not attach same-thread chatter to an open fibre", () => {
  const actions = heuristicActions(
    [
      {
        ...MENTION,
        eventId: "hey",
        threadRootId: "root-1",
        content: "hey",
        isMention: false,
      },
    ],
    OPEN,
    EMPTY_LESSONS,
  );
  assert.deepEqual(actions, [{ type: "skip", eventId: "hey" }]);
});

test("heuristic creates two fibres for two asks in the same thread", () => {
  const actions = heuristicActions(
    [
      {
        eventId: "a1",
        channelId: "c1",
        channelName: "hack-project-mesh",
        threadRootId: "root-1",
        authorPubkey: "vlad",
        authorLabel: "Vlad",
        content: "can you run the triage scripts before the next build please",
        isDm: false,
        isMention: false,
        isSelf: false,
      },
      {
        eventId: "a2",
        channelId: "c1",
        channelName: "hack-project-mesh",
        threadRootId: "root-1",
        authorPubkey: "zhenya",
        authorLabel: "zhenya",
        content: "what is the root cause of last night's deploy rollback",
        isDm: false,
        isMention: false,
        isSelf: false,
      },
    ],
    [],
    EMPTY_LESSONS,
  );
  assert.equal(actions.filter((action) => action.type === "create").length, 2);
  assert.deepEqual(
    actions.map((action) => action.eventIds[0]),
    ["a1", "a2"],
  );
});

test("heuristic skips a message already attached to an open fibre", () => {
  const actions = heuristicActions(
    [{ ...MENTION, eventId: "root-1", threadRootId: "root-1" }],
    OPEN,
    EMPTY_LESSONS,
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

test("parseLlmActions keeps named summaries and drops a fourth sentence", () => {
  const [create] = parseLlmActions(
    {
      actions: [
        {
          type: "create",
          kind: "ask",
          title: "Scripts",
          summary:
            "Vlad asked jacob to run the scripts. He wants them done before the next build. The two scripts must run in order. Ignore this extra sentence.",
          eventIds: ["m1"],
        },
      ],
    },
    OPEN,
  );
  assert.equal(
    create.summary,
    "Vlad asked jacob to run the scripts. He wants them done before the next build. The two scripts must run in order.",
  );
});

test("constrainActions injects a create when the model skips a mention", () => {
  const actions = constrainActions(
    [{ type: "skip", eventId: "m1" }],
    [MENTION],
    [],
    EMPTY_LESSONS,
  );
  assert.equal(actions.length, 1);
  assert.equal(actions[0].type, "create");
  assert.deepEqual(actions[0].eventIds, ["m1"]);
});

test("constrainActions drops a cross-channel update", () => {
  const actions = constrainActions(
    [{ type: "update", fibreId: "f-open", eventIds: ["other"] }],
    [
      {
        ...MENTION,
        eventId: "other",
        channelId: "c2",
        channelName: "general",
      },
    ],
    OPEN,
    EMPTY_LESSONS,
  );
  assert.equal(
    actions.some((action) => action.type === "update"),
    false,
  );
  assert.equal(actions[0].type, "create");
  assert.deepEqual(actions[0].eventIds, ["other"]);
});

test("constrainActions drops a cross-channel merge", () => {
  const other = {
    ...OPEN[0],
    id: "f-other",
    channelId: "c2",
    channelName: "general",
    artifacts: [
      {
        eventId: "g-root",
        channelId: "c2",
        threadRootId: "g-root",
      },
    ],
  };
  const actions = constrainActions(
    [
      {
        type: "merge",
        fibreIds: ["f-open", "f-other"],
        into: "f-open",
      },
    ],
    [],
    [...OPEN, other],
    EMPTY_LESSONS,
  );
  assert.deepEqual(actions, []);
});

test("constrainActions splits a mixed-channel create", () => {
  const actions = constrainActions(
    [{ type: "create", kind: "ask", eventIds: ["m1", "g1"] }],
    [
      MENTION,
      {
        ...MENTION,
        eventId: "g1",
        channelId: "c2",
        channelName: "general",
        isMention: false,
        content: "can you review the rollback tonight",
      },
    ],
    [],
    EMPTY_LESSONS,
  );
  const creates = actions.filter((action) => action.type === "create");
  assert.equal(creates.length, 2);
  assert.deepEqual(creates[0].eventIds, ["m1"]);
  assert.deepEqual(creates[1].eventIds, ["g1"]);
});

test("buildPrompt includes open fibres, channel ids, and skip-default rules", () => {
  const prompt = buildPrompt([MENTION], OPEN, { examples: [] });
  assert.match(prompt, /f-open/);
  assert.match(prompt, /Run triage scripts/);
  assert.match(prompt, /m1/);
  assert.match(prompt, /"channelId":"c1"/);
  assert.match(prompt, /Default action is skip/);
  assert.match(prompt, /isMention: true/);
  assert.match(prompt, /never across channels/);
  assert.match(prompt, /Vlad asked jacob/);
  assert.match(prompt, /never a fourth/);
});

test("limitSummary keeps at most three sentences", () => {
  assert.equal(
    limitSummary("One. Two. Three. Four. Five."),
    "One. Two. Three.",
  );
});

test("narrativeSummary names the person and the ask", () => {
  assert.equal(
    narrativeSummary("ask", MENTION, MENTION.content),
    "Vlad asked in #hack-project-mesh: @jacob can you run the triage scripts before the next build",
  );
});

test("summarizeFibre is compact enough for the prompt", () => {
  const summary = summarizeFibre(OPEN[0]);
  assert.deepEqual(summary.eventIds, ["root-1"]);
  assert.deepEqual(summary.people, ["Vlad"]);
  assert.equal(summary.channelId, "c1");
});
