import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeTimelineHistoryMessages,
  normalizeTimelineMessages,
} from "./messageQueryKeys.ts";

const CHANNEL_ID = "timeline-window-test";
const PUBKEY = "a".repeat(64);

function event({ id, kind = 9, createdAt, tags, content = "" }) {
  return {
    id,
    pubkey: PUBKEY,
    created_at: createdAt,
    kind,
    tags: tags ?? [["h", CHANNEL_ID]],
    content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

function id(prefix, index) {
  return `${prefix}${String(index).padStart(64 - prefix.length, "0")}`;
}

test("normalizeTimelineMessages preserves the complete loaded window", () => {
  const messages = [];
  for (let index = 0; index < 2_100; index += 1) {
    messages.push(event({ id: id("row", index), createdAt: 1_000 + index }));
  }
  messages.push(
    event({
      id: id("aux", 0),
      kind: 7,
      createdAt: 4_000,
      tags: [
        ["h", CHANNEL_ID],
        ["e", id("row", 0)],
      ],
      content: "+",
    }),
  );

  const normalized = normalizeTimelineMessages(messages);

  assert.equal(normalized.filter((item) => item.kind === 9).length, 2_100);
  assert.equal(
    normalized.some((item) => item.id === id("row", 0)),
    true,
  );
  assert.equal(
    normalized.some((item) => item.id === id("aux", 0)),
    true,
  );
});

test("timeline history merge preserves freshly fetched older content roots", () => {
  const current = [];
  const olderPage = [];

  for (let index = 0; index < 2_000; index += 1) {
    current.push(event({ id: id("new", index), createdAt: 10_000 + index }));
  }
  for (let index = 0; index < 100; index += 1) {
    olderPage.push(event({ id: id("old", index), createdAt: 1_000 + index }));
  }

  const merged = mergeTimelineHistoryMessages(current, olderPage);
  const mergedContent = merged
    .filter((item) => item.kind === 9)
    .map((item) => item.id);

  assert.equal(mergedContent.length, 2_100);
  assert.equal(mergedContent[0], id("old", 0));
  assert.equal(mergedContent[99], id("old", 99));
  assert.equal(mergedContent[100], id("new", 0));
  assert.equal(mergedContent.at(-1), id("new", 1_999));
});

test("timeline history merge preserves the older window despite auxiliary events", () => {
  const seedMessages = [];
  const olderPage = [];

  for (let index = 0; index < 700; index += 1) {
    seedMessages.push(
      event({ id: id("new", index), createdAt: 10_000 + index }),
    );
  }
  for (let index = 0; index < 1_303; index += 1) {
    seedMessages.push(
      event({
        id: id("del", index),
        kind: 5,
        createdAt: 11_000 + index,
        tags: [
          ["h", CHANNEL_ID],
          ["e", id("zzz", index)],
        ],
      }),
    );
  }
  for (let index = 0; index < 231; index += 1) {
    seedMessages.push(
      event({
        id: id("rea", index),
        kind: 7,
        createdAt: 13_000 + index,
        tags: [
          ["h", CHANNEL_ID],
          ["e", id("yyy", index)],
        ],
        content: "+",
      }),
    );
  }
  for (let index = 0; index < 1_500; index += 1) {
    olderPage.push(event({ id: id("old", index), createdAt: 1_000 + index }));
  }

  const merged = mergeTimelineHistoryMessages(seedMessages, olderPage);
  const mergedContent = merged
    .filter((item) => item.kind === 9)
    .map((item) => item.id);

  assert.equal(mergedContent.length, 2_200);
  assert.equal(mergedContent[0], id("old", 0));
  assert.equal(mergedContent[1_499], id("old", 1_499));
  assert.equal(mergedContent[1_500], id("new", 0));
  assert.equal(mergedContent.at(-1), id("new", 699));
  assert.equal(merged.filter((item) => item.kind === 5).length, 1_303);
  assert.equal(merged.filter((item) => item.kind === 7).length, 231);
});

test("sortMessages tiebreaks same-second events on id, order-independent", () => {
  // Three events sharing one created_at, fed in two different input orders.
  // The (created_at, id) sort must produce the same sequence both ways, so a
  // history-then-live merge and a live-then-history merge can't shuffle a
  // same-second message to a different visible position.
  const a = event({ id: id("aaa", 1), createdAt: 5_000 });
  const b = event({ id: id("bbb", 1), createdAt: 5_000 });
  const c = event({ id: id("ccc", 1), createdAt: 5_000 });

  const forward = normalizeTimelineMessages([a, b, c]).map((m) => m.id);
  const reverse = normalizeTimelineMessages([c, b, a]).map((m) => m.id);

  assert.deepEqual(forward, reverse);
  assert.deepEqual(forward, [a.id, b.id, c.id]);
});
