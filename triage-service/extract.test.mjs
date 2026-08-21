import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const dir = mkdtempSync(path.join(tmpdir(), "triage-extract-"));
const file = path.join(dir, "data.json");
writeFileSync(file, `${JSON.stringify({})}\n`);
process.env.TRIAGE_DATA_FILE = file;
process.env.TRIAGE_LLM = "0";

const { resetStore } = await import("./store.mjs");
const {
  BURST_WINDOW_SECONDS,
  buildExtractPrompt,
  constrainDecisions,
  decisionsToActions,
  extractDecisions,
  heuristicDecision,
  parseExtractDecisions,
  startsAsContinuation,
  summarizeFibre,
  summarizeOpenFibres,
} = await import("./extract.mjs");

test.after(() => {
  resetStore();
  rmSync(dir, { recursive: true, force: true });
});

const OPEN = [
  {
    id: "f-open",
    kind: "ask",
    status: "open",
    title: "Run triage scripts",
    summary: "Vlad asked for the scripts",
    score: 80,
    channelId: "c1",
    channelName: "hack-project-mesh",
    isDm: false,
    updatedAt: 100,
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

function constrain(decisions, messages, openFibres = []) {
  return constrainDecisions({ decisions, messages, openFibres });
}

test("every message gets a decision even when the model answers for none", () => {
  const decisions = constrain(
    [],
    [MENTION, { ...MENTION, eventId: "m2", content: "ok" }],
    [],
  );

  assert.equal(decisions.length, 2);
  assert.deepEqual(
    decisions.map((decision) => decision.eventId),
    ["m1", "m2"],
  );
});

test("a message the viewer wrote is always ignored", () => {
  const mine = {
    ...MENTION,
    eventId: "mine",
    isSelf: true,
    isMention: false,
    authorPubkey: "me",
    content: "I will get to this after standup",
  };

  assert.equal(heuristicDecision(mine, []).type, "ignore");
  assert.equal(
    constrain(
      [{ eventId: "mine", type: "create", kind: "commitment", title: "x" }],
      [mine],
    )[0].type,
    "ignore",
  );
});

test("a mention the model ignores becomes a fibre rather than vanishing", () => {
  const [decision] = constrain(
    [{ eventId: "m1", type: "ignore", reason: "seems minor" }],
    [MENTION],
    [],
  );

  assert.equal(decision.type, "create");
  assert.equal(decision.kind, "ask");
});

test("a low-value mention stays ignored", () => {
  const [decision] = constrain(
    [{ eventId: "ack", type: "ignore", reason: "acknowledgement" }],
    [{ ...MENTION, eventId: "ack", content: "thanks!" }],
    [],
  );

  assert.equal(decision.type, "ignore");
});

test("a bump joins the fibre it is chasing instead of creating one", () => {
  const bump = {
    ...MENTION,
    eventId: "bump",
    threadRootId: "root-1",
    content: "bump — any update on this?",
  };

  const [fromModel] = constrain(
    [{ eventId: "bump", type: "ignore", reason: "follow-up" }],
    [bump],
    OPEN,
  );
  assert.deepEqual(fromModel, {
    eventId: "bump",
    type: "attach",
    fibreId: "f-open",
  });

  assert.equal(heuristicDecision(bump, OPEN).type, "attach");
});

// The real case that split: jacob posted an ask and its consequence nine
// seconds apart, both top-level, and each became its own fibre.
const BURST = [
  {
    ...MENTION,
    eventId: "ask",
    createdAt: 1_000,
    content:
      "@Vlad I think you'll have to add the bang-mentions because it has implications for the triage engine",
  },
  {
    ...MENTION,
    eventId: "consequence",
    createdAt: 1_009,
    isMention: false,
    content: "so i'd imagine there'd be a ton of conflicts",
  },
];

test("one thought split across two messages becomes one fibre", () => {
  const decisions = constrain([], BURST, []);

  assert.equal(decisions[1].groupKey, "ask");
  const actions = decisionsToActions(decisions, BURST);
  assert.equal(actions.length, 1);
  assert.deepEqual(actions[0].eventIds, ["ask", "consequence"]);
});

test("a burst continuation joins the fibre the first message attached to", () => {
  const messages = [
    { ...BURST[0], eventId: "reply", threadRootId: "root-1" },
    { ...BURST[1], eventId: "consequence", threadRootId: "root-1" },
  ];
  const decisions = constrain([], messages, OPEN);

  assert.deepEqual(
    decisions.map((decision) => [decision.type, decision.fibreId]),
    [
      ["attach", "f-open"],
      ["attach", "f-open"],
    ],
  );
});

test("the model can veto burst grouping with newTopic", () => {
  const decisions = constrain(
    [
      { eventId: "ask", type: "create", kind: "ask", title: "Bang mentions" },
      {
        eventId: "consequence",
        type: "create",
        kind: "question",
        title: "Standup",
        newTopic: true,
      },
    ],
    BURST,
    [],
  );

  assert.equal(decisions[1].groupKey, undefined);
  assert.equal(decisionsToActions(decisions, BURST).length, 2);
});

test("a new subject from the same author starts its own fibre", () => {
  const messages = [
    BURST[0],
    {
      ...BURST[1],
      eventId: "unrelated",
      content: "Has anyone looked at the flaky media upload test?",
    },
  ];
  const decisions = constrain([], messages, []);

  assert.equal(decisions[1].groupKey, undefined);
  assert.equal(decisionsToActions(decisions, messages).length, 2);
});

test("the burst window has a hard edge", () => {
  const atEdge = [
    BURST[0],
    { ...BURST[1], createdAt: 1_000 + BURST_WINDOW_SECONDS },
  ];
  const pastEdge = [
    BURST[0],
    { ...BURST[1], createdAt: 1_000 + BURST_WINDOW_SECONDS + 1 },
  ];

  assert.equal(decisionsToActions(constrain([], atEdge, []), atEdge).length, 1);
  assert.equal(
    decisionsToActions(constrain([], pastEdge, []), pastEdge).length,
    1,
  );
  // Past the window the tail carries nothing on its own, so it is dropped
  // rather than promoted into a fibre of its own.
  assert.equal(constrain([], pastEdge, [])[1].type, "ignore");
});

test("a burst from a different author is not the same thought", () => {
  const messages = [
    BURST[0],
    { ...BURST[1], authorPubkey: "someone-else", authorLabel: "Zhenya" },
  ];

  assert.equal(constrain([], messages, [])[1].groupKey, undefined);
});

test("startsAsContinuation reads back-references, not new subjects", () => {
  assert.equal(startsAsContinuation("so i'd imagine there'd be conflicts"), true);
  assert.equal(startsAsContinuation("and the migration too"), true);
  assert.equal(startsAsContinuation("it also breaks the relay"), true);
  assert.equal(startsAsContinuation("That said, we could ship it"), true);
  assert.equal(startsAsContinuation("Has anyone seen the flaky test?"), false);
  assert.equal(startsAsContinuation("Deploying the relay now"), false);
});

test("a follow-up joins a fibre the same batch is about to create", () => {
  const messages = [
    { ...MENTION, eventId: "ask", threadRootId: null },
    {
      ...MENTION,
      eventId: "bump",
      threadRootId: "ask",
      content: "bump — any update on this?",
    },
  ];
  const decisions = constrain([], messages, []);

  assert.deepEqual(
    decisions.map((decision) => decision.type),
    ["create", "create"],
  );
  assert.equal(decisions[0].groupKey ?? decisions[0].eventId, "ask");
  assert.equal(decisions[1].groupKey, "ask");

  const actions = decisionsToActions(decisions, messages);
  assert.equal(actions.length, 1);
  assert.deepEqual(actions[0].eventIds, ["ask", "bump"]);
});

test("a busy thread becomes one fibre even when nothing is asked", () => {
  const messages = Array.from({ length: 4 }, (_, index) => ({
    ...MENTION,
    eventId: `d${index}`,
    isMention: false,
    threadRootId: "design-root",
    authorPubkey: `person-${index}`,
    content: "thinking out loud about how the nav should behave on mobile",
  }));
  const contexts = new Map(
    messages.map((message) => [message.eventId, { scope: "channel", messages: [], engagement: 72 }]),
  );

  const decisions = constrainDecisions({
    decisions: [],
    messages,
    openFibres: [],
    contexts,
  });

  assert.ok(decisions.every((decision) => decision.type === "create"));
  const actions = decisionsToActions(decisions, messages);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].eventIds.length, 4);
});

test("a quiet thread with nothing asked is still ignored", () => {
  const message = {
    ...MENTION,
    eventId: "quiet",
    isMention: false,
    threadRootId: "t",
    content: "thinking out loud about how the nav should behave on mobile",
  };

  const [decision] = constrainDecisions({
    decisions: [],
    messages: [message],
    openFibres: [],
    contexts: new Map([["quiet", { scope: "channel", messages: [], engagement: 12 }]]),
  });

  assert.equal(decision.type, "ignore");
});

test("a follow-up with nothing to chase stays ignored", () => {
  const [decision] = constrain(
    [{ eventId: "bump", type: "ignore", reason: "follow-up" }],
    [{ ...MENTION, eventId: "bump", content: "bump" }],
    [],
  );

  assert.equal(decision.type, "ignore");
});

test("a cross-channel attach falls back to creating a fibre", () => {
  const [decision] = constrain(
    [{ eventId: "other", type: "attach", fibreId: "f-open", title: "Elsewhere" }],
    [{ ...MENTION, eventId: "other", channelId: "c2", channelName: "general" }],
    OPEN,
  );

  assert.equal(decision.type, "create");
  assert.equal(decision.title, "Elsewhere");
});

test("a cross-channel merge degrades to an attach on the viewer's channel", () => {
  const other = {
    ...OPEN[0],
    id: "f-other",
    channelId: "c2",
    channelName: "general",
    artifacts: [{ eventId: "g-root", channelId: "c2", threadRootId: "g-root" }],
  };

  const [decision] = constrain(
    [
      {
        eventId: "m1",
        type: "merge",
        fibreIds: ["f-open", "f-other"],
        into: "f-open",
      },
    ],
    [MENTION],
    [...OPEN, other],
  );

  assert.deepEqual(decision, {
    eventId: "m1",
    type: "attach",
    fibreId: "f-open",
  });
});

test("a message already held by an open fibre is ignored", () => {
  const [decision] = constrain(
    [{ eventId: "root-1", type: "create", kind: "ask", title: "again" }],
    [{ ...MENTION, eventId: "root-1", threadRootId: "root-1" }],
    OPEN,
  );

  assert.equal(decision.type, "ignore");
});

test("the heuristic ignores an acknowledgement even when it is aimed at you", () => {
  assert.equal(
    heuristicDecision({ ...MENTION, eventId: "ack", content: "thanks!" }, [])
      .type,
    "ignore",
  );
});

test("the heuristic ignores chatter that is not addressed to the viewer", () => {
  assert.equal(
    heuristicDecision(
      { ...MENTION, eventId: "ack", content: "ok", isMention: false },
      [],
    ).type,
    "ignore",
  );
  assert.equal(
    heuristicDecision(
      {
        ...MENTION,
        eventId: "chat",
        isMention: false,
        content: "the deploy finished and everything looks fine",
      },
      [],
    ).type,
    "ignore",
  );
});

test("the heuristic keeps a DM and an urgent request", () => {
  assert.equal(
    heuristicDecision(
      {
        ...MENTION,
        eventId: "dm1",
        channelId: "dm-1",
        isDm: true,
        isMention: false,
        content: "the staging relay is down again, are you around",
      },
      [],
    ).type,
    "create",
  );

  const urgent = heuristicDecision(
    {
      ...MENTION,
      eventId: "u1",
      isMention: false,
      content: "can you review the migration tonight",
    },
    [],
  );
  assert.equal(urgent.type, "create");
  assert.equal(urgent.kind, "ask");
});

test("incident language wins over the shape of the sentence", () => {
  const decision = heuristicDecision(
    {
      ...MENTION,
      eventId: "b1",
      isMention: false,
      content: "can you look at the root cause of last night's rollback",
    },
    [],
  );
  assert.equal(decision.kind, "blocker");
});

test("creates sharing a groupKey collapse into one fibre", () => {
  const messages = [
    { ...MENTION, eventId: "a" },
    { ...MENTION, eventId: "b" },
    { ...MENTION, eventId: "c", channelId: "c2" },
  ];
  const actions = decisionsToActions(
    [
      { eventId: "a", type: "create", kind: "ask", groupKey: "deploy" },
      { eventId: "b", type: "create", kind: "ask", groupKey: "deploy" },
      { eventId: "c", type: "create", kind: "ask", groupKey: "deploy" },
    ],
    messages,
  );

  assert.equal(actions.length, 2);
  assert.deepEqual(actions[0].eventIds, ["a", "b"]);
  assert.deepEqual(actions[1].eventIds, ["c"]);
});

test("decisions map onto the action shape apply understands", () => {
  const actions = decisionsToActions(
    [
      { eventId: "a", type: "ignore", reason: "noise" },
      { eventId: "b", type: "attach", fibreId: "f-open" },
      { eventId: "c", type: "merge", fibreIds: ["x", "y"], into: "x" },
    ],
    [
      { ...MENTION, eventId: "a" },
      { ...MENTION, eventId: "b" },
      { ...MENTION, eventId: "c" },
    ],
  );

  assert.deepEqual(
    actions.map((action) => action.type),
    ["update", "merge"],
  );
  assert.deepEqual(actions[0].eventIds, ["b"]);
});

test("parseExtractDecisions coerces unknown kinds and drops malformed rows", () => {
  const decisions = parseExtractDecisions({
    decisions: [
      { eventId: "a", type: "create", kind: "not-a-kind", title: " Hi " },
      { eventId: "b", type: "attach" },
      { type: "ignore", reason: "no id" },
      { eventId: "c", type: "ignore", reason: " chatter " },
    ],
  });

  assert.equal(decisions.length, 2);
  assert.equal(decisions[0].kind, "fyi");
  assert.equal(decisions[0].title, "Hi");
  assert.equal(decisions[1].reason, "chatter");
});

test("summarizeFibre carries thread roots rather than every event id", () => {
  const summary = summarizeFibre(OPEN[0]);
  assert.deepEqual(summary.threadRootIds, ["root-1"]);
  assert.deepEqual(summary.people, ["Vlad"]);
  assert.equal(summary.artifactCount, 1);
  assert.equal(summary.channelId, "c1");
});

test("a large open set keeps full detail only for the batch's channels", () => {
  const many = Array.from({ length: 200 }, (_, index) => ({
    ...OPEN[0],
    id: `f${index}`,
    channelId: index === 0 ? "c1" : "c-other",
    artifacts: [{ eventId: `e${index}`, channelId: index === 0 ? "c1" : "c-other" }],
  }));

  const summarized = summarizeOpenFibres(many, ["c1"]);
  assert.ok("summary" in summarized[0]);
  assert.equal("summary" in summarized[1], false);
});

test("the stage 1 prompt carries open fibres, context, and the exhaustive rule", () => {
  const prompt = buildExtractPrompt({
    messages: [MENTION],
    contexts: new Map([
      [
        "m1",
        {
          scope: "channel",
          messages: [{ authorLabel: "Zhenya", isSelf: false, content: "earlier" }],
        },
      ],
    ]),
    openFibres: OPEN,
    lessons: {
      examples: [
        { preview: "lunch thread", userAction: "dismissed", note: "always noise" },
      ],
    },
  });

  assert.match(prompt, /f-open/);
  assert.match(prompt, /Run triage scripts/);
  assert.match(prompt, /"channelId":"c1"/);
  assert.match(prompt, /Omitting one is a failure/);
  assert.match(prompt, /never a new fibre/);
  assert.match(prompt, /always noise/);
  assert.match(prompt, /earlier/);
  assert.match(prompt, /You do not rank anything/);
});

test("extractDecisions returns one decision per message without an LLM", async () => {
  const decisions = await extractDecisions({
    pubkey: "pk",
    messages: [MENTION, { ...MENTION, eventId: "m2", content: "hey" }],
    openFibres: [],
  });

  assert.deepEqual(
    decisions.map((decision) => decision.eventId),
    ["m1", "m2"],
  );
  assert.equal(decisions[0].type, "create");
});
