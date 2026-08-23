import assert from "node:assert/strict";
import test from "node:test";

import {
  findReplyParentAuthor,
  getReplyContextEvents,
  lookupReplyParentAuthor,
  resetReplyParentAuthorCache,
  resolveReplyParentAuthor,
} from "./replyContextEvents.ts";

const CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const OTHER_CHANNEL_ID = "22222222-2222-4222-8222-222222222222";
const AUTHOR = "a".repeat(64);

// Real event ids, because `resolveReplyParentAuthor` now rejects anything that
// cannot identify a relay event before it reaches an `ids` filter.
const M1 = "1".repeat(64);
const M2 = "2".repeat(64);

function makeEvent(id, pubkey = AUTHOR) {
  return {
    id,
    pubkey,
    created_at: 1700000000,
    kind: 9,
    tags: [],
    content: "hi",
    sig: "s".repeat(128),
  };
}

/** Minimal QueryClient stand-in covering the two reads the helper makes. */
function makeQueryClient({ channelMessages = [], threadCaches = {} } = {}) {
  return {
    getQueryData: (key) =>
      key[0] === "channel-messages" && key[1] === CHANNEL_ID
        ? channelMessages
        : undefined,
    getQueriesData: ({ queryKey }) =>
      Object.entries(threadCaches)
        .filter(([cacheKey]) => cacheKey.startsWith(`${queryKey[1]}:`))
        .map(([cacheKey, events]) => [
          ["thread-replies", ...cacheKey.split(":")],
          events,
        ]),
  };
}

test("returns the channel timeline when no thread cache exists", () => {
  const message = makeEvent(M1);
  const client = makeQueryClient({ channelMessages: [message] });
  assert.deepEqual(getReplyContextEvents(client, CHANNEL_ID), [message]);
});

test("includes thread-panel replies the channel cache never holds", () => {
  const timeline = makeEvent("root-1");
  const nested = makeEvent("reply-1", "b".repeat(64));
  const client = makeQueryClient({
    channelMessages: [timeline],
    threadCaches: { [`${CHANNEL_ID}:root-1`]: [nested] },
  });
  const events = getReplyContextEvents(client, CHANNEL_ID);
  assert.deepEqual(
    events.map((event) => event.id),
    ["root-1", "reply-1"],
  );
});

test("does not leak another channel's thread cache", () => {
  const client = makeQueryClient({
    channelMessages: [],
    threadCaches: { [`${OTHER_CHANNEL_ID}:root-1`]: [makeEvent("reply-1")] },
  });
  assert.deepEqual(getReplyContextEvents(client, CHANNEL_ID), []);
});

test("tolerates an empty cache", () => {
  const client = { getQueryData: () => undefined, getQueriesData: () => [] };
  assert.deepEqual(getReplyContextEvents(client, CHANNEL_ID), []);
});

test("findReplyParentAuthor resolves the author of the parent event", () => {
  const events = [makeEvent(M1), makeEvent(M2, "c".repeat(64))];
  assert.equal(findReplyParentAuthor(events, M2), "c".repeat(64));
});

test("findReplyParentAuthor returns null for an uncached or absent parent", () => {
  assert.equal(findReplyParentAuthor([makeEvent(M1)], "missing"), null);
  assert.equal(findReplyParentAuthor([makeEvent(M1)], null), null);
  assert.equal(findReplyParentAuthor([makeEvent(M1)], undefined), null);
});

test("findReplyParentAuthor treats a blank pubkey as absent", () => {
  assert.equal(findReplyParentAuthor([makeEvent(M1, "   ")], M1), null);
});

test("lookupReplyParentAuthor short-circuits when there is no parent", () => {
  const client = {
    getQueryData: () => {
      throw new Error("should not read");
    },
    getQueriesData: () => {
      throw new Error("should not scan");
    },
  };
  assert.equal(lookupReplyParentAuthor(client, CHANNEL_ID, null), null);
  assert.equal(lookupReplyParentAuthor(client, CHANNEL_ID, undefined), null);
});

test("lookupReplyParentAuthor prefers the channel cache and skips the scan", () => {
  const client = {
    getQueryData: () => [makeEvent(M1, "d".repeat(64))],
    getQueriesData: () => {
      throw new Error("should not scan on a channel-cache hit");
    },
  };
  assert.equal(lookupReplyParentAuthor(client, CHANNEL_ID, M1), "d".repeat(64));
});

test("lookupReplyParentAuthor falls back to the thread caches on a miss", () => {
  const client = makeQueryClient({
    channelMessages: [makeEvent("root-1")],
    threadCaches: {
      [`${CHANNEL_ID}:root-1`]: [makeEvent("nested", "e".repeat(64))],
    },
  });
  assert.equal(
    lookupReplyParentAuthor(client, CHANNEL_ID, "nested"),
    "e".repeat(64),
  );
  assert.equal(lookupReplyParentAuthor(client, CHANNEL_ID, "absent"), null);
});

test("resolveReplyParentAuthor answers from the cache without a fetch", async () => {
  const client = makeQueryClient({ channelMessages: [makeEvent(M1)] });
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: () => {
      throw new Error("should not fetch");
    },
    kinds: [9],
    parentEventId: M1,
    queryClient: client,
  });
  assert.deepEqual(result, { pubkey: AUTHOR, status: "resolved" });
});

test("resolveReplyParentAuthor falls back to the relay with kinds set", async () => {
  resetReplyParentAuthorCache();
  const client = makeQueryClient();
  const requested = [];
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: async (filter) => {
      requested.push(filter);
      return [makeEvent(M1)];
    },
    kinds: [9, 40008],
    parentEventId: M1,
    queryClient: client,
  });
  assert.deepEqual(result, { pubkey: AUTHOR, status: "resolved" });
  // `kinds` is required — an open-ended filter hits the relay p-gate (403).
  assert.deepEqual(requested[0].kinds, [9, 40008]);
  assert.deepEqual(requested[0].ids, [M1]);
});

test("a parent that is genuinely gone reports absent, not unavailable", async () => {
  resetReplyParentAuthorCache();
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: async () => [],
    kinds: [9],
    parentEventId: M1,
    queryClient: makeQueryClient(),
  });
  assert.deepEqual(result, { pubkey: null, status: "absent" });
});

test("a failed fetch retries before reporting unavailable", async () => {
  resetReplyParentAuthorCache();
  let attempts = 0;
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: async () => {
      attempts += 1;
      throw new Error("relay down");
    },
    kinds: [9],
    parentEventId: M1,
    queryClient: makeQueryClient(),
  });
  assert.deepEqual(result, { pubkey: null, status: "unavailable" });
  // The caller persists its verdict per event id and never recomputes it, so
  // a guess made during a brief blip would be permanent.
  assert.equal(attempts, 3);
});

test("a transient failure resolves once the relay recovers", async () => {
  resetReplyParentAuthorCache();
  let attempts = 0;
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: async () => {
      attempts += 1;
      if (attempts === 1) throw new Error("relay flap");
      return [makeEvent(M1)];
    },
    kinds: [9],
    parentEventId: M1,
    queryClient: makeQueryClient(),
  });
  assert.deepEqual(result, { pubkey: AUTHOR, status: "resolved" });
});

test("an absent parent is not cached, so a later lookup can still find it", async () => {
  resetReplyParentAuthorCache();
  const client = makeQueryClient();
  let calls = 0;
  const resolve = () =>
    resolveReplyParentAuthor({
      channelId: CHANNEL_ID,
      fetchEvents: async () => {
        calls += 1;
        // A parent missing from one query may simply be a kind outside
        // `kinds`; caching that would poison every later reply to it.
        return calls === 1 ? [] : [makeEvent(M1)];
      },
      kinds: [9],
      parentEventId: M1,
      queryClient: client,
    });
  assert.deepEqual(await resolve(), { pubkey: null, status: "absent" });
  assert.deepEqual(await resolve(), { pubkey: AUTHOR, status: "resolved" });
});

test("no parent short-circuits to absent without a fetch", async () => {
  const result = await resolveReplyParentAuthor({
    channelId: CHANNEL_ID,
    fetchEvents: () => {
      throw new Error("should not fetch");
    },
    kinds: [9],
    parentEventId: null,
    queryClient: makeQueryClient(),
  });
  assert.deepEqual(result, { pubkey: null, status: "absent" });
});

test("concurrent lookups of one parent share a single fetch", async () => {
  resetReplyParentAuthorCache();
  const client = makeQueryClient();
  let fetches = 0;
  const resolve = () =>
    resolveReplyParentAuthor({
      channelId: CHANNEL_ID,
      fetchEvents: async () => {
        fetches += 1;
        return [makeEvent(M1)];
      },
      kinds: [9],
      parentEventId: M1,
      queryClient: client,
    });
  // 30 replies to one message must not cost 30 identical relay subscriptions.
  const results = await Promise.all(Array.from({ length: 30 }, resolve));
  assert.equal(fetches, 1);
  for (const result of results) {
    assert.deepEqual(result, { pubkey: AUTHOR, status: "resolved" });
  }
});

test("a lookup that exhausts its retries is not cached", async () => {
  resetReplyParentAuthorCache();
  const client = makeQueryClient();
  let down = true;
  const resolve = () =>
    resolveReplyParentAuthor({
      channelId: CHANNEL_ID,
      fetchEvents: async () => {
        if (down) throw new Error("relay down");
        return [makeEvent(M1)];
      },
      kinds: [9],
      parentEventId: M1,
      queryClient: client,
    });
  assert.deepEqual(await resolve(), { pubkey: null, status: "unavailable" });
  down = false;
  // Caching the failure would make one outage sticky for the whole session.
  assert.deepEqual(await resolve(), { pubkey: AUTHOR, status: "resolved" });
});
