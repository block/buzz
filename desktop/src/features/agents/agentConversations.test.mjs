import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentConversationMentionPubkeys,
  buildAgentConversation,
  buildAgentConversationRecap,
  buildAgentConversationMarkers,
  deriveAgentConversationTitle,
  getAutoRoutedAgentConversationPubkeys,
  getHiddenAgentConversationMessageIds,
  parseAgentConversationMarker,
  readPersistedAgentConversations,
  writePersistedAgentConversations,
} from "./agentConversations.ts";

function message({ body, createdAt, id, pubkey = "human" }) {
  return {
    author: pubkey === "agent" ? "Fizz" : "Kenny Lopez",
    body,
    createdAt,
    depth: id === "root" ? 0 : 1,
    id,
    parentId: id === "root" ? null : "root",
    pubkey,
    rootId: id === "root" ? null : "root",
    time: "1:00 PM",
  };
}

test("continued conversation title condenses a refined Buzz data thread", () => {
  const root = message({
    body: "Can you tell me about what kind of data we have in the Buzz app?",
    createdAt: 1,
    id: "root",
  });
  const agentReply = message({
    body: "Sure, the app has channel, message, and membership data.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const refinement = message({
    body: "I meant, what data do we have about how the users use the product?",
    createdAt: 3,
    id: "refinement",
  });

  const title = deriveAgentConversationTitle({
    agentPubkey: "agent",
    agentReply,
    contextMessages: [root, agentReply, refinement],
    parentMessage: root,
    threadRootId: root.id,
    threadRootMessage: root,
  });

  assert.deepEqual(title, {
    status: "resolved",
    title: "Data in Buzz app",
  });
});

test("continued conversation auto-routes only a single messageable agent", () => {
  assert.deepEqual(
    getAutoRoutedAgentConversationPubkeys([
      { canMessage: true, pubkey: "agent-one" },
    ]),
    ["agent-one"],
  );

  assert.deepEqual(
    getAutoRoutedAgentConversationPubkeys([
      { canMessage: true, pubkey: "agent-one" },
      { canMessage: true, pubkey: "agent-two" },
    ]),
    [],
  );

  assert.deepEqual(
    getAutoRoutedAgentConversationPubkeys([
      { canMessage: false, pubkey: "agent-one" },
    ]),
    [],
  );
});

test("continued conversation mention routing preserves explicit multi-agent mentions", () => {
  assert.deepEqual(
    buildAgentConversationMentionPubkeys({
      autoRouteAgentPubkeys: [],
      mentionPubkeys: ["agent-one"],
    }),
    ["agent-one"],
  );

  assert.deepEqual(
    buildAgentConversationMentionPubkeys({
      autoRouteAgentPubkeys: ["AGENT-ONE"],
      mentionPubkeys: ["agent-one", "agent-two"],
    }),
    ["AGENT-ONE", "agent-two"],
  );
});

function markerEvent({ content = {}, createdAt = 1, id = "marker" } = {}) {
  return {
    id,
    pubkey: "starter",
    created_at: createdAt,
    kind: 40004,
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "agent-reply", "", "agent-reply"],
      ["p", "agent"],
      ["title", "Data in Buzz app"],
    ],
    content: JSON.stringify({
      version: 1,
      title: "Data in Buzz app",
      titleStatus: "resolved",
      agentName: "Fizz",
      agentPubkey: "agent",
      threadRootId: "root",
      threadRootMessageId: "root",
      parentMessageId: "root",
      agentReplyId: "agent-reply",
      ...content,
    }),
    sig: "sig",
  };
}

function withMockLocalStorage(callback) {
  const originalWindow = globalThis.window;
  const store = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => store.get(key) ?? null,
      setItem: (key, value) => store.set(key, String(value)),
    },
  };

  try {
    callback();
  } finally {
    if (originalWindow === undefined) {
      delete globalThis.window;
    } else {
      globalThis.window = originalWindow;
    }
  }
}

test("continued conversation marker parses summary metadata", () => {
  const marker = parseAgentConversationMarker(
    markerEvent({
      content: {
        summary: "Buzz stores channel, message, and usage data.",
        summaryAuthorName: "Fizz",
        summaryAuthorPubkey: "agent",
        summaryCreatedAt: 12,
      },
    }),
  );

  assert.equal(
    marker?.summary,
    "Buzz stores channel, message, and usage data.",
  );
  assert.equal(marker?.summaryAuthorName, "Fizz");
  assert.equal(marker?.summaryAuthorPubkey, "agent");
  assert.equal(marker?.summaryCreatedAt, 12);
});

test("continued conversations persist across app restarts", () => {
  withMockLocalStorage(() => {
    const root = message({
      body: "Can you look at the Buzz data model?",
      createdAt: 1,
      id: "root",
    });
    const agentReply = message({
      body: "I can look at it.",
      createdAt: 2,
      id: "agent-reply",
      pubkey: "agent",
    });
    const conversation = buildAgentConversation({
      agentName: "Fizz",
      agentPubkey: "agent",
      agentReply,
      channel: { id: "channel", name: "general" },
      contextMessages: [root, agentReply],
      parentMessage: root,
      threadRootMessage: root,
    });

    writePersistedAgentConversations("human", [conversation]);
    const persisted = readPersistedAgentConversations("human");

    assert.equal(persisted.length, 1);
    assert.equal(persisted[0].id, conversation.id);
    assert.equal(persisted[0].channelId, "channel");
    assert.equal(persisted[0].agentReply.id, "agent-reply");
  });
});

test("continued conversation marker summary update replaces earlier marker", () => {
  const markers = buildAgentConversationMarkers([
    markerEvent({
      content: {
        startedAt: 10,
        summary: "Buzz stores channel, message, and usage data.",
        summaryAuthorName: "Fizz",
        summaryAuthorPubkey: "agent",
        summaryCreatedAt: 12,
      },
      createdAt: 2,
      id: "second",
    }),
    markerEvent({ content: { startedAt: 1 }, createdAt: 1, id: "first" }),
  ]);

  assert.equal(markers.length, 1);
  assert.equal(markers[0].eventId, "second");
  assert.equal(
    markers[0].summary,
    "Buzz stores channel, message, and usage data.",
  );
  assert.equal(markers[0].startedAt, 1);
});

test("continued conversation marker keeps recap across title-only updates", () => {
  const markers = buildAgentConversationMarkers([
    markerEvent({
      content: {
        startedAt: 1,
        summary: "Buzz stores channel, message, and usage data.",
        summaryAuthorName: "Fizz",
        summaryAuthorPubkey: "agent",
        summaryCreatedAt: 12,
      },
      createdAt: 2,
      id: "summary",
    }),
    markerEvent({
      content: {
        startedAt: 1,
        title: "Updated Buzz data topic",
      },
      createdAt: 3,
      id: "title-only",
    }),
  ]);

  assert.equal(markers.length, 1);
  assert.equal(markers[0].eventId, "title-only");
  assert.equal(markers[0].title, "Updated Buzz data topic");
  assert.equal(
    markers[0].summary,
    "Buzz stores channel, message, and usage data.",
  );
  assert.equal(markers[0].summaryAuthorName, "Fizz");
});

test("continued conversation recap summarizes full conversation context", () => {
  const root = message({
    body: "Can you tell me about what kind of data we have in the Buzz app?",
    createdAt: 1,
    id: "root",
  });
  const agentReply = message({
    body: "Sure, Buzz stores channel, message, and membership data.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const refinement = message({
    body: "What data do we have about how users use the product?",
    createdAt: 3,
    id: "refinement",
  });
  const finalAnswer = message({
    body: "For usage, Buzz tracks:\n1. Channel participation\n2. Message activity\n3. Thread engagement signals.",
    createdAt: 4,
    id: "final-answer",
    pubkey: "agent",
  });

  const recap = buildAgentConversationRecap({
    agentPubkeys: new Set(["agent"]),
    conversationTitle: "Data in Buzz app",
    messages: [root, agentReply, refinement, finalAnswer],
  });

  assert.match(recap ?? "", /\*\*Original request:\*\*/);
  assert.match(recap ?? "", /Later clarified:/);
  assert.match(recap ?? "", /\*\*Findings:\*\*/);
  assert.match(recap ?? "", /\*\*Outcome:\*\*/);
  assert.match(recap ?? "", /usage/i);
  assert.match(
    recap ?? "",
    /\*\*Outcome:\*\* For usage, Buzz tracks:\n\n1\. Channel participation\n2\. Message activity\n3\. Thread engagement signals/,
  );
  assert.doesNotMatch(recap ?? "", /1\. Channel participation 2\./);
  assert.doesNotMatch(recap ?? "", /^- Topic:/m);
  assert.doesNotMatch(recap ?? "", /Agent response:/);
  assert.doesNotMatch(recap ?? "", /Current state:/);
});

test("continued conversation recap keeps long outcome text", () => {
  const root = message({
    body: "Can you summarize the button patterns in Buzz?",
    createdAt: 1,
    id: "root",
  });
  const longOutcome = `${"Buzz has several button variants and sizing patterns. ".repeat(30)}Final implementation note: keep the full recap visible without truncation.`;
  const agentReply = message({
    body: longOutcome,
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });

  const recap = buildAgentConversationRecap({
    agentPubkeys: new Set(["agent"]),
    messages: [root, agentReply],
  });

  assert.match(
    recap ?? "",
    /Final implementation note: keep the full recap visible without truncation/,
  );
  assert.doesNotMatch(recap ?? "", /\.\.\.$/);
});

test("continued conversation marker hides source-thread messages after its anchor", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const agentReply = message({
    body: "I'll look into it.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const beforeMarker = message({
    body: "One note before opening.",
    createdAt: 3,
    id: "before",
  });
  const afterMarker = message({
    body: "This belongs in the dedicated conversation.",
    createdAt: 5,
    id: "after",
    pubkey: "agent",
  });

  const marker = parseAgentConversationMarker(
    markerEvent({ content: { startedAt: 4 }, createdAt: 4 }),
  );

  const hiddenIds = getHiddenAgentConversationMessageIds(
    [root, agentReply, beforeMarker, afterMarker],
    marker ? [marker] : [],
  );

  assert.deepEqual([...hiddenIds], ["before", "after"]);
});

test("continued conversation marker hides same-second messages after the anchor", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const beforeMarker = message({
    body: "One note before opening.",
    createdAt: 4,
    id: "before",
  });
  const agentReply = message({
    body: "I'll look into it.",
    createdAt: 4,
    id: "agent-reply",
    pubkey: "agent",
  });
  const afterMarker = message({
    body: "Still working through this.",
    createdAt: 4,
    id: "after",
    pubkey: "agent",
  });

  const marker = parseAgentConversationMarker(
    markerEvent({ content: { startedAt: 4 }, createdAt: 4 }),
  );

  const hiddenIds = getHiddenAgentConversationMessageIds(
    [root, beforeMarker, agentReply, afterMarker],
    marker ? [marker] : [],
  );

  assert.deepEqual([...hiddenIds], ["after"]);
});
