import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveActivityPillLabel,
  deriveActivityPillPresentation,
  deriveAgentWorkingOrder,
  deriveLastLiveAt,
  partitionComposerWorkingAgents,
} from "./composerLiveActivity.ts";

const CHANNEL = "channel-1";
const OTHER_CHANNEL = "channel-2";

const NOW = Date.parse("2026-07-23T00:01:00.000Z");
const NOW_ISO = new Date(NOW).toISOString();

/** Thought item: spine, headlined by its title. */
const thought = (
  title,
  timestamp,
  channelId = CHANNEL,
  turnId = undefined,
) => ({
  id: `thought-${title}-${timestamp}`,
  type: "thought",
  renderClass: "thought",
  title,
  text: "",
  timestamp,
  channelId,
  turnId,
});

/** Metadata item: meaningful but NOT spine — recedes when real work exists. */
const metadata = (title, timestamp, channelId = CHANNEL) => ({
  id: `metadata-${title}-${timestamp}`,
  type: "metadata",
  renderClass: "raw-rail",
  title,
  sections: [],
  timestamp,
  acpSource: "prompt_context",
  channelId,
});

/** Lifecycle meta-frame (usage tick / commands): spine, but never a headline. */
const lifecycleMeta = (acpSource, timestamp, channelId = CHANNEL) => ({
  id: `lifecycle-${acpSource}-${timestamp}`,
  type: "lifecycle",
  renderClass: "status",
  title: acpSource === "usage_update" ? "Usage" : "Commands",
  text: "Tokens: 1200/8192",
  timestamp,
  acpSource,
  channelId,
});

/** Tool item with a classifier descriptor: headlines tersely (verb + object). */
const fileReadTool = (path, timestamp, channelId = CHANNEL) => ({
  id: `tool-${path}-${timestamp}`,
  type: "tool",
  renderClass: "file-read",
  title: "Read file",
  toolName: "dev__read_file",
  buzzToolName: null,
  status: "completed",
  args: { path },
  result: "",
  isError: false,
  timestamp,
  startedAt: timestamp,
  completedAt: timestamp,
  descriptor: {
    renderClass: "file-read",
    label: "Read file",
    preview: path,
    action: { verb: "Read", object: path },
    source: "harness",
    groupKey: "read_file",
  },
  channelId,
});

/** Streaming assistant message: headline is the (growing) first line. */
const assistantMessage = (id, text, timestamp, channelId = CHANNEL) => ({
  id,
  type: "message",
  role: "assistant",
  title: "",
  text,
  timestamp,
  channelId,
});

const secondsBeforeNow = (seconds) =>
  new Date(NOW - seconds * 1000).toISOString();

test("deriveActivityPillLabel returns the newest headline, no rotation", () => {
  const editing = thought("Editing ChannelPane", secondsBeforeNow(2));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [thought("Reading files", secondsBeforeNow(4)), editing],
  });
  assert.deepEqual(headline, { id: editing.id, label: "Editing ChannelPane" });
});

test("deriveActivityPillLabel headlines tool items tersely (verb + basename)", () => {
  const read = fileReadTool("src/agents/ui/foo.ts", secondsBeforeNow(2));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [read],
  });
  // Terse action tier, not "Read file · src/agents/ui/foo.ts" — the pill's
  // narrow cap must show the informative part of the action.
  assert.deepEqual(headline, { id: read.id, label: "Read foo.ts" });
});

test("deriveActivityPillLabel keeps the last action headline regardless of age", () => {
  // A quiet stretch (long tool call, thinking gap) must NOT decay the label
  // to the generic placeholder — the last real action stays informative.
  const editing = thought("Editing ChannelPane", secondsBeforeNow(300));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [editing],
  });
  assert.deepEqual(headline, { id: editing.id, label: "Editing ChannelPane" });
});

test("deriveActivityPillLabel ignores other-channel items", () => {
  const inChannel = thought("In-channel work", secondsBeforeNow(3));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [
      inChannel,
      thought("Other-channel work", secondsBeforeNow(1), OTHER_CHANNEL),
    ],
  });
  assert.deepEqual(headline, { id: inChannel.id, label: "In-channel work" });
});

test("deriveActivityPillLabel lets spine work headline over fresher metadata reads", () => {
  const realWork = thought("Real work", secondsBeforeNow(4));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [realWork, metadata("Prompt context", secondsBeforeNow(1))],
  });
  assert.deepEqual(headline, { id: realWork.id, label: "Real work" });
});

test("deriveActivityPillLabel falls back to metadata when no spine items exist", () => {
  const context = metadata("Prompt context", secondsBeforeNow(5));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [context],
  });
  assert.deepEqual(headline, { id: context.id, label: "Prompt context" });
});

test("deriveActivityPillLabel returns null for an empty transcript", () => {
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(headline, null);
});

test("deriveActivityPillLabel never headlines usage/commands meta frames", () => {
  const realWork = thought("Real work", secondsBeforeNow(4));
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [
      realWork,
      lifecycleMeta("usage_update", secondsBeforeNow(2)),
      lifecycleMeta("available_commands_update", secondsBeforeNow(1)),
    ],
  });
  // Meta frames are skipped entirely — the older real action still headlines.
  assert.deepEqual(headline, { id: realWork.id, label: "Real work" });
});

test("deriveActivityPillLabel returns null when only meta frames exist", () => {
  const headline = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [lifecycleMeta("usage_update", secondsBeforeNow(1))],
  });
  assert.equal(headline, null);
});

test("deriveActivityPillLabel keeps a stable id while a message streams", () => {
  const first = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [assistantMessage("msg-1", "Pass 1: reading", NOW_ISO)],
  });
  const extended = deriveActivityPillLabel({
    channelId: CHANNEL,
    transcript: [
      assistantMessage("msg-1", "Pass 1: reading the composer wiring", NOW_ISO),
    ],
  });
  // Same item id: the pill updates text in place instead of re-animating.
  assert.equal(first.id, "msg-1");
  assert.equal(extended.id, "msg-1");
  assert.equal(extended.label, "Pass 1: reading the composer wiring");
});

test("deriveActivityPillLabel scopes observer headlines to every live turn", () => {
  const turnTwo = thought(
    "Current turn work",
    secondsBeforeNow(1),
    CHANNEL,
    "turn-2",
  );
  const headline = deriveActivityPillLabel({
    activeTurnIds: new Set(["turn-2", "turn-3"]),
    channelId: CHANNEL,
    transcript: [
      thought("Previous turn work", secondsBeforeNow(2), CHANNEL, "turn-1"),
      turnTwo,
    ],
  });
  assert.deepEqual(headline, { id: turnTwo.id, label: "Current turn work" });

  assert.equal(
    deriveActivityPillLabel({
      activeTurnIds: new Set(["turn-3"]),
      channelId: CHANNEL,
      transcript: [
        thought("Previous turn work", secondsBeforeNow(2), CHANNEL, "turn-1"),
      ],
    }),
    null,
  );
});

test("deriveActivityPillPresentation keeps observer activity authoritative over typing", () => {
  const headline = { id: "action-1", label: "Inspect composer state" };
  const presentation = deriveActivityPillPresentation({
    agentName: "Alice",
    headline,
    isTyping: true,
    workingSource: "observer",
  });

  assert.deepEqual(presentation, headline);
});

test("deriveActivityPillPresentation uses working copy before the first observer action", () => {
  const presentation = deriveActivityPillPresentation({
    agentName: "Alice",
    headline: null,
    isTyping: true,
    workingSource: "observer",
  });

  assert.deepEqual(presentation, {
    id: "generic-working",
    label: "Alice is working…",
  });
});

test("deriveActivityPillPresentation uses typing before observer startup and after completion", () => {
  const headline = { id: "action-1", label: "Inspect composer state" };

  assert.deepEqual(
    deriveActivityPillPresentation({
      agentName: "Alice",
      headline: null,
      isTyping: true,
      workingSource: "none",
    }),
    { id: "typing-override", label: "Alice is typing…" },
  );
  assert.deepEqual(
    deriveActivityPillPresentation({
      agentName: "Alice",
      headline,
      isTyping: true,
      workingSource: "typing",
    }),
    { id: "typing-override", label: "Alice is typing…" },
  );
});

/** Lifecycle noise item ("Turn started") — meaningful:false, never headlines. */
const turnStartedLifecycle = (timestamp, channelId = CHANNEL) => ({
  id: `lifecycle-turn-${timestamp}`,
  type: "lifecycle",
  renderClass: "status",
  title: "Turn started",
  text: "",
  timestamp,
  channelId,
});

/** Fake partition readers over pubkey → source / transcript maps. */
const partitionReaders = (sources, transcripts) => ({
  getTranscript: (pubkey) => transcripts.get(pubkey) ?? [],
  getWorkingSource: (pubkey) => sources.get(pubkey) ?? "none",
});

test("partitionComposerWorkingAgents pills observer-backed agents regardless of transcript", () => {
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(new Map([["alpha", "observer"]]), new Map()),
    pubkeys: ["alpha"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: ["alpha"],
    typingGroupPubkeys: [],
  });
});

test("partitionComposerWorkingAgents groups typing agents with nothing to show", () => {
  // First-ever activity in the channel: no transcript at all.
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(new Map([["alpha", "typing"]]), new Map()),
    pubkeys: ["alpha"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: [],
    typingGroupPubkeys: ["alpha"],
  });
});

test("partitionComposerWorkingAgents keeps a typing agent's pill across the turn-end gap", () => {
  // Turn completed but the agent is still typing: the prior turn's real
  // action keeps the agent pill-worthy — the pill relabels instead of
  // demoting to the typing group.
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(
      new Map([["alpha", "typing"]]),
      new Map([
        ["alpha", [thought("Editing ChannelPane", secondsBeforeNow(9))]],
      ]),
    ),
    pubkeys: ["alpha"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: ["alpha"],
    typingGroupPubkeys: [],
  });
});

test("partitionComposerWorkingAgents ignores lifecycle-noise-only transcripts", () => {
  // A transcript holding only "Turn started" noise (seeded turns, no content
  // frames) is NOT pill-worthy — nothing would render in the hover feed.
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(
      new Map([["alpha", "typing"]]),
      new Map([["alpha", [turnStartedLifecycle(secondsBeforeNow(5))]]]),
    ),
    pubkeys: ["alpha"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: [],
    typingGroupPubkeys: ["alpha"],
  });
});

test("partitionComposerWorkingAgents scopes pill-worthiness to the channel", () => {
  // Real work in ANOTHER channel must not promote this channel's typing.
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(
      new Map([["alpha", "typing"]]),
      new Map([
        ["alpha", [thought("Other work", secondsBeforeNow(3), OTHER_CHANNEL)]],
      ]),
    ),
    pubkeys: ["alpha"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: [],
    typingGroupPubkeys: ["alpha"],
  });
});

test("partitionComposerWorkingAgents splits a mixed roster preserving order", () => {
  const partition = partitionComposerWorkingAgents({
    channelId: CHANNEL,
    ...partitionReaders(
      new Map([
        ["alpha", "observer"],
        ["beta", "typing"],
        ["gamma", "typing"],
      ]),
      new Map([["gamma", [thought("Prior work", secondsBeforeNow(30))]]]),
    ),
    pubkeys: ["alpha", "beta", "gamma"],
  });
  assert.deepEqual(partition, {
    pillPubkeys: ["alpha", "gamma"],
    typingGroupPubkeys: ["beta"],
  });
});

/** Fake working-state reader over pubkey → [{channelId, anchorAt}] entries. */
const workingStates = (entries) => (pubkey) => ({
  channels: entries.get(pubkey) ?? [],
});

test("deriveAgentWorkingOrder puts the earliest-started agent first", () => {
  const states = new Map([
    ["alpha", [{ channelId: CHANNEL, anchorAt: NOW - 20_000 }]],
    ["beta", [{ channelId: CHANNEL, anchorAt: NOW - 90_000 }]],
  ]);
  const order = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(states),
    pubkeys: ["alpha", "beta"],
  });
  assert.deepEqual(order, ["beta", "alpha"]);
});

test("deriveAgentWorkingOrder appends a later starter after existing workers", () => {
  const states = new Map([
    ["alpha", [{ channelId: CHANNEL, anchorAt: NOW - 60_000 }]],
    ["beta", [{ channelId: CHANNEL, anchorAt: NOW - 30_000 }]],
    ["gamma", [{ channelId: CHANNEL, anchorAt: NOW - 2_000 }]],
  ]);
  const order = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(states),
    // Roster order deliberately differs from start order.
    pubkeys: ["gamma", "beta", "alpha"],
  });
  assert.deepEqual(order, ["alpha", "beta", "gamma"]);
});

test("deriveAgentWorkingOrder ignores other-channel anchors", () => {
  const states = new Map([
    ["alpha", [{ channelId: CHANNEL, anchorAt: NOW - 5_000 }]],
    // Beta started much earlier — but in another channel, so it has no
    // anchor for this scope and sorts to the end.
    ["beta", [{ channelId: OTHER_CHANNEL, anchorAt: NOW - 300_000 }]],
  ]);
  const order = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(states),
    pubkeys: ["beta", "alpha"],
  });
  assert.deepEqual(order, ["alpha", "beta"]);
});

test("deriveAgentWorkingOrder uses the earliest anchor across channels when unscoped", () => {
  const states = new Map([
    [
      "alpha",
      [
        { channelId: CHANNEL, anchorAt: NOW - 10_000 },
        { channelId: OTHER_CHANNEL, anchorAt: NOW - 120_000 },
      ],
    ],
    ["beta", [{ channelId: CHANNEL, anchorAt: NOW - 60_000 }]],
  ]);
  const order = deriveAgentWorkingOrder({
    channelId: null,
    getWorkingState: workingStates(states),
    pubkeys: ["beta", "alpha"],
  });
  assert.deepEqual(order, ["alpha", "beta"]);
});

test("deriveAgentWorkingOrder keeps roster order for agents with no anchor", () => {
  const states = new Map([
    ["gamma", [{ channelId: CHANNEL, anchorAt: NOW - 5_000 }]],
  ]);
  const order = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(states),
    pubkeys: ["alpha", "beta", "gamma"],
  });
  assert.deepEqual(order, ["gamma", "alpha", "beta"]);
});

test("deriveAgentWorkingOrder quantizes anchors to seconds so sub-second shifts never reorder", () => {
  // Same wall-clock second; beta's anchor is a few hundred ms earlier (the
  // shape of a retroactive clock-offset refinement). Order must follow the
  // roster, and must not flip when the sub-second part changes.
  const base = Math.floor((NOW - 10_000) / 1000) * 1000;
  const order = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(
      new Map([
        ["alpha", [{ channelId: CHANNEL, anchorAt: base + 700 }]],
        ["beta", [{ channelId: CHANNEL, anchorAt: base + 100 }]],
      ]),
    ),
    pubkeys: ["alpha", "beta"],
  });
  assert.deepEqual(order, ["alpha", "beta"]);

  const afterRefinement = deriveAgentWorkingOrder({
    channelId: CHANNEL,
    getWorkingState: workingStates(
      new Map([
        // Alpha's offset estimate tightened: anchor slid 400ms earlier.
        ["alpha", [{ channelId: CHANNEL, anchorAt: base + 300 }]],
        ["beta", [{ channelId: CHANNEL, anchorAt: base + 100 }]],
      ]),
    ),
    pubkeys: ["alpha", "beta"],
  });
  assert.deepEqual(afterRefinement, order);
});

test("deriveLastLiveAt prefers the newest channel-scoped transcript item", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [
      { channelId: CHANNEL, timestamp: "2026-07-23T00:00:01.000Z" },
      { channelId: OTHER_CHANNEL, timestamp: "2026-07-23T00:00:09.000Z" },
      { channelId: CHANNEL, timestamp: "2026-07-23T00:00:05.000Z" },
    ],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-23T00:00:05.000Z"));
});

test("deriveLastLiveAt sees archived content the panel renders", () => {
  // Regression: archived rows are visible in the preview even when the live
  // transcript window is empty — the pill must not say "No activity yet".
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [],
    archivedEvents: [{ timestamp: "2026-07-20T10:00:00.000Z" }],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-20T10:00:00.000Z"));
});

test("deriveLastLiveAt takes the newest across live, archive, and turn anchor", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [
      { anchorAt: Date.parse("2026-07-23T00:00:30.000Z"), channelId: CHANNEL },
    ],
    archivedEvents: [{ timestamp: "2026-07-23T00:00:10.000Z" }],
    channelId: CHANNEL,
    transcript: [{ channelId: CHANNEL, timestamp: "2026-07-23T00:00:20.000Z" }],
  });
  assert.equal(lastLiveAt, Date.parse("2026-07-23T00:00:30.000Z"));
});

test("deriveLastLiveAt falls back to the active-turn anchor with no items", () => {
  const anchorAt = Date.parse("2026-07-23T00:01:00.000Z");
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [{ anchorAt, channelId: CHANNEL }],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, anchorAt);
});

test("deriveLastLiveAt ignores other-channel turns and returns null when idle", () => {
  const lastLiveAt = deriveLastLiveAt({
    activeTurns: [{ anchorAt: 1, channelId: OTHER_CHANNEL }],
    archivedEvents: [],
    channelId: CHANNEL,
    transcript: [],
  });
  assert.equal(lastLiveAt, null);
});
