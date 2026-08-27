import assert from "node:assert/strict";
import test from "node:test";

import { buildConversationTurnMeta } from "./agentSessionConversationMeta.ts";
import { EMPTY_TRANSCRIPT_TURN_META } from "./agentSessionTranscriptContext.ts";
import { buildTranscriptDisplayBlocks } from "./agentSessionTranscriptGrouping.ts";
import {
  formatPlanChecklistProgress,
  parsePlanChecklist,
} from "./agentSessionPlanChecklist.ts";

function item(overrides) {
  return {
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    ...overrides,
  };
}

const RAW_TIMESTAMP = "2026-06-14T19:00:00.000Z";

/**
 * Raw transcript items, for the tests that go through the real grouping rather
 * than asserting on hand-written blocks. `acpSource` is what decides whether an
 * item is setup lifecycle, a prompt, or work, so these carry it.
 */
function lifecycleItem(id, acpSource, turnId) {
  return item({
    id,
    type: "lifecycle",
    renderClass: "status",
    title: acpSource,
    text: "",
    timestamp: RAW_TIMESTAMP,
    acpSource,
    turnId,
  });
}

function promptItem(id, turnId) {
  return item({
    id,
    type: "message",
    role: "user",
    renderClass: "message",
    title: "Buzz event",
    text: "please look into this",
    timestamp: RAW_TIMESTAMP,
    acpSource: "session/prompt:user",
    turnId,
  });
}

function toolItem(id, turnId) {
  return item({
    id,
    type: "tool",
    renderClass: "shell",
    title: id,
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "ok",
    isError: false,
    timestamp: RAW_TIMESTAMP,
    startedAt: RAW_TIMESTAMP,
    completedAt: RAW_TIMESTAMP,
    descriptor: {
      renderClass: "shell",
      label: "Ran command",
      preview: id,
      source: "shell",
      groupKey: "shell:command",
    },
    turnId,
  });
}

function systemPromptItem(id) {
  return item({
    id,
    type: "metadata",
    renderClass: "metadata",
    title: "System prompt",
    sections: [],
    timestamp: RAW_TIMESTAMP,
    acpSource: "session/new",
    turnId: null,
  });
}

function turnBlock(segments) {
  return { kind: "turn", turnId: "turn-1", segments };
}

/**
 * The leaf items a block is built from, in wire order — including setup
 * lifecycle items, which carry a turn id in the real stream even though they
 * contribute nothing to the streaming tail.
 *
 * `buildConversationTurnMeta` reads the item stream as well as the blocks (it
 * has to: a turn that has only emitted setup rows produces no block at all), so
 * a test that passed blocks alone would be describing a transcript that cannot
 * exist. This keeps the two arguments consistent by construction.
 */
function blockItems(blocks) {
  return blocks.flatMap((block) => {
    if (block.kind === "single") return [block.item];
    if (block.kind !== "turn") return [];
    return block.segments.flatMap((segment) => {
      if (segment.kind === "prompt") return [segment.user];
      if (segment.kind === "setup") return segment.items;
      if (segment.kind === "summary") return segment.summary.items;
      return [segment.item];
    });
  });
}

/** `buildConversationTurnMeta` with the item stream derived from the blocks. */
function metaFor(blocks, { items, ...options }) {
  return buildConversationTurnMeta(blocks, {
    ...options,
    items: items ?? blockItems(blocks),
  });
}

function itemSegment(id, overrides) {
  return {
    kind: "item",
    item: item({
      id,
      timestamp: "2026-06-14T19:00:02.000Z",
      ...overrides,
    }),
  };
}

const thoughtSegment = (id, timestamp) =>
  itemSegment(id, {
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: "…",
    timestamp,
  });

const messageSegment = (id, timestamp, role = "assistant") =>
  itemSegment(id, {
    type: "message",
    renderClass: "message",
    role,
    title: role === "user" ? "Ada" : "Agent",
    text: "done",
    timestamp,
  });

const toolSegment = (id, timestamp) =>
  itemSegment(id, {
    type: "tool",
    renderClass: "shell",
    title: "Ran a command",
    text: "",
    descriptor: { label: "Ran a command", preview: "cargo test" },
    timestamp,
  });

// ---- buildConversationTurnMeta ----

test("buildConversationTurnMeta returns the shared empty value for other variants", () => {
  for (const variant of ["default", "compactPreview"]) {
    assert.equal(
      metaFor([turnBlock([])], {
        isTurnLive: true,
        variant,
      }),
      EMPTY_TRANSCRIPT_TURN_META,
      `${variant} must allocate nothing so its render output is untouched`,
    );
  }
});

test("buildConversationTurnMeta reports nothing streaming when the turn is idle", () => {
  // The hint exists to tell the work block it is still working. A finished turn
  // has no tail, and reporting one would pin the block open forever.
  assert.equal(
    metaFor(
      [
        turnBlock([
          thoughtSegment("thought:1", "2026-06-14T19:00:02.000Z"),
          messageSegment("msg:1", "2026-06-14T19:00:14.000Z"),
        ]),
      ],
      { isTurnLive: false, variant: "conversation" },
    ),
    EMPTY_TRANSCRIPT_TURN_META,
  );
});

test("buildConversationTurnMeta names the trailing item of a live turn", () => {
  const blocks = [
    turnBlock([thoughtSegment("thought:tail", "2026-06-14T19:00:02.000Z")]),
  ];

  assert.equal(
    metaFor(blocks, {
      isTurnLive: true,
      variant: "conversation",
    }).streamingItemId,
    "thought:tail",
    "a thought carries no status of its own, so the hint is how the block knows it is live",
  );
});

test("buildConversationTurnMeta skips setup segments when finding the tail", () => {
  // Setup renders as a quiet divider, not as work, so it must never be reported
  // as the streaming item — a lifecycle row would hold the block open.
  const meta = metaFor(
    [
      turnBlock([
        thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z"),
        {
          kind: "setup",
          items: [
            item({
              id: "life:1",
              type: "lifecycle",
              renderClass: "status",
              title: "Turn started",
              text: "",
              timestamp: "2026-06-14T19:00:00.000Z",
            }),
          ],
        },
      ]),
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.streamingItemId, "thought:1");
});

test("buildConversationTurnMeta counts a mid-turn steer prompt as the tail", () => {
  const meta = metaFor(
    [
      turnBlock([
        thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z"),
        messageSegment("msg:steer", "2026-06-14T19:00:04.000Z", "user"),
      ]),
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.streamingItemId, "msg:steer");
});

test("buildConversationTurnMeta expands a summary segment to reach its last tool", () => {
  // A summary segment holds several leaf items; the tail is the last of them,
  // not the summary itself, which has no item id the work block could match.
  const meta = metaFor(
    [
      turnBlock([
        {
          kind: "summary",
          summary: {
            id: "summary:shell:tool:1",
            label: "Ran 2 commands",
            count: 2,
            items: [
              toolSegment("tool:1", "2026-06-14T19:00:04.000Z").item,
              toolSegment("tool:2", "2026-06-14T19:00:05.000Z").item,
            ],
            renderClass: "shell",
            variant: "same-kind",
            timestamp: "2026-06-14T19:00:04.000Z",
          },
        },
      ]),
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.streamingItemId, "tool:2");
});

test("buildConversationTurnMeta reads a trailing single block directly", () => {
  const meta = metaFor(
    [
      turnBlock([thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z")]),
      {
        kind: "single",
        item: item({
          id: "life:orphan",
          type: "lifecycle",
          renderClass: "status",
          title: "Context compacted",
          text: "",
          timestamp: "2026-06-14T19:00:20.000Z",
        }),
      },
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.streamingItemId, "life:orphan");
});

// ---- buildConversationTurnMeta: liveTurnId ----

/**
 * The work block cannot tell an abandoned step from a running one on its own: a
 * tool keeps its `executing` status forever if the agent dies mid-step. Turn
 * ownership is knowledge only the list has, so it is published here.
 */
test("buildConversationTurnMeta names the live turn so the block can gate running steps", () => {
  const meta = metaFor(
    [turnBlock([thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z")])],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.liveTurnId, "turn-1");
});

test("buildConversationTurnMeta reports no live turn when the turn is idle", () => {
  // This is the case the orphaned-step bug lived in: history reopened, nothing
  // live, but an item still claiming `executing`.
  const meta = metaFor(
    [turnBlock([thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z")])],
    { isTurnLive: false, variant: "conversation" },
  );

  assert.equal(meta.liveTurnId, null);
  assert.equal(
    meta,
    EMPTY_TRANSCRIPT_TURN_META,
    "an idle turn still allocates nothing",
  );
});

test("buildConversationTurnMeta names the live turn even when a lifecycle row trails it", () => {
  // A compaction notice arrives as a `single` block AFTER the turn it belongs
  // to, so "the last block" is not always the live turn — but the last turn is.
  // Reading the last block's kind alone would report no live turn and gate every
  // running step off, which is the mirror image of the bug being fixed.
  const meta = metaFor(
    [
      turnBlock([thoughtSegment("thought:1", "2026-06-14T19:00:01.000Z")]),
      {
        kind: "single",
        item: item({
          id: "life:compact",
          type: "lifecycle",
          renderClass: "status",
          title: "Context compacted",
          text: "",
          timestamp: "2026-06-14T19:00:20.000Z",
        }),
      },
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.liveTurnId, "turn-1");
});

test("buildConversationTurnMeta reports no live turn when there is no turn at all", () => {
  // `turnId: null` explicitly: a session-started row arrives before any turn
  // exists, so it belongs to nobody. The shared `item()` helper defaults to
  // `turn-1`, which would have made this fixture describe a transcript that
  // does have a turn — and the assertion would then be checking that a turn
  // with no BLOCK reports no live turn, which is the opposite of the rule.
  const meta = metaFor(
    [
      {
        kind: "single",
        item: item({
          id: "life:1",
          type: "lifecycle",
          renderClass: "status",
          title: "Session started",
          text: "",
          timestamp: "2026-06-14T19:00:00.000Z",
          turnId: null,
        }),
      },
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  assert.equal(meta.liveTurnId, null);
});

// ---- buildConversationTurnMeta: the gap between two turns ----

/**
 * The next turn owns liveness the moment it appears, even before it has anything
 * to show.
 *
 * A turn that has only emitted setup lifecycle rows (`turn_started`,
 * `session_resolved`) classifies to zero segments and therefore produces NO
 * block — `agentSessionTranscriptGrouping` only pushes a turn block
 * `if (segments.length > 0)`. So "the newest turn with a block" is the turn that
 * ALREADY ENDED for the whole gap between `turn_started` and the new turn's
 * first prompt or thought, and the ended turn's own trailing item then gets
 * reported as the streaming item. Its finished work block reads as live again:
 * the folded summary is replaced by the open rail, a 6-step block drops to its
 * last three with a previous-steps disclosure, and then it all folds back.
 *
 * That gap is real observer-stream latency, and `turn_started` fires on every
 * turn (`activeAgentTurnsStore`), so this was not an edge case — it was every
 * turn, for as long as the agent took to emit its first renderable item.
 *
 * Built from a raw item stream through the real grouping rather than from
 * hand-written blocks, because the whole bug is which turns DO and DO NOT
 * produce a block: hand-built blocks would assume the answer away.
 *
 * The frames pin the PLAIN next turn (`turn_started`, then `session_resolved`,
 * with no `session/new` card) because that is the common path and the only one
 * that pins the bug. On a session restart the `session/new` card lands as a
 * trailing `single` block, which moves `streamingItemId` off turn-1's work block
 * on its own — so a restart-only fixture would pass against the old
 * block-walking code. Both sequences are covered; only the plain one is
 * load-bearing.
 */
test("buildConversationTurnMeta hands liveness to a next turn that has no block yet", () => {
  const stream = (...extra) => [
    lifecycleItem("life:start:1", "turn_started", "turn-1"),
    promptItem("prompt:1", "turn-1"),
    toolItem("tool:1", "turn-1"),
    // turn-1's last act is a completed tool: the agent answered by posting
    // through buzz-cli, so no assistant message trails its work.
    toolItem("tool:2", "turn-1"),
    ...extra,
  ];

  const started = lifecycleItem("life:start:2", "turn_started", "turn-2");
  const resolved = lifecycleItem(
    "life:resolved:2",
    "session_resolved",
    "turn-2",
  );
  const sysPrompt = systemPromptItem("meta:sysprompt");

  const frames = [
    // The plain next turn — no session restart, so no `session/new` card. This
    // is both the common path and the only one that pins the bug: a
    // `session/new` card renders as a trailing `single` block, which moves the
    // streaming item off turn-1's work block by itself and so lets the old
    // block-walking code pass for the wrong reason.
    {
      label: "turn-2 has only started",
      items: stream(started),
    },
    {
      label: "the session has resolved with no restart card",
      items: stream(started, resolved),
    },
    // The restarting next turn, where a system-prompt card lands between the
    // lifecycle rows. A different route through the grouping, kept because it
    // has to settle too.
    {
      label: "the restarting turn's system prompt has landed",
      items: stream(started, sysPrompt),
    },
    {
      label: "the restarted session has resolved",
      items: stream(started, sysPrompt, resolved),
    },
  ];

  for (const frame of frames) {
    const meta = buildConversationTurnMeta(
      buildTranscriptDisplayBlocks(frame.items),
      { isTurnLive: true, items: frame.items, variant: "conversation" },
    );
    assert.equal(
      meta.liveTurnId,
      "turn-2",
      `the newest turn owns liveness once ${frame.label}`,
    );
    assert.equal(
      meta.streamingItemId,
      null,
      `turn-1's tail must not read as streaming once ${frame.label}`,
    );
  }

  // Final frame: turn-2 finally has something renderable, so it gets a block
  // and its own prompt becomes the tail. Included so the sequence ends in the
  // steady state rather than stopping at the gap.
  const renderable = stream(
    started,
    resolved,
    promptItem("prompt:2", "turn-2"),
  );
  const meta = buildConversationTurnMeta(
    buildTranscriptDisplayBlocks(renderable),
    { isTurnLive: true, items: renderable, variant: "conversation" },
  );
  assert.equal(meta.liveTurnId, "turn-2");
  assert.equal(meta.streamingItemId, "prompt:2");
});

test("buildConversationTurnMeta still names the live turn's own trailing item", () => {
  // The other half of the ownership check: when the newest turn IS the turn that
  // owns the trailing item, that item is the streaming one. Without this the
  // gate would be a blanket "never report a tail" and every live block would
  // fold while the reader watched it arrive.
  const items = [
    lifecycleItem("life:start:1", "turn_started", "turn-1"),
    promptItem("prompt:1", "turn-1"),
    toolItem("tool:1", "turn-1"),
  ];
  const meta = buildConversationTurnMeta(buildTranscriptDisplayBlocks(items), {
    isTurnLive: true,
    items,
    variant: "conversation",
  });

  assert.equal(meta.liveTurnId, "turn-1");
  assert.equal(meta.streamingItemId, "tool:1");
});

// ---- parsePlanChecklist ----

test("parsePlanChecklist reads the checkbox markdown the transcript builds", () => {
  const checklist = parsePlanChecklist(
    [
      "- [x] read the transcript",
      "- [ ] write the summary (in progress)",
      "- [ ] ship it",
    ].join("\n"),
  );

  assert.deepEqual(checklist.entries, [
    { label: "read the transcript", status: "completed" },
    { label: "write the summary", status: "in_progress" },
    { label: "ship it", status: "pending" },
  ]);
  assert.equal(checklist.completedCount, 1);
  assert.equal(formatPlanChecklistProgress(checklist), "1/3 complete");
});

test("parsePlanChecklist yields nothing for free-form plan text", () => {
  const checklist = parsePlanChecklist("We will read, then write, then ship.");
  assert.deepEqual(checklist.entries, []);
  assert.equal(formatPlanChecklistProgress(checklist), null);
});
