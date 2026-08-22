import assert from "node:assert/strict";
import test from "node:test";

import {
  buildConversationTurnMeta,
  formatThoughtDisclosureLabel,
} from "./agentSessionConversationMeta.ts";
import { EMPTY_TRANSCRIPT_TURN_META } from "./agentSessionTranscriptContext.ts";
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

function turnBlock(segments) {
  return { kind: "turn", turnId: "turn-1", segments };
}

// ---- buildConversationTurnMeta ----

test("buildConversationTurnMeta returns the shared empty value for other variants", () => {
  for (const variant of ["default", "compactPreview"]) {
    assert.equal(
      buildConversationTurnMeta([turnBlock([])], {
        isTurnLive: true,
        variant,
      }),
      EMPTY_TRANSCRIPT_TURN_META,
      `${variant} must allocate nothing so its render output is untouched`,
    );
  }
});

test("buildConversationTurnMeta times a thought against the turn's next item", () => {
  const meta = buildConversationTurnMeta(
    [
      turnBlock([
        {
          kind: "item",
          item: item({
            id: "thought:1",
            type: "thought",
            renderClass: "thought",
            title: "Thinking",
            text: "…",
            timestamp: "2026-06-14T19:00:02.000Z",
          }),
        },
        {
          kind: "item",
          item: item({
            id: "msg:1",
            type: "message",
            renderClass: "message",
            role: "assistant",
            title: "Agent",
            text: "done",
            timestamp: "2026-06-14T19:00:14.000Z",
          }),
        },
      ]),
    ],
    { isTurnLive: false, variant: "conversation" },
  );

  assert.deepEqual(meta.thoughtDurationSecondsById, { "thought:1": 12 });
  assert.equal(meta.streamingItemId, null);
});

test("buildConversationTurnMeta leaves a trailing thought untimed and marks it streaming when live", () => {
  const blocks = [
    turnBlock([
      {
        kind: "item",
        item: item({
          id: "thought:tail",
          type: "thought",
          renderClass: "thought",
          title: "Thinking",
          text: "…",
          timestamp: "2026-06-14T19:00:02.000Z",
        }),
      },
    ]),
  ];

  const live = buildConversationTurnMeta(blocks, {
    isTurnLive: true,
    variant: "conversation",
  });
  assert.deepEqual(live.thoughtDurationSecondsById, {});
  assert.equal(live.streamingItemId, "thought:tail");

  const idle = buildConversationTurnMeta(blocks, {
    isTurnLive: false,
    variant: "conversation",
  });
  assert.equal(idle.streamingItemId, null);
});

test("buildConversationTurnMeta counts a prompt as an item and skips setup segments", () => {
  const meta = buildConversationTurnMeta(
    [
      turnBlock([
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
        {
          kind: "item",
          item: item({
            id: "thought:1",
            type: "thought",
            renderClass: "thought",
            title: "Thinking",
            text: "…",
            timestamp: "2026-06-14T19:00:01.000Z",
          }),
        },
        {
          kind: "prompt",
          context: null,
          setup: [],
          user: item({
            id: "msg:steer",
            type: "message",
            renderClass: "message",
            role: "user",
            title: "Ada",
            text: "actually…",
            timestamp: "2026-06-14T19:00:04.000Z",
          }),
        },
      ]),
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  // Setup contributes nothing, so the thought is timed against the steer prompt.
  assert.deepEqual(meta.thoughtDurationSecondsById, { "thought:1": 3 });
  assert.equal(meta.streamingItemId, "msg:steer");
});

// ---- formatThoughtDisclosureLabel ----

/**
 * Product ruling (ss-core-02, Slice B review): reasoning folds as soon as the
 * agent ACTS on it — the next leaf item of any kind — not only when the next
 * assistant message arrives. A thought pinned open across a long tool run is
 * exactly the noise the focus view removes. This test exists so the rule is
 * explicit rather than an accident of `items[index + 1]`.
 */
test("buildConversationTurnMeta ends a thought at the next tool call, not the next message", () => {
  const meta = buildConversationTurnMeta(
    [
      turnBlock([
        {
          kind: "item",
          item: item({
            id: "thought:1",
            type: "thought",
            renderClass: "thought",
            title: "Thinking",
            text: "…",
            timestamp: "2026-06-14T19:00:02.000Z",
          }),
        },
        {
          kind: "item",
          item: item({
            id: "tool:1",
            type: "tool",
            renderClass: "shell",
            title: "Ran a command",
            text: "",
            descriptor: { label: "Ran a command", preview: "cargo test" },
            timestamp: "2026-06-14T19:00:06.000Z",
          }),
        },
        {
          kind: "item",
          item: item({
            id: "msg:1",
            type: "message",
            renderClass: "message",
            role: "assistant",
            title: "Agent",
            text: "tests pass",
            timestamp: "2026-06-14T19:01:30.000Z",
          }),
        },
      ]),
    ],
    { isTurnLive: true, variant: "conversation" },
  );

  // 4s to the tool call, NOT 88s to the assistant message: the thought is done
  // the moment the agent acts, so it folds instead of staying open for the run.
  assert.deepEqual(meta.thoughtDurationSecondsById, { "thought:1": 4 });
  // The trailing message is the streaming tail, so the thought is not streaming.
  assert.equal(meta.streamingItemId, "msg:1");
});

test("formatThoughtDisclosureLabel says Thinking while streaming", () => {
  assert.equal(
    formatThoughtDisclosureLabel({ durationSeconds: 4, isStreaming: true }),
    "Thinking…",
  );
});

test("formatThoughtDisclosureLabel reports a duration once the turn moved on", () => {
  assert.equal(
    formatThoughtDisclosureLabel({ durationSeconds: 9, isStreaming: false }),
    "Thought for 9s",
  );
});

test("formatThoughtDisclosureLabel avoids a misleading 0s", () => {
  assert.equal(
    formatThoughtDisclosureLabel({ durationSeconds: 0, isStreaming: false }),
    "Thought for a moment",
  );
  assert.equal(formatThoughtDisclosureLabel({ isStreaming: false }), "Thought");
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
