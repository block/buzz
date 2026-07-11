import assert from "node:assert/strict";
import test from "node:test";

import { buildTranscript } from "./agentSessionTranscript.ts";
import {
  buildTranscriptDisplayBlocks,
  flattenDisplayBlocks,
} from "./agentSessionTranscriptGrouping.ts";
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

test("buildTranscript surfaces session/request_permission as a permission lifecycle item", () => {
  const transcript = buildTranscript([
    {
      seq: 1,
      timestamp: "2026-06-30T09:00:00.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        method: "session/request_permission",
        params: {
          toolCallId: "tool-1",
          title: "Confirm force-with-lease push to block/buzz.",
          options: [
            { optionId: "allow_once", kind: "allow_once", name: "Allow" },
            { optionId: "reject_once", kind: "reject_once", name: "Reject" },
          ],
        },
      },
    },
  ]);

  assert.equal(transcript.length, 1);
  assert.equal(transcript[0].type, "lifecycle");
  assert.equal(transcript[0].renderClass, "permission");
  assert.equal(transcript[0].title, "Permission requested");
  assert.match(transcript[0].text, /Confirm force-with-lease push/);
});

test("buildTranscript stamps completedAt when a terminal tool update is inserted first", () => {
  const transcript = buildTranscript([
    {
      seq: 1,
      timestamp: "2026-06-30T09:00:00.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "session-1",
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: "tool-1",
            toolName: "dev__shell",
            status: "completed",
            rawInput: { command: "echo hi" },
            content: [{ type: "text", text: "hi" }],
          },
        },
      },
    },
  ]);

  assert.equal(transcript[0].type, "tool");
  assert.equal(transcript[0].completedAt, "2026-06-30T09:00:00.000Z");
});

test("buildTranscript preserves permission, free-form status, and raw rail render classes", () => {
  const transcript = buildTranscript([
    {
      ...baseEvent,
      seq: 1,
      kind: "acp_read",
      payload: {
        method: "session/request_permission",
        params: {
          title: "Confirm force-with-lease push",
          toolCallId: "tool-push",
          options: [
            { optionId: "allow_once", kind: "allow_once", name: "Allow" },
            { optionId: "reject_once", kind: "reject_once", name: "Reject" },
          ],
        },
      },
    },
    {
      ...baseEvent,
      seq: 2,
      kind: "acp_read",
      payload: {
        type: "observer_connected",
        title: "Observer connected",
        text: "ACP stream attached",
      },
    },
    {
      ...baseEvent,
      seq: 3,
      kind: "raw_json_rpc",
      payload: {
        method: "workspace/diagnostic",
        params: { ok: true },
      },
    },
  ]);

  const permissionItem = transcript.find(
    (item) =>
      item.id.startsWith("permission:") && item.renderClass === "permission",
  );
  assert.ok(
    permissionItem,
    "permission request should flow through the reducer",
  );
  assert.doesNotMatch(
    permissionItem.text,
    /^Permission requested\b/,
    "permission detail should not duplicate the row title",
  );
  assert.ok(
    transcript.some(
      (item) =>
        item.renderClass === "status" && item.title === "Observer connected",
    ),
    "free-form status fixture should flow through the reducer",
  );
  assert.ok(
    transcript.some(
      (item) => item.type === "metadata" && item.renderClass === "raw-rail",
    ),
    "raw_json_rpc fixture should flow through the reducer",
  );
});

test("buildTranscript separates repeated lifecycle text", () => {
  const events = [
    {
      seq: 1,
      timestamp: "2026-06-30T09:00:00.000Z",
      kind: "turn_error",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: { outcome: "recovered", error: "first" },
    },
    {
      seq: 2,
      timestamp: "2026-06-30T09:00:01.000Z",
      kind: "turn_error",
      agentIndex: 0,
      channelId: "channel-1",
      sessionId: "session-1",
      turnId: "turn-1",
      payload: { outcome: "recovered", error: "second" },
    },
  ];

  const [item] = buildTranscript(events);
  assert.equal(item.type, "lifecycle");
  assert.equal(item.text, "recovered: first\nrecovered: second");
});

// --- permission outcome (Fix #3) ---

function makePermissionRequest(seq, requestId, turnId = "turn-1") {
  return {
    seq,
    timestamp: "2026-06-30T10:00:00.000Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId,
    payload: {
      jsonrpc: "2.0",
      id: requestId,
      method: "session/request_permission",
      params: {
        title: "Confirm push",
        toolCallId: "tool-1",
        options: [
          { optionId: "allow_once", kind: "allow_once", name: "Allow" },
          { optionId: "reject_once", kind: "reject_once", name: "Reject" },
        ],
      },
    },
  };
}

function makePermissionResponse(seq, requestId, outcome, optionId = null) {
  const resultOutcome =
    outcome === "selected" ? { outcome: "selected", optionId } : { outcome };
  return {
    seq,
    timestamp: "2026-06-30T10:00:01.000Z",
    kind: "acp_write",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: "turn-1",
    payload: {
      jsonrpc: "2.0",
      id: requestId,
      result: { outcome: resultOutcome },
    },
  };
}

test("buildTranscript appends Approved outcome when allow_once is selected", () => {
  const transcript = buildTranscript([
    makePermissionRequest(1, "req-1"),
    makePermissionResponse(2, "req-1", "selected", "allow_once"),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "permission");
  assert.equal(item.outcome, "Approved (allow_once)");
  assert.doesNotMatch(item.text ?? "", /Approved/);
});

test("buildTranscript appends Denied outcome when reject_once is selected", () => {
  const transcript = buildTranscript([
    makePermissionRequest(1, "req-2"),
    makePermissionResponse(2, "req-2", "selected", "reject_once"),
  ]);

  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.outcome, "Denied (reject_once)");
  assert.doesNotMatch(item.text ?? "", /Denied/);
});

test("buildTranscript appends Cancelled outcome on cancelled response", () => {
  const transcript = buildTranscript([
    makePermissionRequest(1, "req-3"),
    makePermissionResponse(2, "req-3", "cancelled"),
  ]);

  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.outcome, "Cancelled");
  assert.doesNotMatch(item.text ?? "", /Cancelled/);
});

test("buildTranscript no-ops on a permission response with an unmatched id", () => {
  const transcript = buildTranscript([
    makePermissionRequest(1, "req-4"),
    makePermissionResponse(2, "req-WRONG", "selected", "allow_once"),
  ]);

  // The permission item exists but has no outcome appended — the mismatched
  // response id must not crash or attach to the wrong item.
  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "permission");
  assert.equal(item.outcome, undefined);
  assert.doesNotMatch(item.text ?? "", /Approved/);
  assert.doesNotMatch(item.text ?? "", /Denied/);
});

test("buildTranscript appends Approved outcome for a numeric JSON-RPC id (selected allow_once)", () => {
  // JSON-RPC 2.0 allows numeric ids; the ACP runtime preserves them as
  // serde_json::Value. asString() drops numbers, so this exercises the
  // jsonRpcId() helper path that handles finite-number ids.
  const transcript = buildTranscript([
    makePermissionRequest(1, 42),
    makePermissionResponse(2, 42, "selected", "allow_once"),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "permission");
  assert.equal(item.outcome, "Approved (allow_once)");
  assert.doesNotMatch(item.text ?? "", /Approved/);
});

test("buildTranscript appends Cancelled outcome for a numeric JSON-RPC id (cancelled)", () => {
  const transcript = buildTranscript([
    makePermissionRequest(1, 99),
    makePermissionResponse(2, 99, "cancelled"),
  ]);

  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.outcome, "Cancelled");
  assert.doesNotMatch(item.text ?? "", /Cancelled/);
});

test('buildTranscript does not collide between numeric id 1 and string id "1"', () => {
  // JSON.stringify produces "1" for number 1 and "\"1\"" for string "1",
  // so requests with these two different id types must NOT cross-attach.
  const transcriptNumeric = buildTranscript([
    makePermissionRequest(1, 1),
    makePermissionResponse(2, 1, "selected", "allow_once"),
  ]);
  const transcriptString = buildTranscript([
    makePermissionRequest(1, "1"),
    makePermissionResponse(2, "1", "selected", "reject_once"),
  ]);

  assert.equal(transcriptNumeric[0].outcome, "Approved (allow_once)");
  assert.equal(transcriptString[0].outcome, "Denied (reject_once)");
});

// ─── observer parity: new session/update classifier cases ────────────────────

test("buildTranscript renders current_mode_update as a lifecycle status line", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "current_mode_update",
      currentModeId: "plan",
    }),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "status");
  assert.equal(item.title, "Mode");
  assert.equal(item.text, "plan");
  assert.equal(item.acpSource, "current_mode_update");
});

test("buildTranscript suppresses current_mode_update when currentModeId is missing", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, { sessionUpdate: "current_mode_update" }),
  ]);
  assert.equal(transcript.length, 0);
});

test("buildTranscript renders usage_update as a lifecycle status line with tokens", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "usage_update",
      used: 1500,
      size: 8192,
    }),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "status");
  assert.equal(item.title, "Usage");
  assert.equal(item.text, "Tokens: 1500/8192");
  assert.equal(item.acpSource, "usage_update");
});

test("buildTranscript renders usage_update with cost when present", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "usage_update",
      used: 800,
      size: 4096,
      cost: { amount: 0.0025, currency: "USD" },
    }),
  ]);

  assert.equal(transcript.length, 1);
  assert.equal(transcript[0].text, "Tokens: 800/4096 ($0.0025 USD)");
});

test("buildTranscript coalesces usage_update to latest-per-turn (replace, not append)", () => {
  // Three usage frames in the same turn must produce exactly ONE lifecycle item
  // showing the LAST value — not an accumulation of all three.
  const transcript = buildTranscript([
    acpToolUpdate(1, { sessionUpdate: "usage_update", used: 100, size: 8192 }),
    acpToolUpdate(2, { sessionUpdate: "usage_update", used: 300, size: 8192 }),
    acpToolUpdate(3, { sessionUpdate: "usage_update", used: 600, size: 8192 }),
  ]);

  const usageItems = transcript.filter((i) => i.acpSource === "usage_update");
  assert.equal(usageItems.length, 1, "must coalesce to one item");
  assert.equal(usageItems[0].text, "Tokens: 600/8192");
});

test("buildTranscript suppresses usage_update when used or size is missing", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, { sessionUpdate: "usage_update", used: 100 }), // size absent
    acpToolUpdate(2, { sessionUpdate: "usage_update", size: 8192 }), // used absent
  ]);
  assert.equal(transcript.length, 0);
});

test("buildTranscript renders available_commands_update as a lifecycle status line", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "available_commands_update",
      availableCommands: [
        { name: "create_plan", description: "Create a plan" },
        { name: "research_codebase", description: "Research the codebase" },
      ],
    }),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "status");
  assert.equal(item.title, "Commands");
  assert.equal(item.text, "Commands available: 2");
  assert.equal(item.acpSource, "available_commands_update");
});

test("buildTranscript renders available_commands_update with zero commands", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "available_commands_update",
      availableCommands: [],
    }),
  ]);

  assert.equal(transcript.length, 1);
  assert.equal(transcript[0].text, "Commands available: 0");
});

test("buildTranscript renders config_option_update as a lifecycle status line", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "config_option_update",
      configOptions: [
        { id: "model", name: "Model", type: "select", currentValue: "gpt-4o" },
        { id: "mode", name: "Mode", type: "select", currentValue: "auto" },
      ],
    }),
  ]);

  assert.equal(transcript.length, 1);
  const item = transcript[0];
  assert.equal(item.type, "lifecycle");
  assert.equal(item.renderClass, "status");
  assert.equal(item.title, "Config");
  assert.equal(item.text, "Model = gpt-4o, Mode = auto");
  assert.equal(item.acpSource, "config_option_update");
});

test("buildTranscript suppresses config_option_update when configOptions is empty", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, {
      sessionUpdate: "config_option_update",
      configOptions: [],
    }),
  ]);
  assert.equal(transcript.length, 0);
});

test("buildTranscript does not render keepalive (stays in else-dropped bucket)", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, { sessionUpdate: "keepalive" }),
  ]);
  assert.equal(transcript.length, 0);
});

test("buildTranscript does not render unknown session/update types (firehose safety net)", () => {
  const transcript = buildTranscript([
    acpToolUpdate(1, { sessionUpdate: "some_future_type", value: 42 }),
  ]);
  assert.equal(transcript.length, 0);
});

// --- system-prompt ordering ---

test("observer feed renders system-prompt before prompt-context in display order (first turn, realistic pool.rs sequence)", () => {
  // Reproduces the real ordering bug: pool.rs emits turn_started BEFORE session/new,
  // so turn_started creates the turn bucket first. Without the injection mechanism,
  // displayOrder becomes [turn(turn-1), single(system-prompt)] and System prompt
  // renders after the entire turn block — after Prompt context.
  // The fix: system-prompt items (acpSource "session/new") are held and injected
  // into the prompt segment of the first turn that follows them in stream order.
  const events = [
    {
      seq: 1,
      timestamp: "2026-07-01T10:00:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: null,
      turnId: "turn-1",
      payload: { source: "channel", triggeringEventIds: [] },
    },
    {
      seq: 2,
      timestamp: "2026-07-01T10:00:00.100Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: null,
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 1,
        method: "session/new",
        params: {
          systemPrompt:
            "[Base]\nYou are a helpful assistant.\n\n[System]\nObserver Agent.",
        },
      },
    },
    {
      seq: 3,
      timestamp: "2026-07-01T10:00:00.200Z",
      kind: "session_resolved",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: { sessionId: "sess-1", isNewSession: true },
    },
    {
      seq: 4,
      timestamp: "2026-07-01T10:00:01.000Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 2,
        method: "session/prompt",
        params: {
          sessionId: "sess-1",
          prompt: [
            {
              type: "text",
              text: `[Buzz event: @mention]\nEvent ID: ${"a".repeat(64)}\nFrom: x (hex: ${"b".repeat(64)})\nContent: hello`,
            },
            { type: "text", text: "[Thread context]\nPrior messages here." },
          ],
        },
      },
    },
  ];

  // Route through the display layer — this is the layer that contained the bug.
  const rawItems = buildTranscript(events);
  const displayItems = flattenDisplayBlocks(
    buildTranscriptDisplayBlocks(rawItems),
  );
  const systemPromptIdx = displayItems.findIndex(
    (i) => i.title === "System prompt",
  );
  const promptContextIdx = displayItems.findIndex(
    (i) => i.title === "Prompt context",
  );
  assert.ok(systemPromptIdx !== -1, "expected a System prompt item");
  assert.ok(promptContextIdx !== -1, "expected a Prompt context item");
  assert.ok(
    systemPromptIdx < promptContextIdx,
    `expected System prompt (idx ${systemPromptIdx}) before Prompt context (idx ${promptContextIdx}) in display order`,
  );
  // Also verify the fix input: system-prompt item must have turnId=null so the
  // display grouper treats it as a standalone entry, not a turn-bucket item.
  const systemPromptRawIdx = rawItems.findIndex(
    (i) => i.title === "System prompt",
  );
  assert.equal(
    rawItems[systemPromptRawIdx].turnId ?? null,
    null,
    "system-prompt item must have turnId=null to avoid turn-bucket grouping",
  );
});

test("observer feed renders system-prompt before prompt-context in display order (multi-turn)", () => {
  // On subsequent turns, session/new does not re-fire. The system-prompt item
  // injected into turn-1 must not re-appear in turn-2's prompt segment.
  const events = [
    {
      seq: 1,
      timestamp: "2026-07-01T10:00:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: null,
      turnId: "turn-1",
      payload: { source: "channel", triggeringEventIds: [] },
    },
    {
      seq: 2,
      timestamp: "2026-07-01T10:00:00.100Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: null,
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 1,
        method: "session/new",
        params: {
          systemPrompt: "[Base]\nYou are helpful.\n\n[System]\nObserver.",
        },
      },
    },
    {
      seq: 3,
      timestamp: "2026-07-01T10:00:00.200Z",
      kind: "session_resolved",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: { sessionId: "sess-1", isNewSession: true },
    },
    {
      seq: 4,
      timestamp: "2026-07-01T10:00:01.000Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 2,
        method: "session/prompt",
        params: {
          sessionId: "sess-1",
          prompt: [
            {
              type: "text",
              text: `[Buzz event: @mention]\nEvent ID: ${"a".repeat(64)}\nFrom: x (hex: ${"b".repeat(64)})\nContent: turn 1`,
            },
            { type: "text", text: "[Thread context]\nEmpty." },
          ],
        },
      },
    },
    {
      seq: 5,
      timestamp: "2026-07-01T10:05:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-2",
      payload: { source: "channel", triggeringEventIds: [] },
    },
    {
      seq: 6,
      timestamp: "2026-07-01T10:05:01.000Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-2",
      payload: {
        jsonrpc: "2.0",
        id: 3,
        method: "session/prompt",
        params: {
          sessionId: "sess-1",
          prompt: [
            {
              type: "text",
              text: `[Buzz event: @mention]\nEvent ID: ${"c".repeat(64)}\nFrom: x (hex: ${"d".repeat(64)})\nContent: turn 2`,
            },
            { type: "text", text: "[Thread context]\nOne prior message." },
          ],
        },
      },
    },
  ];

  const rawItems = buildTranscript(events);
  const displayItems = flattenDisplayBlocks(
    buildTranscriptDisplayBlocks(rawItems),
  );
  const systemPromptIdx = displayItems.findIndex(
    (i) => i.title === "System prompt",
  );
  // Both turns produce a Prompt context — grab the first one (turn-1).
  const firstPromptContextIdx = displayItems.findIndex(
    (i) => i.title === "Prompt context",
  );
  assert.ok(systemPromptIdx !== -1, "expected a System prompt item");
  assert.ok(
    firstPromptContextIdx !== -1,
    "expected at least one Prompt context item",
  );
  assert.ok(
    systemPromptIdx < firstPromptContextIdx,
    `expected System prompt (idx ${systemPromptIdx}) before first Prompt context (idx ${firstPromptContextIdx}) in display order`,
  );
  const systemPromptRawIdx = rawItems.findIndex(
    (i) => i.title === "System prompt",
  );
  assert.equal(
    rawItems[systemPromptRawIdx].turnId ?? null,
    null,
    "system-prompt item must have turnId=null",
  );
});

test("steer ingress bundles its prompt context into the steer prompt segment, not a standalone row", () => {
  // The stray "Prompt context · 1 section" row was leaking from the goose
  // session/steer path: it upserted metadata without an acpSource, so display
  // grouping never consumed it into a prompt bundle. It must now ride behind
  // the steer message bubble's checks-icon dialog like session/prompt context.
  const events = [
    {
      seq: 1,
      timestamp: "2026-07-01T10:00:00.000Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: "ch-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 5,
        method: "_goose/unstable/session/steer",
        params: {
          sessionId: "sess-1",
          prompt: [
            {
              type: "text",
              text: `[Buzz event: @mention]\nEvent ID: ${"e".repeat(64)}\nFrom: x (hex: ${"f".repeat(64)})\nContent: steer me`,
            },
            { type: "text", text: "[Thread context]\nPrior messages here." },
          ],
        },
      },
    },
  ];

  const rawItems = buildTranscript(events);
  const steerMessage = rawItems.find((item) => item.type === "message");
  const steerContext = rawItems.find((item) => item.type === "metadata");
  assert.equal(steerMessage?.acpSource, "session/steer:user");
  assert.equal(steerContext?.acpSource, "session/steer:context");

  const [block] = buildTranscriptDisplayBlocks(rawItems);
  assert.equal(block.kind, "turn");
  const promptSegment = block.segments.find(
    (segment) => segment.kind === "prompt",
  );
  assert.ok(promptSegment, "expected steer message to render a prompt bundle");
  assert.equal(promptSegment.context?.id, steerContext.id);
  assert.ok(
    !block.segments.some(
      (segment) => segment.kind === "item" && segment.item.type === "metadata",
    ),
    "steer context must not leak as a standalone metadata row",
  );
});

// --- session/prompt late delivery (live subscription timing race) ---

test("buildTranscript correctly renders prompt segment when session/prompt arrives after status lifecycle events", () => {
  // Simulates the live-subscription timing race: status events (commands, mode,
  // usage) arrive first because the desktop subscribed slightly after turn start,
  // then session/prompt arrives later (e.g. via reconnect replay or archive
  // backfill). buildTranscript is called in out-of-order sequence order but
  // processTranscriptEvent handles insertion — the full rebuild path in
  // appendAgentEvent (slow path for out-of-order) re-processes events sorted by
  // timestamp+seq, so the prompt segment must appear.
  const TURN = "turn-oot";
  const SESS = "sess-oot";
  const CH = "22222222-2222-2222-2222-222222222222";
  const AUTHOR_HEX = "b".repeat(64);
  const EVENT_HEX = "d".repeat(64);

  const makeEvent = (seq, kind, timestamp, payload) => ({
    seq,
    kind,
    timestamp,
    agentIndex: 0,
    channelId: CH,
    sessionId: SESS,
    turnId: TURN,
    payload,
  });

  // Status events arrive first (lower seq but same timestamp as prompt)
  const commandsEvent = makeEvent(2, "acp_read", "2026-06-18T00:01:01Z", {
    method: "session/update",
    params: {
      sessionId: SESS,
      update: {
        sessionUpdate: "available_commands_update",
        availableCommands: ["cmd1", "cmd2"],
      },
    },
  });

  const modeEvent = makeEvent(3, "acp_read", "2026-06-18T00:01:01Z", {
    method: "session/update",
    params: {
      sessionId: SESS,
      update: { sessionUpdate: "current_mode_update", currentModeId: "code" },
    },
  });

  // session/prompt has the lowest seq — it was published first but arrived last
  const promptEvent = makeEvent(1, "acp_write", "2026-06-18T00:01:00Z", {
    method: "session/prompt",
    params: {
      sessionId: SESS,
      prompt: [
        {
          type: "text",
          text: `[Buzz event: @mention]\nEvent ID: ${EVENT_HEX.toUpperCase()}\nFrom: Alice (hex: ${AUTHOR_HEX})\nContent: please help`,
        },
        { type: "text", text: "[Context]\nScope: thread" },
      ],
    },
  });

  // Deliver events out of order: status first, then prompt
  const items = buildTranscript([commandsEvent, modeEvent, promptEvent]);

  const userMsg = items.find((i) => i.type === "message" && i.role === "user");
  assert.ok(
    userMsg,
    "user message item must be present even when session/prompt arrives after status events",
  );
  assert.equal(
    userMsg.text,
    "please help",
    "user message text extracted from session/prompt Content: line",
  );

  const blocks = buildTranscriptDisplayBlocks(items);
  const turnBlock = blocks.find((b) => b.kind === "turn");
  assert.ok(turnBlock, "expected a turn block");
  const promptSegment = turnBlock.segments.find((s) => s.kind === "prompt");
  assert.ok(
    promptSegment,
    "prompt segment must be present when session/prompt arrives out-of-order",
  );
  assert.equal(
    promptSegment.user.text,
    "please help",
    "prompt segment carries the correct user text",
  );
});

// --- session-boundary ordering: restart scenario end-to-end ─────────────────

test("buildTranscript restart sequence: system-prompt renders after session-boundary, not before", () => {
  // Full two-session restart sequence routed through processTranscriptEvent.
  // This is the production scenario that the grouping-only fix missed:
  // upsertMetadata's replaceItem kept the system-prompt item at its first-session
  // array position (before any sess-1 activity), so splitIntoSessionRuns saw it
  // first (currentRun === null) and placed it in run sess-1 — above the boundary.
  //
  // Fix requires BOTH:
  //   1. Normalizer: reposition system-prompt to the stream tail on re-fire
  //      (removeItem + pushItem with new timestamp) so it arrives at the restart
  //      event position in stream order.
  //   2. Grouping: splitIntoSessionRuns pending-buffer re-anchors the
  //      stale-stamped tail item into the new session run's head.
  //
  // Final display order must be: boundary < system-prompt < user-prompt < sess-2 activity.
  // The session/prompt:user event mirrors production (a restart is triggered by
  // a user @mention; the screenshot that surfaced the bug shows exactly this).
  const CH = "33333333-3333-3333-3333-333333333333";
  const AUTHOR_HEX = "c".repeat(64);
  const USER_EVENT_HEX = "e".repeat(64);

  const sess1Events = [
    // sess-1 turn_started
    {
      seq: 1,
      timestamp: "2026-07-01T10:00:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: CH,
      sessionId: null,
      turnId: "turn-1",
      payload: { source: "channel", triggeringEventIds: [] },
    },
    // sess-1 session/new (first fire — pushes system-prompt to the stream)
    {
      seq: 2,
      timestamp: "2026-07-01T10:00:00.100Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: CH,
      sessionId: null,
      turnId: "turn-1",
      payload: {
        jsonrpc: "2.0",
        id: 1,
        method: "session/new",
        params: {
          systemPrompt:
            "[Base]\nYou are a helpful assistant.\n\n[System]\nObserver.",
        },
      },
    },
    // sess-1 resolves
    {
      seq: 3,
      timestamp: "2026-07-01T10:00:00.200Z",
      kind: "session_resolved",
      agentIndex: 0,
      channelId: CH,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: { sessionId: "sess-1", isNewSession: true },
    },
    // sess-1 activity
    {
      seq: 4,
      timestamp: "2026-07-01T10:00:01.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: CH,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {
        method: "session/update",
        params: {
          sessionId: "sess-1",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "call-1",
            status: "completed",
            title: "shell",
            kind: "shell",
            rawInput: { command: "echo hello" },
            content: { type: "text", text: "hello" },
          },
        },
      },
    },
  ];

  const restartEvents = [
    // Restart: turn_started with null sessionId
    {
      seq: 5,
      timestamp: "2026-07-01T11:00:00.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: CH,
      sessionId: null,
      turnId: "turn-2",
      payload: { source: "channel", triggeringEventIds: [] },
    },
    // session/new re-fires for the same channel (restart signal):
    // upsertMetadata must REPOSITION to tail, not replace in-place.
    {
      seq: 6,
      timestamp: "2026-07-01T11:00:00.100Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: CH,
      sessionId: null,
      turnId: "turn-2",
      payload: {
        jsonrpc: "2.0",
        id: 2,
        method: "session/new",
        params: {
          systemPrompt:
            "[Base]\nYou are a helpful assistant.\n\n[System]\nObserver.",
        },
      },
    },
    // New session resolves
    {
      seq: 7,
      timestamp: "2026-07-01T11:00:00.200Z",
      kind: "session_resolved",
      agentIndex: 0,
      channelId: CH,
      sessionId: "sess-2",
      turnId: "turn-2",
      payload: { sessionId: "sess-2", isNewSession: true },
    },
    // sess-2 user @mention prompt (production shape: a restart is triggered by
    // a user message; this is what surfaced the original bug in the screenshot).
    {
      seq: 8,
      timestamp: "2026-07-01T11:00:00.300Z",
      kind: "acp_write",
      agentIndex: 0,
      channelId: CH,
      sessionId: "sess-2",
      turnId: "turn-2",
      payload: {
        jsonrpc: "2.0",
        id: 3,
        method: "session/prompt",
        params: {
          sessionId: "sess-2",
          prompt: [
            {
              type: "text",
              text: `[Buzz event: @mention]\nEvent ID: ${USER_EVENT_HEX.toUpperCase()}\nFrom: Will (hex: ${AUTHOR_HEX})\nContent: @Paul status check? I had to restart`,
            },
          ],
        },
      },
    },
    // sess-2 activity
    {
      seq: 9,
      timestamp: "2026-07-01T11:00:01.000Z",
      kind: "acp_read",
      agentIndex: 0,
      channelId: CH,
      sessionId: "sess-2",
      turnId: "turn-2",
      payload: {
        method: "session/update",
        params: {
          sessionId: "sess-2",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "call-2",
            status: "completed",
            title: "shell",
            kind: "shell",
            rawInput: { command: "echo world" },
            content: { type: "text", text: "world" },
          },
        },
      },
    },
  ];

  const items = buildTranscript([...sess1Events, ...restartEvents]);
  const blocks = buildTranscriptDisplayBlocks(items, "sess-2");

  // (a) Exactly one session-boundary block between the two sessions.
  const boundaryBlocks = blocks.filter((b) => b.kind === "session-boundary");
  assert.equal(
    boundaryBlocks.length,
    1,
    "exactly one session-boundary block for a two-session restart",
  );

  const boundaryIdx = blocks.indexOf(boundaryBlocks[0]);

  // (b) The system-prompt must appear AFTER the boundary — not before it.
  // After consolidation: system-prompt always renders as a standalone single
  // (never inside the prompt bundle), before the turn's user bubble.
  const systemPromptBlockIdx = blocks.findIndex(
    (b) => b.kind === "single" && b.item?.acpSource === "session/new",
  );
  assert.ok(
    systemPromptBlockIdx !== -1,
    "system-prompt item must be present in the output",
  );
  assert.ok(
    boundaryIdx < systemPromptBlockIdx,
    `boundary (idx ${boundaryIdx}) must precede system-prompt (idx ${systemPromptBlockIdx})`,
  );

  // (c) sess-2 activity must appear AFTER both boundary AND system-prompt.
  // This pins the full required order: boundary → system-prompt → activity.
  // The system-prompt standalone single and the tool activity are now in
  // distinct blocks; use flat item indices to assert ordering.
  const flat = flattenDisplayBlocks(blocks);
  const sess2ActivityItem = flat.find(
    (i) => i.type === "tool" && i.sessionId === "sess-2",
  );
  assert.ok(
    sess2ActivityItem,
    "sess-2 tool activity must be present in flattened output",
  );
  const sess2BlockIdx = blocks.findIndex((b) =>
    flattenDisplayBlocks([b]).some(
      (i) => i.type === "tool" && i.sessionId === "sess-2",
    ),
  );
  assert.ok(
    boundaryIdx < sess2BlockIdx,
    `boundary (idx ${boundaryIdx}) must precede sess-2 activity (idx ${sess2BlockIdx})`,
  );
  // system-prompt and tool activity may be in the same turn block (prompt bundle
  // + activity segment). Assert ordering via flat item indices.
  const flatSystemPromptIdx = flat.findIndex(
    (i) => i.acpSource === "session/new",
  );
  const flatSess2ActivityIdx = flat.findIndex(
    (i) => i.type === "tool" && i.sessionId === "sess-2",
  );
  assert.ok(
    flatSystemPromptIdx !== -1,
    "system-prompt must appear in flattened output",
  );
  assert.ok(
    flatSystemPromptIdx < flatSess2ActivityIdx,
    `system-prompt (flat idx ${flatSystemPromptIdx}) must precede sess-2 activity (flat idx ${flatSess2ActivityIdx})`,
  );
});
