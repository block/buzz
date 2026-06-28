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
import { isConversationMessage } from "./ui/AgentConversationScreen.helpers.ts";

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
    const workspaceScope = "wss://relay.example.com";
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

    writePersistedAgentConversations("human", workspaceScope, [conversation]);
    const persisted = readPersistedAgentConversations("human", workspaceScope);
    const otherWorkspace = readPersistedAgentConversations(
      "human",
      "wss://other.example.com",
    );

    assert.equal(persisted.length, 1);
    assert.equal(persisted[0].id, conversation.id);
    assert.equal(persisted[0].channelId, "channel");
    assert.equal(persisted[0].agentReply.id, "agent-reply");
    assert.equal(otherWorkspace.length, 0);
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

test("continued conversation marker with a missing anchor does not hide thread messages", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const reply = message({
    body: "One note before opening.",
    createdAt: 3,
    id: "reply",
  });
  const marker = parseAgentConversationMarker(
    markerEvent({ content: { agentReplyId: "missing-reply" }, createdAt: 4 }),
  );

  const hiddenIds = getHiddenAgentConversationMessageIds(
    [root, reply],
    marker ? [marker] : [],
  );

  assert.deepEqual([...hiddenIds], []);
});

test("continued conversation markers keep later task anchors visible", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const firstAnchor = message({
    body: "I'll look into it.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const hiddenReply = message({
    body: "This should live in the first task.",
    createdAt: 3,
    id: "hidden",
  });
  const secondAnchor = message({
    body: "Let's split this into another task.",
    createdAt: 4,
    id: "second-anchor",
    pubkey: "agent",
  });
  const laterReply = message({
    body: "This should also be hidden.",
    createdAt: 5,
    id: "later",
  });

  const firstMarker = parseAgentConversationMarker(
    markerEvent({ content: { startedAt: 2 }, createdAt: 2 }),
  );
  const secondMarker = parseAgentConversationMarker({
    ...markerEvent({
      content: { agentReplyId: "second-anchor", startedAt: 4 },
      createdAt: 4,
      id: "second-marker",
    }),
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "second-anchor", "", "agent-reply"],
      ["p", "agent"],
      ["title", "Second task"],
    ],
  });

  const hiddenIds = getHiddenAgentConversationMessageIds(
    [root, firstAnchor, hiddenReply, secondAnchor, laterReply],
    [firstMarker, secondMarker].filter(Boolean),
  );

  assert.deepEqual([...hiddenIds], ["hidden", "later"]);
});

test("dedicated conversation view stops at the next task anchor", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const firstAnchor = message({
    body: "I'll look into it.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const firstTaskReply = message({
    body: "This belongs in the first task.",
    createdAt: 3,
    id: "first-task-reply",
  });
  const secondAnchor = message({
    body: "Let's split this into another task.",
    createdAt: 4,
    id: "second-anchor",
    pubkey: "agent",
  });
  const secondTaskReply = message({
    body: "This belongs in the second task.",
    createdAt: 5,
    id: "second-task-reply",
  });
  const messages = [
    root,
    firstAnchor,
    firstTaskReply,
    secondAnchor,
    secondTaskReply,
  ];
  const conversation = buildAgentConversation({
    agentName: "Fizz",
    agentPubkey: "agent",
    agentReply: firstAnchor,
    channel: { id: "channel", name: "general" },
    contextMessages: messages,
    parentMessage: root,
    threadRootMessage: root,
  });
  const firstMarker = parseAgentConversationMarker(
    markerEvent({ content: { startedAt: 2 }, createdAt: 2 }),
  );
  const secondMarker = parseAgentConversationMarker({
    ...markerEvent({
      content: { agentReplyId: "second-anchor", startedAt: 4 },
      createdAt: 4,
      id: "second-marker",
    }),
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "second-anchor", "", "agent-reply"],
      ["p", "agent"],
      ["title", "Second task"],
    ],
  });
  const markers = [firstMarker, secondMarker].filter(Boolean);

  const visibleIds = messages
    .filter((entry) =>
      isConversationMessage(entry, conversation, markers, messages),
    )
    .map((entry) => entry.id);

  assert.deepEqual(visibleIds, ["root", "agent-reply", "first-task-reply"]);
});

test("dedicated conversation view keeps older task descendant replies", () => {
  const root = message({
    body: "Can you look into the data model?",
    createdAt: 1,
    id: "root",
  });
  const firstAnchor = message({
    body: "I'll look into it.",
    createdAt: 2,
    id: "agent-reply",
    pubkey: "agent",
  });
  const firstTaskReply = message({
    body: "This belongs in the first task.",
    createdAt: 3,
    id: "first-task-reply",
  });
  const secondAnchor = message({
    body: "Let's split this into another task.",
    createdAt: 4,
    id: "second-anchor",
    pubkey: "agent",
  });
  const secondTaskReply = message({
    body: "This belongs in the second task.",
    createdAt: 5,
    id: "second-task-reply",
  });
  const firstTaskFollowup = {
    ...message({
      body: "Continuing the first task after task two started.",
      createdAt: 6,
      id: "first-task-followup",
    }),
    parentId: "first-task-reply",
    rootId: "root",
  };
  const plainRootReply = message({
    body: "A plain root reply after task two should not join task one.",
    createdAt: 7,
    id: "plain-root-reply",
  });
  const messages = [
    root,
    firstAnchor,
    firstTaskReply,
    secondAnchor,
    secondTaskReply,
    firstTaskFollowup,
    plainRootReply,
  ];
  const conversation = buildAgentConversation({
    agentName: "Fizz",
    agentPubkey: "agent",
    agentReply: firstAnchor,
    channel: { id: "channel", name: "general" },
    contextMessages: messages,
    parentMessage: root,
    threadRootMessage: root,
  });
  const firstMarker = parseAgentConversationMarker(
    markerEvent({ content: { startedAt: 2 }, createdAt: 2 }),
  );
  const secondMarker = parseAgentConversationMarker({
    ...markerEvent({
      content: { agentReplyId: "second-anchor", startedAt: 4 },
      createdAt: 4,
      id: "second-marker",
    }),
    tags: [
      ["h", "channel"],
      ["e", "root", "", "root"],
      ["e", "second-anchor", "", "agent-reply"],
      ["p", "agent"],
      ["title", "Second task"],
    ],
  });
  const markers = [firstMarker, secondMarker].filter(Boolean);

  const visibleIds = messages
    .filter((entry) =>
      isConversationMessage(entry, conversation, markers, messages),
    )
    .map((entry) => entry.id);

  assert.deepEqual(visibleIds, [
    "root",
    "agent-reply",
    "first-task-reply",
    "first-task-followup",
  ]);
});
