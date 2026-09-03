import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveInboxCompletionSelection,
  resolveInboxFilterSelection,
} from "./inboxSelection.ts";

const items = [
  { conversationId: "first-conversation", id: "first-event" },
  { conversationId: "second-conversation", id: "second-event" },
];

test("filter selection preserves a conversation that remains visible", () => {
  assert.deepEqual(
    resolveInboxFilterSelection({
      isNarrow: false,
      items,
      selectedConversationId: "second-conversation",
    }),
    { autoSelectedEventId: null, preserveSelection: true },
  );
});

test("wide filter selection immediately selects the first valid row", () => {
  assert.deepEqual(
    resolveInboxFilterSelection({
      isNarrow: false,
      items,
      selectedConversationId: "filtered-out-conversation",
    }),
    { autoSelectedEventId: "first-event", preserveSelection: false },
  );
});

test("narrow filter selection returns to the list when selection is invalid", () => {
  assert.deepEqual(
    resolveInboxFilterSelection({
      isNarrow: true,
      items,
      selectedConversationId: "filtered-out-conversation",
    }),
    { autoSelectedEventId: null, preserveSelection: false },
  );
});

test("empty filter selection clears detail at every width", () => {
  assert.deepEqual(
    resolveInboxFilterSelection({
      isNarrow: false,
      items: [],
      selectedConversationId: "filtered-out-conversation",
    }),
    { autoSelectedEventId: null, preserveSelection: false },
  );
});

test("wide explicit completion selects the next Inbox conversation", () => {
  assert.equal(
    resolveInboxCompletionSelection({
      completedConversationId: "first-conversation",
      isNarrow: false,
      items,
    }),
    "second-event",
  );
});

test("wide explicit completion falls back to the previous final row", () => {
  assert.equal(
    resolveInboxCompletionSelection({
      completedConversationId: "second-conversation",
      isNarrow: false,
      items,
    }),
    "first-event",
  );
});

test("narrow explicit completion returns to the Inbox list", () => {
  assert.equal(
    resolveInboxCompletionSelection({
      completedConversationId: "first-conversation",
      isNarrow: true,
      items,
    }),
    null,
  );
});
