import assert from "node:assert/strict";
import test from "node:test";

import { buildTranscript } from "./agentSessionTranscript.ts";

const baseEvent = {
  seq: 1,
  timestamp: "2026-06-18T00:00:00Z",
  kind: "acp_write",
  agentIndex: 0,
  channelId: "11111111-1111-1111-1111-111111111111",
  sessionId: "sess-1",
  turnId: "turn-1",
};

// --- stub-overflow vanish (pins the pre-existing degraded-frame behavior) ---

test("buildTranscript drops a session/prompt turn whose frame was stubbed by the size trimmer", () => {
  // When fit_observer_event_to_budget cannot shrink a frame below the cap it
  // replaces the whole payload with {elided, originalBytes} (no `method`), so
  // the method-keyed acp_write dispatch matches no arm and there is no terminal
  // else: the turn produces ZERO transcript items. This is worse than a
  // "1 section" collapse (the item vanishes entirely) and is pre-existing,
  // outside the format_prompt seam. Pin it so a later change can't silently
  // regress the vanish-vs-degrade behavior without updating this test.
  const stubbed = {
    ...baseEvent,
    payload: {
      elided: "acp_write payload too large",
      originalBytes: 123456,
    },
  };

  assert.deepEqual(buildTranscript([stubbed]), []);
});

// --- positive control: a well-formed multi-block prompt DOES render ---

test("buildTranscript renders Prompt context + user message for a multi-block session/prompt frame", () => {
  // Guards the vanish assertion above against a false pass from a broken
  // import or dispatch: a normal per-section prompt frame must still produce a
  // user message and a "Prompt context" metadata item.
  const event = {
    ...baseEvent,
    payload: {
      method: "session/prompt",
      params: {
        sessionId: "sess-1",
        prompt: [
          { type: "text", text: "[Agent Memory — core]\nremember this" },
          { type: "text", text: "[Context]\nScope: thread" },
          {
            type: "text",
            text: `[Buzz event: @mention]\nFrom: x (hex: ${"a".repeat(64)})\nContent: hello`,
          },
        ],
      },
    },
  };

  const items = buildTranscript([event]);
  const titles = items.map((i) => i.title);
  assert.ok(
    items.some((i) => i.type === "metadata" && i.title === "Prompt context"),
    `expected a Prompt context metadata item, got titles: ${titles.join(", ")}`,
  );
  const promptContext = items.find((i) => i.title === "Prompt context");
  assert.deepEqual(
    promptContext.sections.map((s) => s.title),
    ["Agent Memory — core", "Context", "Buzz event: @mention"],
    "every section header is counted",
  );
});
