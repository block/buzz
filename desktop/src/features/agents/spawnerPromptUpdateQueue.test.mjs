import assert from "node:assert/strict";
import test from "node:test";

import {
  findAckKey,
  mergeHydrated,
  queueReducer,
  shouldRetryPromptUpdate,
} from "./spawnerPromptUpdateQueue.ts";

function pending(entries) {
  return new Map(entries.map((entry) => [entry.key, entry]));
}

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

test("hydration keeps entries queued before the relay origin resolved", () => {
  // A write made while the origin was unknown lives in memory only, since
  // there was no community-scoped key to persist it to. Hydration must merge
  // rather than replace, or that edit is lost.
  const stored = new Map([["sp:a", { promptHash: "stored" }]]);
  const inMemory = new Map([["sp:b", { promptHash: "in-memory" }]]);
  const merged = mergeHydrated(stored, inMemory);
  assert.deepEqual([...merged.keys()], ["sp:a", "sp:b"]);
});

test("hydration lets the newer in-memory entry win a key collision", () => {
  const stored = new Map([["sp:a", { promptHash: "stored" }]]);
  const inMemory = new Map([["sp:a", { promptHash: "in-memory" }]]);
  assert.equal(
    mergeHydrated(stored, inMemory).get("sp:a").promptHash,
    "in-memory",
  );
});

test("hydration with one side empty returns the other untouched", () => {
  const stored = new Map([["sp:a", { promptHash: "stored" }]]);
  assert.equal(mergeHydrated(stored, new Map()), stored);
  assert.equal(mergeHydrated(new Map(), stored), stored);
});

test("an ack matches on agent pubkey even when the slug differs", () => {
  // The slug a client queues under falls back to one derived from the agent's
  // name, so it can drift from the spawner's after a rename or a late spec
  // load. Matching on slug alone would never ack, resending forever.
  const state = pending([
    { key: "sp:ag", spawnerPubkey: "sp", agentPubkey: "ag", specSlug: "old" },
  ]);
  assert.equal(findAckKey(state, "sp", "ag", "renamed"), "sp:ag");
});

test("an ack falls back to the slug only when no agent pubkey is reported", () => {
  const state = pending([
    { key: "sp:ag", spawnerPubkey: "sp", agentPubkey: "ag", specSlug: "fizz" },
  ]);
  assert.equal(findAckKey(state, "sp", null, "fizz"), "sp:ag");
  // A reported-but-unknown agent pubkey is a different agent, not a slug hint.
  assert.equal(findAckKey(state, "sp", "other", "fizz"), null);
});

test("an ack never crosses spawners", () => {
  const state = pending([
    { key: "sp:ag", spawnerPubkey: "sp", agentPubkey: "ag", specSlug: "fizz" },
  ]);
  assert.equal(findAckKey(state, "other-sp", "ag", "fizz"), null);
});

test("a delivered, unacked entry is not retried immediately", () => {
  // Rust's reconcile loop republishes the spawner's announcement right after
  // applying a prompt update, before the confirming status has a chance to
  // land — retrying here would force a needless repeat container restart.
  const now = 1_000_000;
  const entry = { promptHash: "h1", queuedAt: now, lastSentAt: now };
  assert.equal(shouldRetryPromptUpdate(entry, now + 1_000), false);
});

test("a delivered entry is retried once it has sat unacked past the floor", () => {
  const now = 1_000_000;
  const entry = { promptHash: "h1", queuedAt: now, lastSentAt: now };
  assert.equal(shouldRetryPromptUpdate(entry, now + 4 * 60 * 1000), true);
});

test("an entry whose last send failed is always retried", () => {
  const now = 1_000_000;
  const entry = { promptHash: "", queuedAt: now, lastSentAt: now };
  assert.equal(shouldRetryPromptUpdate(entry, now + 1), true);
});

test("ack still clears a delivered entry regardless of how recently it sent", () => {
  const now = 1_000_000;
  const s = queueReducer(new Map(), {
    type: "enqueue",
    key: "sp:ag",
    promptHash: "h1",
    queuedAt: now,
    lastSentAt: now,
  });
  assert.equal(
    queueReducer(s, { type: "ack", key: "sp:ag", promptHash: "h1" }).size,
    0,
  );
});
