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

test("normalizeTimelineMessages caps visible content, not unrelated auxiliary events", () => {
  const june12Roots = [
    "1f86d0450b3c2c376691e7a8232fbd8a5b8408ecad2f5eb0209e7bfcfdf9af80",
    "3b745de3d9ff91c464b0cbf26e3e628a5a8c05dccf3f9781b1fbd99a0f6f5e7b",
    "cb404eeae1517bb2ed2e9975dfd2efd12d0a7ef17b87c3a6573b251b9865f7a4",
    "337de49c712fcf84f5689a6c11ce36018817197d578468b8132c6dc3d1a13131",
    "ac683f35cda2e8c1e9ae609e0b9f5dc23a7d434637b4f0505495c3a2b6f52aae",
  ];
  const messages = [];

  for (let index = 0; index < 500; index += 1) {
    messages.push(event({ id: id("old", index), createdAt: 1_000 + index }));
  }
  for (const [index, rootId] of june12Roots.entries()) {
    messages.push(
      event({ id: rootId, createdAt: 2_000 + index, content: "June 12 root" }),
    );
  }
  for (let index = 0; index < 1_303; index += 1) {
    messages.push(
      event({
        id: id("del", index),
        kind: 5,
        createdAt: 3_000 + index,
        tags: [
          ["h", CHANNEL_ID],
          ["e", id("zzz", index)],
        ],
      }),
    );
  }
  for (let index = 0; index < 231; index += 1) {
    messages.push(
      event({
        id: id("rea", index),
        kind: 7,
        createdAt: 5_000 + index,
        tags: [
          ["h", CHANNEL_ID],
          ["e", id("yyy", index)],
        ],
        content: "+",
      }),
    );
  }
  for (let index = 0; index < 1_495; index += 1) {
    messages.push(event({ id: id("new", index), createdAt: 6_000 + index }));
  }

  const normalized = normalizeTimelineMessages(messages);

  assert.equal(normalized.filter((item) => item.kind === 9).length, 2_000);
  assert.deepEqual(
    june12Roots.map((rootId) => normalized.some((item) => item.id === rootId)),
    [true, true, true, true, true],
  );
  assert.equal(normalized.filter((item) => item.kind === 5).length, 1_303);
  assert.equal(normalized.filter((item) => item.kind === 7).length, 231);
});

test("normalizeTimelineMessages still caps old visible content", () => {
  const retainedRoot = `${"a".repeat(63)}1`;
  const reaction = `${"b".repeat(63)}1`;
  const reactionDeletion = `${"c".repeat(63)}1`;
  const messages = [];

  for (let index = 0; index < 2_000; index += 1) {
    messages.push(event({ id: id("old", index), createdAt: 1_000 + index }));
  }
  messages.push(event({ id: retainedRoot, createdAt: 4_000 }));
  messages.push(
    event({
      id: reaction,
      kind: 7,
      createdAt: 4_001,
      tags: [
        ["h", CHANNEL_ID],
        ["e", retainedRoot],
      ],
      content: "+",
    }),
  );
  messages.push(
    event({
      id: reactionDeletion,
      kind: 5,
      createdAt: 4_002,
      tags: [
        ["h", CHANNEL_ID],
        ["e", reaction],
      ],
    }),
  );

  const normalized = normalizeTimelineMessages(messages);

  assert.equal(
    normalized.some((item) => item.id === id("old", 0)),
    false,
  );
  assert.equal(
    normalized.some((item) => item.id === retainedRoot),
    true,
  );
  assert.equal(
    normalized.some((item) => item.id === reaction),
    true,
  );
  assert.equal(
    normalized.some((item) => item.id === reactionDeletion),
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
