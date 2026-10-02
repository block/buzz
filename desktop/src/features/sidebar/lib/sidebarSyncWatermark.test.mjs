import assert from "node:assert/strict";
import test from "node:test";

// We need a minimal localStorage stub since we're running in Node.
function withFreshStorage(fn) {
  const store = new Map();
  const ls = {
    getItem: (k) => store.get(k) ?? null,
    setItem: (k, v) => store.set(k, v),
    removeItem: (k) => store.delete(k),
    clear: () => store.clear(),
  };
  const orig = globalThis.window?.localStorage;
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  globalThis.window.localStorage = ls;
  try {
    fn(ls);
  } finally {
    if (orig !== undefined) globalThis.window.localStorage = orig;
    else delete globalThis.window.localStorage;
  }
}

const { readWatermark, advanceWatermark, runBootstrap } = await import(
  "./sidebarSyncWatermark.ts"
);

// Relay URLs are normalised (trimmed, lowercase, trailing slash stripped)
// so the same relay written two ways produces the same key.
const RELAY = "wss://relay.example.com";
const RELAY_ENCODED = encodeURIComponent("wss://relay.example.com");

// ── readWatermark ────────────────────────────────────────────────────────────

test("readWatermark: returns 0 when no key exists", () => {
  withFreshStorage(() => {
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
  });
});

test("readWatermark: returns 0 when stored value is 0", () => {
  withFreshStorage((ls) => {
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`, "0");
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
  });
});

test("readWatermark: returns stored positive integer", () => {
  withFreshStorage((ls) => {
    ls.setItem(
      `buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`,
      "1700000000",
    );
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
  });
});

test("readWatermark: scopes by blobType", () => {
  withFreshStorage((ls) => {
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`, "100");
    ls.setItem(`buzz-sync-watermark.v1:sort:pk:${RELAY_ENCODED}`, "200");
    assert.equal(readWatermark("pk", "sections", RELAY), 100);
    assert.equal(readWatermark("pk", "sort", RELAY), 200);
  });
});

test("readWatermark: normalises relay URL (trailing slash, case)", () => {
  withFreshStorage(() => {
    // Write with one form, read with another — must produce the same value.
    advanceWatermark("pk", "sections", "WSS://Relay.Example.Com/", 999);
    assert.equal(
      readWatermark("pk", "sections", "wss://relay.example.com"),
      999,
    );
    assert.equal(
      readWatermark("pk", "sections", "WSS://Relay.Example.Com/"),
      999,
    );
  });
});

// ── advanceWatermark ─────────────────────────────────────────────────────────

test("advanceWatermark: writes when no prior value exists", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", RELAY, 1700000000);
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
  });
});

test("advanceWatermark: advances when next > current", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", RELAY, 100);
    advanceWatermark("pk", "sections", RELAY, 200);
    assert.equal(readWatermark("pk", "sections", RELAY), 200);
  });
});

test("advanceWatermark: does not regress when next <= current (monotonic)", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", RELAY, 500);
    advanceWatermark("pk", "sections", RELAY, 400); // older — must not overwrite
    advanceWatermark("pk", "sections", RELAY, 500); // equal — must not overwrite
    assert.equal(readWatermark("pk", "sections", RELAY), 500);
  });
});

test("advanceWatermark: round-trips across separate reads (simulated restart)", () => {
  withFreshStorage(() => {
    // Session A writes watermark.
    advanceWatermark("pk", "sections", RELAY, 1700000042);
    // Session B reads it back.
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000042);
  });
});

// ── Relay-A / Relay-B isolation ──────────────────────────────────────────────

test("relay-A watermark does not suppress first-sync on relay-B", () => {
  withFreshStorage(() => {
    const relayA = "wss://a.relay.test";
    const relayB = "wss://b.relay.test";
    advanceWatermark("pk", "sections", relayA, 1700000100);
    assert.equal(
      readWatermark("pk", "sections", relayB),
      0,
      "relay B watermark must be independent of relay A",
    );
  });
});

test("relay-A watermark is preserved after relay-B session", () => {
  withFreshStorage(() => {
    const relayA = "wss://a.relay.test";
    const relayB = "wss://b.relay.test";
    advanceWatermark("pk", "sections", relayA, 1700000100);
    advanceWatermark("pk", "sections", relayB, 1700000200);
    assert.equal(
      readWatermark("pk", "sections", relayA),
      1700000100,
      "relay A head must not be clobbered by relay B activity",
    );
  });
});

// ── runBootstrap policy — tested once; mutations to any branch fail here ─────

function makeBootstrapArgs({ fetchResult, lastHead, localNonEmpty }) {
  let n = 0;
  return {
    args: {
      fetchResult,
      lastHead,
      localStore: { items: localNonEmpty ? ["x"] : [] },
      isLocalNonEmpty: (s) => s.items.length > 0,
      publishFn: () => {
        n++;
      },
    },
    publishCount: () => n,
  };
}

// Guard: fetch failed → hold, zero publishes.
// Mutation: removing the failed branch causes a seed on first-sync case.
test("runBootstrap: fetch failed returns hold and never calls publishFn", () => {
  const { args, publishCount } = makeBootstrapArgs({
    fetchResult: { status: "failed" },
    lastHead: 0,
    localNonEmpty: true,
  });
  const result = runBootstrap(args);
  assert.equal(result.action, "hold");
  assert.equal(
    publishCount(),
    0,
    "publishFn must not be called on failed fetch",
  );
});

// Guard: fetch absent + prior head > 0 → hold, zero publishes (stale-dev-build case).
// Mutation: setting lastHead to 0 causes a seed.
test("runBootstrap: fetch absent with prior head returns hold and never calls publishFn", () => {
  const { args, publishCount } = makeBootstrapArgs({
    fetchResult: { status: "absent" },
    lastHead: 1700000000,
    localNonEmpty: true,
  });
  const result = runBootstrap(args);
  assert.equal(result.action, "hold");
  assert.equal(
    publishCount(),
    0,
    "publishFn must not be called when prior head exists",
  );
});

// Guard: fetch absent + head 0 + local non-empty → publishFn called exactly once, hold returned.
// Mutation: removing the absent+head-0 seed call leaves publishCount at 0.
test("runBootstrap: first-sync (absent + zero head + non-empty local) calls publishFn and returns hold", () => {
  const { args, publishCount } = makeBootstrapArgs({
    fetchResult: { status: "absent" },
    lastHead: 0,
    localNonEmpty: true,
  });
  const result = runBootstrap(args);
  assert.equal(result.action, "hold");
  assert.equal(
    publishCount(),
    1,
    "publishFn must be called exactly once on first-sync",
  );
});

// Guard: fetch absent + head 0 + empty local → no publish, hold returned.
test("runBootstrap: first-sync with empty local store does not call publishFn", () => {
  const { args, publishCount } = makeBootstrapArgs({
    fetchResult: { status: "absent" },
    lastHead: 0,
    localNonEmpty: false,
  });
  const result = runBootstrap(args);
  assert.equal(result.action, "hold");
  assert.equal(publishCount(), 0, "empty local store must not trigger seed");
});

// Guard: fetch found → apply-remote returned, no publish.
// Mutation: removing the found branch drops the remote data.
test("runBootstrap: fetch found returns apply-remote with data and never calls publishFn", () => {
  const remoteData = {
    store: { version: 1, items: [] },
    createdAt: 100,
    eventId: "e1",
  };
  const { args, publishCount } = makeBootstrapArgs({
    fetchResult: {
      status: "found",
      data: remoteData,
      createdAt: 100,
      eventId: "e1",
    },
    lastHead: 0,
    localNonEmpty: true,
  });
  const result = runBootstrap(args);
  assert.equal(result.action, "apply-remote");
  assert.deepEqual(result.data, remoteData);
  assert.equal(
    publishCount(),
    0,
    "publishFn must not be called when remote was found",
  );
});
