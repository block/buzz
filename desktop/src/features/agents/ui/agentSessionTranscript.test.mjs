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
const PROMPT_EVENT_ID = "c".repeat(64);

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
            text: `[Buzz event: @mention]\nEvent ID: ${PROMPT_EVENT_ID.toUpperCase()}\nFrom: x (hex: ${"a".repeat(64)})\nContent: hello`,
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
  const userMessage = items.find((i) => i.type === "message");
  assert.equal(userMessage.messageId, PROMPT_EVENT_ID);
});

test("buildTranscript falls back to a single turn trigger id for older prompt frames", () => {
  const promptEvent = {
    ...baseEvent,
    seq: 2,
    payload: {
      method: "session/prompt",
      params: {
        sessionId: "sess-1",
        prompt: [
          {
            type: "text",
            text: `[Buzz event: @mention]\nFrom: x (hex: ${"a".repeat(64)})\nContent: hello`,
          },
        ],
      },
    },
  };
  const [userMessage] = buildTranscript([
    {
      ...baseEvent,
      seq: 1,
      kind: "turn_started",
      payload: {
        triggeringEventIds: [PROMPT_EVENT_ID],
      },
    },
    promptEvent,
  ]).filter((candidate) => candidate.type === "message");

  assert.equal(userMessage.messageId, PROMPT_EVENT_ID);
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

function sessionUpdate(seq, update, overrides = {}) {
  return {
    ...baseEvent,
    ...overrides,
    seq,
    kind: "acp_read",
    payload: {
      method: "session/update",
      params: {
        sessionId: overrides.sessionId ?? baseEvent.sessionId,
        update,
      },
    },
  };
}

function assistantChunk(seq, messageId, text, overrides = {}) {
  return sessionUpdate(
    seq,
    {
      sessionUpdate: "agent_message_chunk",
      messageId,
      content: { type: "text", text },
    },
    overrides,
  );
}

test("buildTranscript preserves author pubkeys on user message chunks", () => {
  const authorPubkey = "b".repeat(64);
  const [item] = buildTranscript([
    sessionUpdate(25, {
      sessionUpdate: "user_message_chunk",
      messageId: "user-chunk-1",
      authorPubkey,
      content: { type: "text", text: "please keep this visible" },
    }),
  ]).filter((candidate) => candidate.type === "message");

  assert.equal(item.role, "user");
  assert.equal(item.authorPubkey, authorPubkey);
  assert.equal(item.messageId, null);
});

test("buildTranscript preserves real event ids on user message chunks", () => {
  const authorPubkey = "b".repeat(64);
  const messageId = "d".repeat(64);
  const [item] = buildTranscript([
    sessionUpdate(26, {
      sessionUpdate: "user_message_chunk",
      messageId,
      authorPubkey,
      content: { type: "text", text: "this came from a channel message" },
    }),
  ]).filter((candidate) => candidate.type === "message");

  assert.equal(item.role, "user");
  assert.equal(item.messageId, messageId);
});

test("buildTranscript de-duplicates repeated tool updates into one canonical row", () => {
  const items = toolItems([
    acpToolUpdate(40, {
      sessionUpdate: "tool_call",
      toolCallId: "call-dupe",
      status: "executing",
      title: "shell",
      kind: "shell",
      rawInput: { command: "echo hi" },
    }),
    acpToolUpdate(41, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-dupe",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawOutput: "hi",
    }),
    acpToolUpdate(42, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-dupe",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawOutput: "hi",
    }),
  ]);

  assert.equal(items.length, 1);
  assert.equal(items[0].id, `tool:${baseEvent.channelId}:call-dupe`);
  assert.equal(items[0].status, "completed");
  assert.equal(items[0].result, "hi");
});

test("buildTranscript keeps a completed tool terminal when a late executing call arrives", () => {
  const [item] = toolItems([
    acpToolUpdate(50, {
      sessionUpdate: "tool_call_update",
      toolCallId: "call-regression",
      status: "completed",
      title: "shell",
      kind: "shell",
      rawOutput: "done",
    }),
    acpToolUpdate(51, {
      sessionUpdate: "tool_call",
      toolCallId: "call-regression",
      status: "executing",
      title: "shell",
      kind: "shell",
      rawInput: { command: "echo done" },
    }),
  ]);

  assert.equal(item.status, "completed");
  assert.equal(item.completedAt, baseEvent.timestamp);
  assert.deepEqual(item.args, { command: "echo done" });
  assert.equal(item.result, "done");
});

test("buildTranscript rebuilds out-of-order tool frames as one canonical row with retained ids", () => {
  const [item] = toolItems([
    sessionUpdate(
      60,
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "call-out-of-order",
        status: "completed",
        title: "read_file",
        kind: "read_file",
        rawOutput: "file contents",
      },
      {
        channelId: "22222222-2222-2222-2222-222222222222",
        sessionId: "sess-2",
        turnId: "turn-2",
        timestamp: "2026-06-18T00:00:05Z",
      },
    ),
    sessionUpdate(
      61,
      {
        sessionUpdate: "tool_call",
        toolCallId: "call-out-of-order",
        status: "executing",
        title: "read_file",
        kind: "read_file",
        rawInput: { path: "AGENTS.md" },
      },
      {
        channelId: "22222222-2222-2222-2222-222222222222",
        sessionId: "sess-2",
        turnId: "turn-2",
        timestamp: "2026-06-18T00:00:04Z",
      },
    ),
  ]);

  assert.equal(
    item.id,
    "tool:22222222-2222-2222-2222-222222222222:call-out-of-order",
  );
  assert.equal(item.status, "completed");
  assert.deepEqual(item.args, { path: "AGENTS.md" });
  assert.equal(item.channelId, "22222222-2222-2222-2222-222222222222");
  assert.equal(item.turnId, "turn-2");
  assert.equal(item.sessionId, "sess-2");
});

test("buildTranscript coalesces assistant chunks until the message is sealed", () => {
  const messages = buildTranscript([
    assistantChunk(70, "msg-1", "Hello "),
    assistantChunk(71, "msg-1", "world"),
  ]).filter((item) => item.type === "message" && item.role === "assistant");

  assert.equal(messages.length, 1);
  assert.equal(messages[0].text, "Hello world");
  assert.equal(messages[0].id, `assistant:${baseEvent.channelId}:msg-1`);
});

test("buildTranscript starts a continuation for same-message chunks after sealing", () => {
  const messages = buildTranscript([
    assistantChunk(80, "msg-2", "First"),
    acpToolUpdate(81, {
      sessionUpdate: "tool_call",
      toolCallId: "call-seal",
      status: "executing",
      title: "shell",
      kind: "shell",
    }),
    assistantChunk(82, "msg-2", "Second"),
  ]).filter((item) => item.type === "message" && item.role === "assistant");

  assert.equal(messages.length, 2);
  assert.equal(messages[0].text, "First");
  assert.equal(messages[1].text, "Second");
  assert.match(messages[1].id, /:c\d+$/);
});

test("buildTranscript preserves channel, turn, and session ids through message updates", () => {
  const [message] = buildTranscript([
    assistantChunk(90, "msg-identity", "One ", {
      channelId: "33333333-3333-3333-3333-333333333333",
      sessionId: "sess-identity",
      turnId: "turn-identity",
    }),
    assistantChunk(91, "msg-identity", "Two", {
      channelId: "33333333-3333-3333-3333-333333333333",
      sessionId: null,
      turnId: null,
    }),
  ]).filter((item) => item.type === "message" && item.role === "assistant");

  assert.equal(message.text, "One Two");
  assert.equal(message.channelId, "33333333-3333-3333-3333-333333333333");
  assert.equal(message.turnId, "turn-identity");
  assert.equal(message.sessionId, "sess-identity");
});

test("buildTranscript promotes ACP plan updates to first-class plan items", () => {
  const items = buildTranscript([
    sessionUpdate(90, {
      sessionUpdate: "plan",
      content: { type: "text", text: "- [ ] Build registry" },
    }),
  ]);

  assert.equal(items.length, 1);
  assert.equal(items[0].type, "plan");
  assert.equal(items[0].renderClass, "plan");
  assert.match(items[0].text, /Build registry/);
});

test("buildTranscript stores first-class render class descriptors for tool items", () => {
  const [item] = toolItems([
    acpToolUpdate(91, {
      sessionUpdate: "tool_call",
      toolCallId: "call-edit",
      status: "completed",
      title: "str_replace",
      kind: "str_replace",
      rawInput: { path: "src/app.ts" },
    }),
  ]);

  assert.equal(item.renderClass, "file-edit");
  assert.equal(item.descriptor.label, "Edited file");
  assert.equal(item.descriptor.preview, "src/app.ts");
});
