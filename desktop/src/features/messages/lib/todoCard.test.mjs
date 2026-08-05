import assert from "node:assert/strict";
import test from "node:test";

import {
  countDoneItems,
  extractTodoCard,
  MAX_TODO_CARD_ITEMS,
  reduceTodoResponses,
  stripTodoCardSentinel,
} from "./todoCard.ts";

const TOM_PUBKEY = "aabbccddeeff0011";
const YASHODA_PUBKEY = "ddeeff00112233aa";
const BYSTANDER_PUBKEY = "112233aabbccddee";
const CARD_EVENT_ID = "cafe0000000000000000000000000000";
const OTHER_CARD_ID = "beef0000000000000000000000000000";

// Helper: build a fenced sentinel body containing the given payload.
function withSentinel(prose, payload) {
  return `${prose}\n\n\`\`\`buzz:todo-card\n${JSON.stringify(payload)}\n\`\`\``;
}

const TWO_ITEM_CARD = {
  v: 1,
  title: "Launch checklist",
  items: [
    { id: "a1", text: "Tom: flip the flag", assignee: TOM_PUBKEY },
    { id: "b2", text: "Yashoda: verify dashboards", assignee: YASHODA_PUBKEY },
  ],
};

const UNASSIGNED_CARD = {
  v: 1,
  items: [{ id: "a1", text: "Anyone: empty the keg" }],
};

// Helper: build a kind:40009 response event.
let eventCounter = 0;
function response({
  pubkey,
  itemId,
  done,
  createdAt,
  cardId = CARD_EVENT_ID,
  id,
  content,
}) {
  eventCounter += 1;
  return {
    id: id ?? `evt${String(eventCounter).padStart(4, "0")}`,
    pubkey,
    created_at: createdAt,
    kind: 40009,
    tags: [
      ["h", "channel-1"],
      ["e", cardId],
      ["item", itemId],
    ],
    content: content ?? JSON.stringify({ done }),
    sig: "",
  };
}

// ── extractTodoCard ───────────────────────────────────────────────────────────

test("extractTodoCard returns null when no sentinel present", () => {
  assert.equal(extractTodoCard("Just a normal message."), null);
});

test("extractTodoCard returns null for empty string", () => {
  assert.equal(extractTodoCard(""), null);
});

test("extractTodoCard parses a valid two-item card", () => {
  assert.deepEqual(
    extractTodoCard(withSentinel("Launch checklist:", TWO_ITEM_CARD)),
    TWO_ITEM_CARD,
  );
});

test("extractTodoCard parses items without assignees", () => {
  assert.deepEqual(
    extractTodoCard(withSentinel("prose", UNASSIGNED_CARD)),
    UNASSIGNED_CARD,
  );
});

test("extractTodoCard returns null for malformed JSON", () => {
  const content = "prose\n\n```buzz:todo-card\n{not json}\n```";
  assert.equal(extractTodoCard(content), null);
});

test("extractTodoCard returns null for unterminated fence", () => {
  const content = `prose\n\n\`\`\`buzz:todo-card\n${JSON.stringify(TWO_ITEM_CARD)}`;
  assert.equal(extractTodoCard(content), null);
});

test("extractTodoCard returns null for wrong version", () => {
  assert.equal(
    extractTodoCard(withSentinel("prose", { ...TWO_ITEM_CARD, v: 2 })),
    null,
  );
});

test("extractTodoCard returns null for empty item list", () => {
  assert.equal(
    extractTodoCard(withSentinel("prose", { v: 1, items: [] })),
    null,
  );
});

test("extractTodoCard returns null above the item cap", () => {
  const items = Array.from({ length: MAX_TODO_CARD_ITEMS + 1 }, (_, i) => ({
    id: `item-${i}`,
    text: `Task ${i}`,
  }));
  assert.equal(extractTodoCard(withSentinel("prose", { v: 1, items })), null);
});

test("extractTodoCard accepts exactly the item cap", () => {
  const items = Array.from({ length: MAX_TODO_CARD_ITEMS }, (_, i) => ({
    id: `item-${i}`,
    text: `Task ${i}`,
  }));
  assert.notEqual(
    extractTodoCard(withSentinel("prose", { v: 1, items })),
    null,
  );
});

test("extractTodoCard returns null for duplicate item ids", () => {
  const payload = {
    v: 1,
    items: [
      { id: "a1", text: "one" },
      { id: "a1", text: "two" },
    ],
  };
  assert.equal(extractTodoCard(withSentinel("prose", payload)), null);
});

test("extractTodoCard returns null for non-string item fields", () => {
  const payload = { v: 1, items: [{ id: 7, text: "one" }] };
  assert.equal(extractTodoCard(withSentinel("prose", payload)), null);
});

test("extractTodoCard returns null for empty assignee", () => {
  const payload = { v: 1, items: [{ id: "a1", text: "one", assignee: "" }] };
  assert.equal(extractTodoCard(withSentinel("prose", payload)), null);
});

// ── stripTodoCardSentinel ─────────────────────────────────────────────────────

test("stripTodoCardSentinel removes the fence and keeps the prose", () => {
  const content = withSentinel("Launch checklist:", TWO_ITEM_CARD);
  assert.equal(stripTodoCardSentinel(content), "Launch checklist:\n");
});

test("stripTodoCardSentinel keeps trailing prose after the fence", () => {
  const content = `${withSentinel("Before.", TWO_ITEM_CARD)}\nAfter.`;
  assert.equal(stripTodoCardSentinel(content), "Before.\n\nAfter.");
});

test("stripTodoCardSentinel is a no-op without a sentinel", () => {
  assert.equal(stripTodoCardSentinel("plain message"), "plain message");
});

// ── reduceTodoResponses ───────────────────────────────────────────────────────

test("no responses → all items pending", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, []);
  assert.deepEqual(state.get("a1"), {
    done: false,
    completedBy: null,
    completedAt: null,
  });
  assert.deepEqual(state.get("b2"), {
    done: false,
    completedBy: null,
    completedAt: null,
  });
  assert.equal(countDoneItems(TWO_ITEM_CARD, state), 0);
});

test("assignee check-off marks the item done with attribution", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: true, createdAt: 100 }),
  ]);
  assert.deepEqual(state.get("a1"), {
    done: true,
    completedBy: TOM_PUBKEY,
    completedAt: 100,
  });
  assert.equal(state.get("b2")?.done, false);
  assert.equal(countDoneItems(TWO_ITEM_CARD, state), 1);
});

test("latest response per pubkey wins — un-check reverses a check", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: true, createdAt: 100 }),
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: false, createdAt: 200 }),
  ]);
  assert.deepEqual(state.get("a1"), {
    done: false,
    completedBy: null,
    completedAt: null,
  });
});

test("out-of-order delivery folds identically", () => {
  const later = response({
    pubkey: TOM_PUBKEY,
    itemId: "a1",
    done: false,
    createdAt: 200,
  });
  const earlier = response({
    pubkey: TOM_PUBKEY,
    itemId: "a1",
    done: true,
    createdAt: 100,
  });
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    later,
    earlier,
  ]);
  assert.equal(state.get("a1")?.done, false);
});

test("same created_at ties break by event id", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: TOM_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 100,
      id: "bbb",
    }),
    response({
      pubkey: TOM_PUBKEY,
      itemId: "a1",
      done: false,
      createdAt: 100,
      id: "aaa",
    }),
  ]);
  // "bbb" sorts after "aaa" → done:true wins.
  assert.equal(state.get("a1")?.done, true);
});

test("non-assignee completion counts with attribution", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: BYSTANDER_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 100,
    }),
  ]);
  assert.deepEqual(state.get("a1"), {
    done: true,
    completedBy: BYSTANDER_PUBKEY,
    completedAt: 100,
  });
});

test("assignee's response overrides a non-assignee responder", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: BYSTANDER_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 200,
    }),
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: false, createdAt: 100 }),
  ]);
  // The assignee has responded (done:false) — their state wins even though a
  // bystander's completion is more recent.
  assert.equal(state.get("a1")?.done, false);
});

test("unassigned item: any responder completes, most recent attributed", () => {
  const state = reduceTodoResponses(UNASSIGNED_CARD, CARD_EVENT_ID, [
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: true, createdAt: 100 }),
    response({
      pubkey: YASHODA_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 200,
    }),
  ]);
  assert.deepEqual(state.get("a1"), {
    done: true,
    completedBy: YASHODA_PUBKEY,
    completedAt: 200,
  });
});

test("unassigned item: un-check only removes the un-checker's completion", () => {
  const state = reduceTodoResponses(UNASSIGNED_CARD, CARD_EVENT_ID, [
    response({ pubkey: TOM_PUBKEY, itemId: "a1", done: true, createdAt: 100 }),
    response({
      pubkey: YASHODA_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 200,
    }),
    response({
      pubkey: YASHODA_PUBKEY,
      itemId: "a1",
      done: false,
      createdAt: 300,
    }),
  ]);
  // Yashoda un-checked hers; Tom's completion still stands.
  assert.deepEqual(state.get("a1"), {
    done: true,
    completedBy: TOM_PUBKEY,
    completedAt: 100,
  });
});

test("responses for another card are ignored", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: TOM_PUBKEY,
      itemId: "a1",
      done: true,
      createdAt: 100,
      cardId: OTHER_CARD_ID,
    }),
  ]);
  assert.equal(state.get("a1")?.done, false);
});

test("responses for unknown item ids are ignored", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: TOM_PUBKEY,
      itemId: "nope",
      done: true,
      createdAt: 100,
    }),
  ]);
  assert.equal(countDoneItems(TWO_ITEM_CARD, state), 0);
});

test("malformed response content is ignored", () => {
  const state = reduceTodoResponses(TWO_ITEM_CARD, CARD_EVENT_ID, [
    response({
      pubkey: TOM_PUBKEY,
      itemId: "a1",
      createdAt: 100,
      content: "not json",
    }),
    response({
      pubkey: TOM_PUBKEY,
      itemId: "a1",
      createdAt: 200,
      content: JSON.stringify({ done: "yes" }),
    }),
  ]);
  assert.equal(state.get("a1")?.done, false);
});
