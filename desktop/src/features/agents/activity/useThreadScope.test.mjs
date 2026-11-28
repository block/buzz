import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  createThreadEventCache,
  resolveTurnThreadsRetriable,
} from "./useThreadScope.ts";

function relayEvent(id) {
  return {
    id,
    pubkey: "f".repeat(64),
    created_at: 1,
    kind: 9,
    tags: [],
    content: "hello",
    sig: "sig",
  };
}

describe("createThreadEventCache", () => {
  it("caches successful event lookups", async () => {
    let fetches = 0;
    const cache = createThreadEventCache(async (eventId) => {
      fetches += 1;
      return relayEvent(eventId);
    });

    const first = await cache.fetchEventCached("evt");
    const second = await cache.fetchEventCached("evt");

    assert.equal(first?.id, "evt");
    assert.equal(second, first);
    assert.equal(fetches, 1);
  });

  it("caches definitive event-not-found misses", async () => {
    let fetches = 0;
    const cache = createThreadEventCache(async () => {
      fetches += 1;
      throw new Error("get_event: event not found");
    });

    assert.equal(await cache.fetchEventCached("missing"), null);
    assert.equal(await cache.fetchEventCached("missing"), null);
    assert.equal(fetches, 1);
  });

  it("rejects and evicts transient lookups so later calls can retry", async () => {
    let fetches = 0;
    const cache = createThreadEventCache(async (eventId) => {
      fetches += 1;
      if (fetches === 1) {
        throw new Error("relay temporarily unavailable");
      }
      return relayEvent(eventId);
    });

    await assert.rejects(
      () => cache.fetchEventCached("eventual"),
      /relay temporarily unavailable/,
    );
    assert.equal((await cache.fetchEventCached("eventual"))?.id, "eventual");
    assert.equal(fetches, 2);
  });
});

describe("resolveTurnThreadsRetriable", () => {
  it("does not record a resolution when a transient lookup rejects", async () => {
    const resolved = await resolveTurnThreadsRetriable(
      [
        {
          id: "turn-1",
          startT: 1,
          source: "channel",
          triggeringEventIds: ["event-1"],
        },
      ],
      async () => {
        throw new Error("relay temporarily unavailable");
      },
    );

    assert.equal(resolved.has("turn-1"), false);
  });
});
