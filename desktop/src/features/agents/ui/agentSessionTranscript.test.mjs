import assert from "node:assert/strict";
import test from "node:test";

import { buildTranscript } from "./agentSessionTranscript.ts";
import { formatToolTitle } from "./agentSessionToolCatalog.ts";

const baseEvent = {
  seq: 1,
  timestamp: "2026-06-18T00:00:00Z",
  kind: "acp_write",
  agentIndex: 0,
  channelId: "11111111-1111-1111-1111-111111111111",
  sessionId: "sess-1",
  turnId: "turn-1",
};

function acpToolUpdate(seq, update) {
  return {
    ...baseEvent,
    seq,
    kind: "acp_read",
    payload: {
      method: "session/update",
      params: {
        sessionId: baseEvent.sessionId,
        update,
      },
    },
  };
}

function toolItems(events) {
  return buildTranscript(events).filter((item) => item.type === "tool");
}

function activityTitle(item) {
  return formatToolTitle(item.buzzToolName ?? item.toolName, item.title);
}

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

test("buildTranscript keeps read_file activity categorized by the actual tool when output names Buzz tools", () => {
  const [item] = toolItems([
    acpToolUpdate(10, {
      sessionUpdate: "tool_call",
      toolCallId: "call-read-file",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: {
        path: "desktop/src/features/agents/ui/agentSessionToolCatalog.ts",
      },
    }),
    acpToolUpdate(11, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-read-file",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: {
        path: "desktop/src/features/agents/ui/agentSessionToolCatalog.ts",
      },
      content: {
        type: "text",
        text: 'const BUZZ_READ_TOOLS = new Set(["get_feed", "get_event"]);\nconst BUZZ_WRITE_TOOLS = new Set(["delete_message"]);',
      },
    }),
  ]);

  assert.equal(item.toolName, "read_file");
  assert.equal(item.buzzToolName, null);
  assert.equal(item.title, "read_file");
  assert.equal(activityTitle(item), "read_file");
  assert.equal(item.status, "completed");
  assert.match(item.result, /get_feed/);
  assert.match(item.result, /delete_message/);
});

test("buildTranscript keeps shell activity categorized by the actual tool when grep output names Buzz tools", () => {
  const [item] = toolItems([
    acpToolUpdate(20, {
      sessionUpdate: "tool_call",
      toolCallId: "call-shell-rg",
      status: "executing",
      title: "shell",
      kind: "shell",
      rawInput: {
        command: 'rg -n "get_event|delete_message" desktop/src',
      },
    }),
    acpToolUpdate(21, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-shell-rg",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawInput: {
        command: 'rg -n "get_event|delete_message" desktop/src',
      },
      rawOutput:
        'desktop/src/features/agents/ui/agentSessionToolCatalog.ts:83:  "get_event",\n' +
        'desktop/src/features/agents/ui/agentSessionToolCatalog.ts:92:  "delete_message",',
    }),
  ]);

  assert.equal(item.toolName, "shell");
  assert.equal(item.buzzToolName, null);
  assert.equal(activityTitle(item), "shell");
  assert.equal(item.status, "completed");
  assert.match(item.result, /get_event/);
  assert.match(item.result, /delete_message/);
});

test("buildTranscript categorizes explicit Buzz tool calls for the activity bar", () => {
  const [item] = toolItems([
    acpToolUpdate(30, {
      sessionUpdate: "tool_call",
      toolCallId: "call-get-feed",
      status: "executing",
      title: "Tool call",
      toolName: "get_feed",
      rawInput: { limit: 20 },
    }),
    acpToolUpdate(31, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-get-feed",
      status: "completed",
      title: "Tool call",
      toolName: "get_feed",
      content: { type: "text", text: "[]" },
    }),
  ]);

  assert.equal(item.toolName, "get_feed");
  assert.equal(item.buzzToolName, "get_feed");
  assert.equal(activityTitle(item), "Get Feed");
  assert.deepEqual(item.args, { limit: 20 });
  assert.equal(item.status, "completed");
});
