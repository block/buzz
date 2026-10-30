import assert from "node:assert/strict";
import test from "node:test";

import {
  BoundedCache,
  createEmojiResolver,
  createPrefixSearcher,
  EMOJI_AUTOCOMPLETE_DEBOUNCE_MS,
  EMOJI_NEGATIVE_TTL_MS,
  normalizeEmojiSearchPrefix,
} from "./emojiCatalogSource.ts";

const emoji = (shortcode) => ({ shortcode, url: `https://x/${shortcode}.png` });

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("BoundedCache evicts the least recently used key past its cap", () => {
  const cache = new BoundedCache(2);
  cache.set("a", 1);
  cache.set("b", 2);
  cache.get("a");
  cache.set("c", 3);
  assert.equal(cache.size, 2);
  assert.equal(cache.get("b"), undefined);
  assert.equal(cache.get("a"), 1);
  assert.equal(cache.get("c"), 3);
});

test("search prefix turns separators into spaces like the entry content", () => {
  assert.equal(normalizeEmojiSearchPrefix(":Party_Par-"), "party par");
});

test("searcher waits for the debounce, rejects short prefixes, and caches by prefix", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const calls = [];
  const searcher = createPrefixSearcher({
    fetch: async (prefix) => {
      calls.push(prefix);
      return [emoji("party_parrot")];
    },
    debounceMs: EMOJI_AUTOCOMPLETE_DEBOUNCE_MS,
    resultCap: 20,
  });

  assert.deepEqual(await searcher.request("p"), { status: "short" });

  const first = searcher.request("par");
  t.mock.timers.tick(EMOJI_AUTOCOMPLETE_DEBOUNCE_MS - 1);
  assert.deepEqual(calls, []);
  t.mock.timers.tick(1);
  const outcome = await first;
  assert.equal(outcome.status, "ok");
  assert.deepEqual(calls, ["par"]);

  const cached = await searcher.request("PAR");
  assert.equal(cached.status, "ok");
  assert.deepEqual(calls, ["par"]);
});

test("a newer request fences out an older reply and a superseded debounce", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const slow = deferred();
  const fast = deferred();
  const replies = {
    pa: slow,
    par: fast,
    zzz: { promise: Promise.resolve([emoji("zzz")]) },
  };
  const searcher = createPrefixSearcher({
    fetch: (prefix) => replies[prefix].promise,
    debounceMs: 250,
    resultCap: 20,
  });

  const older = searcher.request("pa");
  t.mock.timers.tick(250);
  const newer = searcher.request("par");
  t.mock.timers.tick(250);
  fast.resolve([emoji("parrot")]);
  slow.resolve([emoji("pa_old")]);
  assert.equal((await newer).status, "ok");
  assert.equal((await older).status, "stale");

  const superseded = searcher.request("zz");
  const replacement = searcher.request("zzz");
  assert.equal((await superseded).status, "stale");
  t.mock.timers.tick(250);
  assert.equal((await replacement).status, "ok");
});

test("reset fences an in-flight reply from the old community and clears the cache", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const reply = deferred();
  let fetches = 0;
  const searcher = createPrefixSearcher({
    fetch: () => {
      fetches += 1;
      return fetches === 1 ? reply.promise : Promise.resolve([emoji("new")]);
    },
    debounceMs: 250,
    resultCap: 20,
  });
  const pending = searcher.request("par");
  t.mock.timers.tick(250);
  searcher.reset();
  reply.resolve([emoji("old_community")]);
  assert.equal((await pending).status, "stale");

  const again = searcher.request("par");
  t.mock.timers.tick(250);
  const outcome = await again;
  assert.equal(outcome.status, "ok");
  assert.deepEqual(outcome.results, [emoji("new")]);
  assert.equal(fetches, 2);
});

test("searcher caps results and the cache stays bounded", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const many = Array.from({ length: 60 }, (_, i) => emoji(`e${i}`));
  let fetches = 0;
  const searcher = createPrefixSearcher({
    fetch: async () => {
      fetches += 1;
      return many;
    },
    debounceMs: 250,
    resultCap: 20,
    cacheCap: 2,
  });
  for (const prefix of ["aa", "bb", "cc"]) {
    const p = searcher.request(prefix);
    t.mock.timers.tick(250);
    const outcome = await p;
    assert.equal(outcome.results.length, 20);
  }
  assert.equal(fetches, 3);
  const cachedHit = await searcher.request("cc");
  assert.equal(cachedHit.results.length, 20);
  const evicted = searcher.request("aa");
  t.mock.timers.tick(250);
  await evicted;
  assert.equal(fetches, 4);
});

test("searcher reports a failed fetch instead of an empty success", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const searcher = createPrefixSearcher({
    fetch: async () => {
      throw new Error("boom");
    },
    debounceMs: 250,
    resultCap: 20,
  });
  const p = searcher.request("par");
  t.mock.timers.tick(250);
  const outcome = await p;
  assert.equal(outcome.status, "error");
  const retry = searcher.request("par");
  t.mock.timers.tick(250);
  assert.equal((await retry).status, "error");
});

test("resolver answers from cache and sends one batched fetch for the rest", async () => {
  const fetched = [];
  const resolver = createEmojiResolver({
    fetch: async (names) => {
      fetched.push(names);
      return names.filter((n) => n !== "nope").map(emoji);
    },
  });
  resolver.remember([emoji("cached")]);
  const result = await resolver.resolve([
    ":Cached:",
    "a",
    "b",
    "a",
    "nope",
    "bad name",
  ]);
  assert.deepEqual(fetched, [["a", "b", "nope"]]);
  assert.deepEqual([...result.found.keys()].sort(), ["a", "b", "cached"]);
  assert.equal(result.failed, false);

  await resolver.resolve(["a", "b", "cached"]);
  assert.equal(fetched.length, 1);
});

test("resolver remembers absent names until the negative TTL passes, but not failures", async () => {
  let clock = 1000;
  let calls = 0;
  let failNext = true;
  const resolver = createEmojiResolver({
    now: () => clock,
    fetch: async () => {
      calls += 1;
      if (failNext) {
        failNext = false;
        throw new Error("offline");
      }
      return [];
    },
  });
  const failed = await resolver.resolve(["ghost"]);
  assert.equal(failed.failed, true);
  assert.equal(failed.found.size, 0);

  await resolver.resolve(["ghost"]);
  assert.equal(calls, 2);
  await resolver.resolve(["ghost"]);
  assert.equal(calls, 2);
  clock += EMOJI_NEGATIVE_TTL_MS + 1;
  await resolver.resolve(["ghost"]);
  assert.equal(calls, 3);
});

test("resolver reset drops the cache and ignores a lookup still in flight", async () => {
  const reply = deferred();
  const resolver = createEmojiResolver({ fetch: () => reply.promise });
  const pending = resolver.resolve(["late"]);
  resolver.reset();
  reply.resolve([emoji("late")]);
  assert.equal((await pending).found.size, 0);
  assert.equal(resolver.lookup("late"), undefined);
});

test("resolver covers at most maxNames per fetch and does not cache the rest as absent", async () => {
  const fetched = [];
  const resolver = createEmojiResolver({
    maxNames: 2,
    fetch: async (names) => {
      fetched.push([...names]);
      return names.map(emoji);
    },
  });
  const first = await resolver.resolve(["a", "b", "c"]);
  assert.deepEqual(fetched, [["a", "b"]]);
  assert.deepEqual([...first.found.keys()], ["a", "b"]);
  const second = await resolver.resolve(["c"]);
  assert.deepEqual(fetched, [["a", "b"], ["c"]]);
  assert.deepEqual([...second.found.keys()], ["c"]);
});
