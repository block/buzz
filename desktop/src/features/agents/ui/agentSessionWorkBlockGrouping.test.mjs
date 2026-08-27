/**
 * Work-block grouping: which of a turn's items fold onto the rail, where the
 * block splits, and what its folded line says.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  formatPreviousStepsLabel,
  formatWorkBlockSummaryLabel,
  groupConversationWorkBlocks,
  projectWorkBlockEntries,
  summarizeWorkBlock,
  windowWorkBlockEntries,
  WORK_BLOCK_LIVE_WINDOW_SIZE,
} from "./agentSessionWorkBlockGrouping.ts";

const SHARED = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };

/**
 * Project the block's items with the block's own turn live — i.e. the agent is
 * working on `turn-1` right now, which is the turn every fixture here belongs
 * to. Most projection cases are about what a step looks like while its own turn
 * is being worked, so this is the default lens.
 */
const projectLive = (items) =>
  projectWorkBlockEntries(items, { liveTurnId: "turn-1" });

/**
 * Project with no live turn — reopened history, or a session that has ended.
 * The distinction matters because a tool item's `executing` status is written
 * once at the start and never revised if the agent dies, so it is only evidence
 * of work in flight when a session still owns the turn.
 */
const projectHistory = (items) =>
  projectWorkBlockEntries(items, { liveTurnId: null });

function thought(id, timestamp = "2026-06-14T19:00:02.000Z") {
  return {
    ...SHARED,
    id,
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: "weighing the options",
    timestamp,
  };
}

function tool(id, overrides = {}) {
  return {
    ...SHARED,
    id,
    type: "tool",
    renderClass: "shell",
    descriptor: {
      renderClass: "shell",
      label: "Ran a command",
      preview: "cargo test",
      tone: "neutral",
      source: "shell",
    },
    title: "Ran a command",
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: { command: "cargo test" },
    result: "ok",
    isError: false,
    timestamp: "2026-06-14T19:00:04.000Z",
    startedAt: "2026-06-14T19:00:04.000Z",
    completedAt: "2026-06-14T19:00:05.000Z",
    ...overrides,
  };
}

function message(id, role, overrides = {}) {
  return {
    ...SHARED,
    id,
    type: "message",
    renderClass: "message",
    role,
    title: role === "user" ? "Ada" : "Test Agent",
    text: "hello",
    timestamp: "2026-06-14T19:00:09.000Z",
    ...overrides,
  };
}

function plan(id) {
  return {
    ...SHARED,
    id,
    type: "plan",
    renderClass: "plan",
    title: "Plan",
    text: "- [ ] ship it",
    timestamp: "2026-06-14T19:00:07.000Z",
  };
}

function lifecycle(id, renderClass) {
  return {
    ...SHARED,
    id,
    type: "lifecycle",
    renderClass,
    title:
      renderClass === "permission"
        ? "Permission requested"
        : "Context compacted",
    text: "",
    timestamp: "2026-06-14T19:00:06.000Z",
  };
}

const itemSegments = (...items) =>
  items.map((item) => ({ kind: "item", item }));
const kinds = (segments) => segments.map((segment) => segment.kind);
const blockItemIds = (segment) => segment.block.items.map((item) => item.id);

// ---- membership ----

test("thinking, tool steps and the prompt/answer split fold into one block", () => {
  const prompt = {
    kind: "prompt",
    user: message("msg:user", "user"),
    context: null,
    setup: [],
  };
  const answer = message("msg:answer", "assistant");
  const grouped = groupConversationWorkBlocks([
    prompt,
    ...itemSegments(thought("thought:1"), tool("tool:1"), tool("tool:2")),
    { kind: "item", item: answer },
  ]);

  assert.deepEqual(kinds(grouped), ["prompt", "work-block", "item"]);
  assert.deepEqual(blockItemIds(grouped[1]), ["thought:1", "tool:1", "tool:2"]);
  assert.equal(
    grouped[2].item.id,
    "msg:answer",
    "the turn's answer stays outside the block as prose",
  );
});

test("a single work item still becomes a block", () => {
  // No minimum: a lone step must not render through a different mechanism than
  // a run of steps, or the rail would appear and disappear by count.
  const grouped = groupConversationWorkBlocks(itemSegments(tool("tool:1")));
  assert.deepEqual(kinds(grouped), ["work-block"]);
  assert.deepEqual(blockItemIds(grouped[0]), ["tool:1"]);
});

test("an interim agent note is work but the final answer is not", () => {
  const grouped = groupConversationWorkBlocks(
    itemSegments(
      tool("tool:1"),
      message("msg:interim", "assistant"),
      tool("tool:2"),
      message("msg:answer", "assistant"),
    ),
  );

  assert.deepEqual(kinds(grouped), ["work-block", "item"]);
  assert.deepEqual(
    blockItemIds(grouped[0]),
    ["tool:1", "msg:interim", "tool:2"],
    "a mid-turn note reads as progress and belongs on the rail",
  );
  assert.equal(grouped[1].item.id, "msg:answer");
});

test("only the LAST assistant message is treated as the answer", () => {
  // Two trailing notes: the earlier one is still work.
  const grouped = groupConversationWorkBlocks(
    itemSegments(
      tool("tool:1"),
      message("msg:first", "assistant"),
      message("msg:last", "assistant"),
    ),
  );
  assert.deepEqual(kinds(grouped), ["work-block", "item"]);
  assert.deepEqual(blockItemIds(grouped[0]), ["tool:1", "msg:first"]);
  assert.equal(grouped[1].item.id, "msg:last");
});

test("a turn with no answer folds all of its work", () => {
  // Mid-turn: nothing has been answered yet, so every item is work and the
  // block must not hold back its most recent step waiting for an answer.
  const grouped = groupConversationWorkBlocks(
    itemSegments(thought("thought:1"), tool("tool:1")),
  );
  assert.deepEqual(kinds(grouped), ["work-block"]);
  assert.deepEqual(blockItemIds(grouped[0]), ["thought:1", "tool:1"]);
});

// ---- what stays out ----

test("the plan stays a sibling and splits the work around it", () => {
  const grouped = groupConversationWorkBlocks(
    itemSegments(tool("tool:1"), plan("plan:1"), tool("tool:2")),
  );

  assert.deepEqual(kinds(grouped), ["work-block", "item", "work-block"]);
  assert.equal(grouped[1].item.id, "plan:1");
  assert.deepEqual(blockItemIds(grouped[0]), ["tool:1"]);
  assert.deepEqual(blockItemIds(grouped[2]), ["tool:2"]);
});

test("message sends stay outside the work block as chat bubbles", () => {
  const send = tool("tool:send", {
    args: { content: "Posted an update" },
    buzzToolName: "send_message",
    descriptor: {
      renderClass: "message",
      label: "Sent message",
      preview: "Posted an update",
      tone: "neutral",
      source: "buzz",
    },
    renderClass: "message",
    toolName: "send_message",
  });
  const grouped = groupConversationWorkBlocks(
    itemSegments(tool("tool:1"), send, tool("tool:2")),
  );

  assert.deepEqual(kinds(grouped), ["work-block", "item", "work-block"]);
  assert.deepEqual(blockItemIds(grouped[0]), ["tool:1"]);
  assert.equal(
    grouped[1].item.id,
    "tool:send",
    "the existing message presenter can render the send as a readable bubble",
  );
  assert.deepEqual(blockItemIds(grouped[2]), ["tool:2"]);
});

test("permission gates and errors stay loud, in position", () => {
  for (const renderClass of ["permission", "error", "status"]) {
    const grouped = groupConversationWorkBlocks(
      itemSegments(
        tool("tool:1"),
        lifecycle(`life:${renderClass}`, renderClass),
        tool("tool:2"),
      ),
    );
    assert.deepEqual(
      kinds(grouped),
      ["work-block", "item", "work-block"],
      `${renderClass} must never fold into a collapsed block`,
    );
    assert.equal(
      grouped[1].item.id,
      `life:${renderClass}`,
      `${renderClass} must stay where it happened, not be lifted out of order`,
    );
  }
});

test("the user's prompt is never work", () => {
  const grouped = groupConversationWorkBlocks([
    {
      kind: "prompt",
      user: message("msg:user", "user"),
      context: null,
      setup: [],
    },
  ]);
  assert.deepEqual(kinds(grouped), ["prompt"]);
});

test("setup segments neither join a block nor split one", () => {
  // Setup renders as a quiet divider, not as work, and contributes no items —
  // so it must not sit between two blocks that ought to be one.
  const grouped = groupConversationWorkBlocks([
    { kind: "item", item: tool("tool:1") },
    { kind: "setup", items: [] },
    { kind: "item", item: tool("tool:2") },
  ]);
  assert.deepEqual(kinds(grouped), ["work-block", "setup", "work-block"]);
});

test("a default-variant summary is expanded back to its leaf steps", () => {
  // The other variants' "Read 3 files" summaries are a competing answer to the
  // same grouping problem. Nesting one inside the block would cost the reader a
  // second click to reach a step.
  const grouped = groupConversationWorkBlocks([
    {
      kind: "summary",
      summary: {
        id: "summary:shell:tool:1",
        label: "Ran 2 commands",
        count: 2,
        items: [tool("tool:1"), tool("tool:2")],
        renderClass: "shell",
        variant: "same-kind",
        timestamp: "2026-06-14T19:00:04.000Z",
      },
    },
  ]);

  assert.deepEqual(kinds(grouped), ["work-block"]);
  assert.deepEqual(blockItemIds(grouped[0]), ["tool:1", "tool:2"]);
});

// ---- identity ----

test("the block id is derived from its first step so appends do not remount it", () => {
  const first = groupConversationWorkBlocks(itemSegments(tool("tool:1")));
  const grown = groupConversationWorkBlocks(
    itemSegments(tool("tool:1"), tool("tool:2"), tool("tool:3")),
  );
  assert.equal(
    grown[0].block.id,
    first[0].block.id,
    "a growing block must keep its identity or the reader's disclosure choice is lost",
  );
});

// ---- projection ----

test("every item projects to exactly one rail kind, paired with its own item type", () => {
  // The closed set is the point: glyph and body render off a switch over these
  // three, so an item that projected to nothing (or to two things) is what let
  // a note pick up a wrench on the abandoned card.
  //
  // The pairing half is what makes the body switch able to read `item.text`
  // directly. `WorkBlockEntry` is a discriminated union, so `{ kind: "note",
  // item: <thought> }` does not type-check — but a hand-written swap between the
  // two prose branches is the easy mistake, and TypeScript is stripped at
  // runtime, so the pairing is asserted here as well.
  const entries = projectLive([
    thought("thought:1"),
    message("msg:interim", "assistant"),
    tool("tool:1"),
  ]);
  assert.deepEqual(
    entries.map((entry) => entry.kind),
    ["thought", "note", "tool"],
  );
  assert.deepEqual(
    entries.map((entry) => [entry.kind, entry.item.type]),
    [
      ["thought", "thought"],
      ["note", "message"],
      ["tool", "tool"],
    ],
    "a kind always carries its own item type — the body switch reads the item without re-checking",
  );
  assert.deepEqual(
    entries.map((entry) => entry.item.id),
    ["thought:1", "msg:interim", "tool:1"],
    "projection preserves arrival order",
  );
});

test("only a tool step can be running or failed", () => {
  const stateOf = (item) => projectLive([item])[0].state;

  assert.equal(stateOf(tool("t", { status: "executing" })), "running");
  assert.equal(stateOf(tool("t", { status: "pending" })), "running");
  assert.equal(
    stateOf(tool("t", { status: "failed" })),
    "failed",
    "a failed status is a failure even without isError",
  );
  assert.equal(
    stateOf(tool("t", { isError: true })),
    "failed",
    "an error result is a failure even when the status reads completed",
  );
  assert.equal(stateOf(tool("t")), "settled");

  // Prose has no outcome of its own. A thought whose text happens to mention a
  // failure, or a note, must not colour the rail or the folded count.
  assert.equal(stateOf(thought("thought:1")), "settled");
  assert.equal(stateOf(message("msg:1", "assistant")), "settled");
});

test("a step that is both running and errored still reads as running", () => {
  // Order matters: a tool can carry a stale isError from a retry while the new
  // attempt executes. Reporting it as failed would fold a live block's count to
  // "N steps · 1 failed" while the work is still in flight.
  assert.equal(
    projectLive([tool("t", { status: "executing", isError: true })])[0].state,
    "running",
  );
});

// ---- liveness ----

/**
 * An abandoned step is not a running step.
 *
 * `executing` is written when a step starts and never revised if the agent dies
 * first, so reopened history keeps that status forever. In this block `running`
 * is not just a glyph — one running entry makes `summarizeWorkBlock` report
 * `isActive`, which suppresses the folded summary line and holds the rail open.
 * Ungated, a single orphaned step therefore renders finished history as work in
 * progress, pulsing indefinitely.
 */
test("an executing step whose turn is not live reads as settled, not running", () => {
  const orphan = tool("t", { status: "executing", completedAt: null });

  assert.equal(
    projectLive([orphan])[0].state,
    "running",
    "the same item IS running while a session owns its turn",
  );
  assert.equal(
    projectHistory([orphan])[0].state,
    "settled",
    "with no live turn the status only says the step began, not that it is happening",
  );
  assert.equal(
    projectWorkBlockEntries([orphan], { liveTurnId: "turn-2" })[0].state,
    "settled",
    "an agent live on a LATER turn does not resurrect an earlier turn's abandoned step",
  );
});

test("a pending step whose turn is not live reads as settled too", () => {
  // Both in-flight statuses go through the same gate; `pending` is the one a
  // crash between queue and start leaves behind.
  const queued = tool("t", { status: "pending", completedAt: null });
  assert.equal(projectLive([queued])[0].state, "running");
  assert.equal(projectHistory([queued])[0].state, "settled");
});

test("an abandoned step is settled rather than failed, so it never inflates the failure count", () => {
  // We do not know an abandoned step failed — only that nobody finished it.
  // Counting it as a failure would put "1 failed" on the folded line for work
  // that may well have succeeded without its terminal update being recorded.
  const status = summarizeWorkBlock(
    projectHistory([
      tool("tool:1"),
      tool("tool:2", { status: "executing", completedAt: null }),
    ]),
    { streamingItemId: null },
  );
  assert.deepEqual(status, { count: 2, failedCount: 0, isActive: false });
  assert.equal(
    formatWorkBlockSummaryLabel(status),
    "2 steps",
    "reopened history folds to a neutral count",
  );
});

test("a genuinely failed step is still failed when its turn is not live", () => {
  // The gate is only about the in-flight statuses. A step that recorded a
  // failure recorded a fact, and history must keep reporting it.
  const failed = () => tool("t", { isError: true, status: "failed" });
  assert.equal(projectHistory([failed()])[0].state, "failed");
  assert.equal(
    summarizeWorkBlock(projectHistory([failed()]), { streamingItemId: null })
      .failedCount,
    1,
  );
});

test("a step with no turn id is not owned by a live turn", () => {
  // `null === null` must not read as ownership: an item that never recorded a
  // turn cannot be shown to belong to the live one.
  assert.equal(
    projectWorkBlockEntries(
      [tool("t", { status: "executing", completedAt: null, turnId: null })],
      { liveTurnId: null },
    )[0].state,
    "settled",
  );
});

// ---- status ----

test("summarizeWorkBlock counts steps and failures", () => {
  const status = summarizeWorkBlock(
    projectLive([
      tool("tool:1"),
      tool("tool:2", { isError: true, status: "failed" }),
      thought("thought:1"),
    ]),
    { streamingItemId: null },
  );
  assert.deepEqual(status, { count: 3, failedCount: 1, isActive: false });
});

test("a block is active when a step is running OR when it holds the streaming item", () => {
  assert.equal(
    summarizeWorkBlock(
      projectLive([tool("tool:1", { status: "executing", completedAt: null })]),
      { streamingItemId: null },
    ).isActive,
    true,
    "a step reporting itself as executing is enough",
  );

  // A thought streaming in carries no tool status, so status alone would miss it.
  const streamingThought = projectLive([thought("thought:1")]);
  assert.equal(
    summarizeWorkBlock(streamingThought, { streamingItemId: "thought:1" })
      .isActive,
    true,
    "the list's streaming hint covers work that carries no status of its own",
  );
  assert.equal(
    summarizeWorkBlock(streamingThought, { streamingItemId: "other" }).isActive,
    false,
    "a streaming item in a DIFFERENT block must not make this one live",
  );
});

// ---- labels ----

test("the folded line names a failure rather than hiding it behind a count", () => {
  assert.equal(
    formatWorkBlockSummaryLabel({ count: 6, failedCount: 0, isActive: false }),
    "6 steps",
  );
  assert.equal(
    formatWorkBlockSummaryLabel({ count: 6, failedCount: 1, isActive: false }),
    "6 steps · 1 failed",
  );
  assert.equal(
    formatWorkBlockSummaryLabel({ count: 6, failedCount: 2, isActive: false }),
    "6 steps · 2 failed",
  );
  assert.equal(
    formatWorkBlockSummaryLabel({ count: 1, failedCount: 0, isActive: false }),
    "1 step",
    "a single step must not read as '1 steps'",
  );
});

test("the previous-steps label is singular for one step", () => {
  assert.equal(formatPreviousStepsLabel(1), "1 previous step");
  assert.equal(formatPreviousStepsLabel(4), "4 previous steps");
});

// ---- live window ----

const entryIds = (entries) => entries.map((entry) => entry.item.id);

test("a live block shows the last N steps in true order and hides the rest", () => {
  const entries = projectLive(["a", "b", "c", "d", "e"].map((id) => tool(id)));
  const { hiddenEntries, visibleEntries } = windowWorkBlockEntries(entries, {
    isActive: true,
  });

  assert.equal(visibleEntries.length, WORK_BLOCK_LIVE_WINDOW_SIZE);
  assert.deepEqual(
    entryIds(visibleEntries),
    ["c", "d", "e"],
    "the window is chronological — arrival order, not reversed",
  );
  assert.deepEqual(entryIds(hiddenEntries), ["a", "b"]);
});

test("a finished block shows every step", () => {
  const entries = projectLive(["a", "b", "c", "d", "e"].map((id) => tool(id)));
  const { hiddenEntries, visibleEntries } = windowWorkBlockEntries(entries, {
    isActive: false,
  });
  assert.equal(hiddenEntries.length, 0);
  assert.equal(visibleEntries.length, 5);
});

test("a live block at or under the window size hides nothing", () => {
  const entries = projectLive(["a", "b", "c"].map((id) => tool(id)));
  const { hiddenEntries, visibleEntries } = windowWorkBlockEntries(entries, {
    isActive: true,
  });
  assert.equal(
    hiddenEntries.length,
    0,
    "no disclosure until there is a step to hide",
  );
  assert.equal(visibleEntries.length, 3);
});
