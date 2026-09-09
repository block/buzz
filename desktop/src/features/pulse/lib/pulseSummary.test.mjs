import assert from "node:assert/strict";
import { test } from "node:test";
import { buildSummaryInput, parsePulseSummary } from "./pulseSummary.ts";
const now = 1_800_000_000;
const reads = { isThreadMuted: () => false, isFollowingThread: () => false };
const item = (id, extra = {}) => ({
  id,
  rootId: id,
  latestAt: now,
  channel: { id: "dm", name: "Alice", channelType: "dm" },
  messages: [
    {
      id,
      pubkey: "alice",
      author: "Alice",
      body: "Can you review the layout?",
      createdAt: now,
    },
  ],
  ...extra,
});

test("summary context joins sequential DM messages so later answers can resolve questions", () => {
  const input = buildSummaryInput(
    [
      item("question"),
      item("answer", {
        messages: [
          {
            id: "answer",
            pubkey: "me",
            author: "You",
            body: "Yes, approved.",
            createdAt: now + 1,
          },
        ],
      }),
    ],
    "me",
    reads,
    "scope",
    now,
  );
  assert.equal(input.conversations[0].messages.length, 2);
  assert.equal(input.conversations[0].messages[1].isViewer, true);
  assert.equal(input.conversations[0].messages[1].body, "Yes, approved.");
});
test("summary input bounds channels and context and excludes muted or old activity", () => {
  const input = buildSummaryInput(
    Array.from({ length: 100 }, (_, i) => item(String(i))),
    "me",
    reads,
    "scope",
    now,
  );
  assert.equal(input.conversations.length, 3);
  assert.ok(input.conversations.every((c) => c.messages.length <= 8));
  assert.equal(
    buildSummaryInput(
      [item("old", { latestAt: now - 49 * 3600 })],
      "me",
      reads,
      "scope",
      now,
    ).conversations.length,
    0,
  );
  assert.equal(
    buildSummaryInput(
      [item("muted")],
      "me",
      { ...reads, isThreadMuted: () => true },
      "scope",
      now,
    ).conversations.length,
    0,
  );
});
test("model summaries retain exact sources and reject invented links and malformed output", () => {
  const valid = {
    highlights: [
      {
        summary: "Alice has a question about the layout",
        conversationIds: ["question"],
      },
    ],
  };
  const result = parsePulseSummary(valid, new Set(["question"]));
  assert.equal(result[0].label, valid.highlights[0].summary);
  assert.deepEqual([...result[0].ids], ["question"]);
  for (const value of [
    null,
    {},
    { highlights: [{ summary: "Invented", conversationIds: ["missing"] }] },
    { highlights: [{ summary: "", conversationIds: ["question"] }] },
  ]) {
    assert.throws(() => parsePulseSummary(value, new Set(["question"])));
  }
});

test("briefing accepts ten grounded highlights and rejects overflow", () => {
  const highlights = Array.from({ length: 11 }, (_, i) => ({
    summary: `Person ${i} has an update on their project`,
    conversationIds: [`source-${i}`],
  }));
  const allowed = new Set(highlights.flatMap((item) => item.conversationIds));
  assert.equal(
    parsePulseSummary({ highlights: highlights.slice(0, 10) }, allowed).length,
    10,
  );
  assert.throws(() => parsePulseSummary({ highlights }, allowed));
});
