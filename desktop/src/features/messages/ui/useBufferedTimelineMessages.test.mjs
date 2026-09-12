import assert from "node:assert/strict";
import test from "node:test";

import { selectBufferedTimelineMessages } from "./useBufferedTimelineMessages.ts";

const rows = (...ids) => ids.map((id) => ({ id }));

test("freezes live arrivals after the semantic tail while scrolled up", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenMessageIds: ["a", "b", "c"],
      isAtBottom: false,
      messages: rows("a", "b", "c", "d", "e"),
    }).map(({ id }) => id),
    ["a", "b", "c"],
  );
});

test("admits older-history prepends without exposing buffered arrivals", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenMessageIds: ["a", "b", "c"],
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
      frozenMessageIds: ["a", "b"],
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
      frozenMessageIds: ["old-tail"],
      isAtBottom: false,
      messages,
    }),
    messages,
  );
});

test("admits reconnect gap rows inside the frozen window, buffering only rows after its tail", () => {
  assert.deepEqual(
    selectBufferedTimelineMessages({
      frozenMessageIds: ["old-live", "newer", "tail"],
      isAtBottom: false,
      messages: rows("old-live", "gap-1", "gap-2", "newer", "tail", "live"),
    }).map((e) => e.id),
    ["old-live", "gap-1", "gap-2", "newer", "tail"],
  );
});
