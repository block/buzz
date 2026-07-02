import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChatActivityPlacement,
  shouldHidePersistedAgentMessage,
} from "./chatActivity.ts";

const self = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const agent =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

function message(overrides) {
  return {
    id: overrides.id,
    localKey: overrides.id,
    pubkey: overrides.pubkey ?? self,
    content: overrides.content ?? "",
    created_at: overrides.created_at ?? 1,
    tags: overrides.tags ?? [],
    kind: overrides.kind ?? 45001,
  };
}

test("buildChatActivityPlacement attaches a turn to its source prompt", () => {
  const messages = [
    message({ id: "user-1", content: "Can you check this?" }),
    message({ id: "agent-1", pubkey: agent, content: "Done." }),
  ];
  const placement = buildChatActivityPlacement({
    agentPubkey: agent,
    messages,
    transcript: [
      {
        id: "prompt",
        type: "message",
        renderClass: "message",
        role: "user",
        title: "User",
        text: "Can you check this?",
        timestamp: "2026-07-02T07:00:00.000Z",
        turnId: "turn-1",
        channelId: "chat-1",
        sessionId: "session-1",
        messageId: "user-1",
        acpSource: "session/prompt:user",
        authorPubkey: self,
      },
      {
        id: "tool",
        type: "tool",
        renderClass: "shell",
        descriptor: {
          renderClass: "shell",
          label: "Ran command",
          preview: "pnpm test",
        },
        title: "pnpm test",
        toolName: "shell",
        buzzToolName: null,
        status: "completed",
        args: { command: "pnpm test" },
        result: "ok",
        isError: false,
        timestamp: "2026-07-02T07:00:01.000Z",
        startedAt: "2026-07-02T07:00:01.000Z",
        completedAt: "2026-07-02T07:00:02.000Z",
        turnId: "turn-1",
        channelId: "chat-1",
        sessionId: "session-1",
      },
      {
        id: "assistant",
        type: "message",
        renderClass: "message",
        role: "assistant",
        title: "Assistant",
        text: "Done.",
        timestamp: "2026-07-02T07:00:03.000Z",
        turnId: "turn-1",
        channelId: "chat-1",
        sessionId: "session-1",
      },
    ],
  });

  const attached = placement.blocksByMessageId.get("user-1") ?? [];
  assert.equal(attached.length, 1);
  assert.equal(attached[0].attachedMessageId, "user-1");
  assert.equal(attached[0].suppressPromptMessage, true);
  assert.equal(placement.unplacedBlocks.length, 0);
  assert.equal(placement.totalBlockCount, 1);
});

test("shouldHidePersistedAgentMessage hides only transcript-covered agent text", () => {
  const hiddenAgentMessageIds = new Set(["agent-1"]);

  assert.equal(
    shouldHidePersistedAgentMessage({
      event: message({ id: "agent-1", pubkey: agent, content: "Done." }),
      hiddenAgentMessageIds,
    }),
    true,
  );

  assert.equal(
    shouldHidePersistedAgentMessage({
      event: message({ id: "user-1", pubkey: self, content: "Done." }),
      hiddenAgentMessageIds,
    }),
    false,
  );
});

test("buildChatActivityPlacement hides only the latest matching persisted agent message", () => {
  const messages = [
    message({ id: "agent-old", pubkey: agent, content: "Done." }),
    message({ id: "agent-new", pubkey: agent, content: "Done." }),
  ];

  const placement = buildChatActivityPlacement({
    agentPubkey: agent,
    messages,
    transcript: [
      {
        id: "assistant",
        type: "message",
        renderClass: "message",
        role: "assistant",
        title: "Assistant",
        text: "Done.",
        timestamp: "2026-07-02T07:00:03.000Z",
        turnId: "turn-1",
        channelId: "chat-1",
        sessionId: "session-1",
      },
    ],
  });

  assert.deepEqual([...placement.hiddenAgentMessageIds], ["agent-new"]);
});

test("buildChatActivityPlacement matches persisted agent text after chat cleanup", () => {
  const messages = [
    message({
      id: "agent-emoji",
      pubkey: agent,
      content: "Let's do it! 🚀 What are we trying out?",
    }),
  ];

  const placement = buildChatActivityPlacement({
    agentPubkey: agent,
    messages,
    transcript: [
      {
        id: "assistant",
        type: "message",
        renderClass: "message",
        role: "assistant",
        title: "Assistant",
        text: "Let's do it! What are we trying out?",
        timestamp: "2026-07-02T07:00:03.000Z",
        turnId: "turn-1",
        channelId: "chat-1",
        sessionId: "session-1",
      },
    ],
  });

  assert.deepEqual([...placement.hiddenAgentMessageIds], ["agent-emoji"]);
});

test("buildChatActivityPlacement hides intermediate persisted agent turn messages", () => {
  const messages = [
    message({ id: "user-1", content: "Can you check the menu?" }),
    message({
      id: "agent-progress",
      pubkey: agent,
      content: "Now let me find the agent popover menu component.",
    }),
    message({
      id: "agent-final",
      pubkey: agent,
      content: "The agent popover lives in ProfilePopover.tsx.",
    }),
  ];

  const placement = buildChatActivityPlacement({
    agentPubkey: agent,
    messages,
    transcript: [],
  });

  assert.deepEqual([...placement.hiddenAgentMessageIds], ["agent-progress"]);
});

test("buildChatActivityPlacement keeps separate agent turns visible", () => {
  const messages = [
    message({ id: "user-1", content: "First?" }),
    message({
      id: "agent-first",
      pubkey: agent,
      content: "First answer.",
    }),
    message({ id: "user-2", content: "Second?" }),
    message({
      id: "agent-second",
      pubkey: agent,
      content: "Second answer.",
    }),
  ];

  const placement = buildChatActivityPlacement({
    agentPubkey: agent,
    messages,
    transcript: [],
  });

  assert.deepEqual([...placement.hiddenAgentMessageIds], []);
});
