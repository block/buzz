import assert from "node:assert/strict";
import test from "node:test";

import { openPresenceSubscription } from "./presenceRelaySubscription.ts";
import { KIND_PRESENCE_UPDATE } from "../constants/kinds.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);

function liveWithReadiness(readiness) {
  const calls = [];
  let closeCount = 0;
  const openLive = async (filter, onEvent, onReady, readinessTimeoutMs) => {
    calls.push({ filter, onEvent, readinessTimeoutMs });
    onReady(readiness);
    return async () => {
      closeCount += 1;
    };
  };
  return { calls, closeCount: () => closeCount, openLive };
}

test("presence waits for EOSE and sends only normalized authors", async () => {
  const { calls, closeCount, openLive } = liveWithReadiness("eose");
  const onEvent = () => {};
  const close = await openPresenceSubscription(
    [B, A.toUpperCase(), A],
    onEvent,
    openLive,
  );

  assert.deepEqual(calls, [
    {
      filter: { kinds: [KIND_PRESENCE_UPDATE], authors: [A, B], limit: 0 },
      onEvent,
      readinessTimeoutMs: 5_000,
    },
  ]);
  assert.equal(closeCount(), 0);
  await close();
  assert.equal(closeCount(), 1);
});

for (const readiness of ["timeout", "closed"]) {
  test(`presence ${readiness} closes the candidate and rejects`, async () => {
    const { closeCount, openLive } = liveWithReadiness(readiness);
    await assert.rejects(
      openPresenceSubscription([A], () => {}, openLive),
      readiness === "closed" ? /rejected/ : /timed out/i,
    );
    assert.equal(closeCount(), 1);
  });
}

test("empty demand never reaches the relay subscribe primitive", async () => {
  const { calls, openLive } = liveWithReadiness("eose");
  await assert.rejects(
    openPresenceSubscription([], () => {}, openLive),
    /at least one author/,
  );
  assert.deepEqual(calls, []);
});
