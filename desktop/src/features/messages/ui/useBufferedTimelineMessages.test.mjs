import assert from "node:assert/strict";
import test from "node:test";

import { selectBufferedTimelineMessages } from "./useBufferedTimelineMessages.ts";

const rows = (...ids) => ids.map((id) => ({ id }));

test("freezes live arrivals after the semantic tail while scrolled up", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "b", "c"],
      isAtBottom: false,
      messages: rows("a", "b", "c", "d", "e"),
    }).map(({ id }) => id),
    ["a", "b", "c"],
  );
});

test("admits older-history prepends without exposing buffered arrivals", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "b", "c"],
      isAtBottom: false,
      messages: rows("older-a", "older-b", "a", "b", "c", "d"),
    }).map(({ id }) => id),
    ["older-a", "older-b", "a", "b", "c"],
  );
});

test("releases the full logical dataset at bottom", () => {
  const messages = rows("a", "b", "c", "d");
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "b"],
      isAtBottom: true,
      messages,
    }),
    messages,
  );
});

test("accepts an authoritative replacement when its old tail disappeared", () => {
  const messages = rows("x", "y");
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["old-tail"],
      isAtBottom: false,
      messages,
    }),
    messages,
  );
});

test("keeps the tail frozen when a frozen row is deleted", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "b", "c"],
      isAtBottom: false,
      messages: rows("a", "b", "d"),
    }).map(({ id }) => id),
    ["a", "b"],
  );
});

test("keeps frozen rows once each when they come back reordered", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "b", "c"],
      isAtBottom: false,
      messages: rows("older", "b", "a", "c", "d"),
    }).map(({ id }) => id),
    ["older", "a", "b", "c"],
  );
});

test("keeps the tail frozen when a frozen optimistic row is acknowledged", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenRowKeys: ["a", "pending-b"],
      isAtBottom: false,
      messages: [
        ...rows("a"),
        { id: "b", renderKey: "pending-b" },
        ...rows("c"),
      ],
    }).map(({ id }) => id),
    ["a", "b"],
  );
});
