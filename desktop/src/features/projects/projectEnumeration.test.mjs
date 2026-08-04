import assert from "node:assert/strict";
import test from "node:test";

import { enumerateProjectEvents } from "./projectEnumeration.ts";

function relayEvent(id, createdAt) {
  return {
    id: id.repeat(64),
    kind: 30617,
    pubkey: "a".repeat(64),
    created_at: createdAt,
    content: "",
    tags: [["d", id]],
  };
}

function fetcherFor(events) {
  return async ({ limit, since, until }) =>
    events
      .filter(
        (event) =>
          (since === undefined || event.created_at >= since) &&
          (until === undefined || event.created_at <= until),
      )
      .sort(
        (left, right) =>
          right.created_at - left.created_at || left.id.localeCompare(right.id),
      )
      .slice(0, limit);
}

test("enumerateProjectEvents drains a tied boundary second before advancing", async () => {
  const events = [
    relayEvent("a", 1_000),
    relayEvent("b", 900),
    relayEvent("c", 900),
    relayEvent("d", 800),
  ];

  const result = await enumerateProjectEvents(fetcherFor(events), [30617], 3);

  assert.deepEqual(
    result.map((event) => event.id).sort(),
    events.map((event) => event.id).sort(),
  );
});

test("enumerateProjectEvents refuses to present a truncated boundary as complete", async () => {
  const events = [
    relayEvent("a", 1_000),
    relayEvent("b", 1_000),
    relayEvent("c", 1_000),
  ];

  await assert.rejects(
    enumerateProjectEvents(fetcherFor(events), [30617], 2),
    /cannot exhaustively enumerate/,
  );
});
