import assert from "node:assert/strict";
import test from "node:test";

import {
  retainedWindowBounds,
  RETAINED_TIMELINE_WINDOW_SIZE,
} from "./useRetainedTimelineWindow.ts";

const messages = (start, count) =>
  Array.from({ length: count }, (_, index) => ({
    id: `message-${start + index}`,
  }));
const range = (values = {}) => ({
  channelId: "channel",
  firstId: "message-350",
  followLatest: false,
  lastId: "message-599",
  sourceFirstId: "message-0",
  sourceLastId: "message-599",
  ...values,
});

test("retained timeline mounts no more than 250 real messages", () => {
  const all = messages(0, 600);
  assert.deepEqual(retainedWindowBounds(all, range()), {
    start: 350,
    end: 600,
  });
  assert.equal(600 - 350, RETAINED_TIMELINE_WINDOW_SIZE);
});

test("new arrivals stay outside the retained window while reading history", () => {
  const all = messages(0, 601);
  assert.deepEqual(retainedWindowBounds(all, range()), {
    start: 350,
    end: 600,
  });
});

test("the latest window follows ordinary appends while pinned to bottom", () => {
  const all = messages(0, 601);
  assert.deepEqual(retainedWindowBounds(all, range({ followLatest: true })), {
    start: 351,
    end: 601,
  });
});

test("history prepends retain the same semantic message boundaries", () => {
  const all = messages(-50, 650);
  assert.deepEqual(retainedWindowBounds(all, range({ followLatest: true })), {
    start: 400,
    end: 650,
  });
});
