import assert from "node:assert/strict";
import test from "node:test";

import { queueReducer } from "./spawnerPromptUpdateQueue.ts";

test("enqueue is latest-write-wins per agent", () => {
  let s = queueReducer(new Map(), {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h1",
    queuedAt: 1,
  });
  s = queueReducer(s, {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h2",
    queuedAt: 2,
  });
  assert.equal(s.get("sp:ag").promptHash, "h2");
  assert.equal(s.size, 1);
});

test("matching ack clears, stale ack does not", () => {
  const s = queueReducer(new Map(), {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h2",
    queuedAt: 2,
  });
  assert.equal(
    queueReducer(s, { type: "ack", key: "sp:ag", promptHash: "h1" }).size,
    1,
  );
  assert.equal(
    queueReducer(s, { type: "ack", key: "sp:ag", promptHash: "h2" }).size,
    0,
  );
});

test("ack for an unknown key is a no-op", () => {
  const s = queueReducer(new Map(), {
    type: "ack",
    key: "sp:ag",
    promptHash: "h1",
  });
  assert.equal(s.size, 0);
});

test("ack with a nullish promptHash is a no-op", () => {
  const s = queueReducer(new Map(), {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h2",
    queuedAt: 2,
  });
  assert.equal(
    queueReducer(s, { type: "ack", key: "sp:ag", promptHash: null }).size,
    1,
  );
  assert.equal(
    queueReducer(s, { type: "ack", key: "sp:ag", promptHash: undefined }).size,
    1,
  );
});

test("reset clears every pending entry", () => {
  let s = queueReducer(new Map(), {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h1",
    queuedAt: 1,
  });
  s = queueReducer(s, { type: "reset" });
  assert.equal(s.size, 0);
});
