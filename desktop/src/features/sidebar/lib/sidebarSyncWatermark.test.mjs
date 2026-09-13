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

// ── readWatermark + advanceWatermark ─────────────────────────────────────────

test("readWatermark: returns 0 for missing/zero; reads stored value; scopes by blobType and relay", () =>
  withFreshStorage((ls) => {
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`, "0");
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
    ls.setItem(
      `buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`,
      "1700000000",
    );
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
    // Scoped by blobType.
    ls.setItem(`buzz-sync-watermark.v1:sort:pk:${RELAY_ENCODED}`, "200");
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
    assert.equal(readWatermark("pk", "sort", RELAY), 200);
  }));

test("advanceWatermark: monotonic write/advance/no-regress; relay isolation; URL normalization", () =>
  withFreshStorage(() => {
    // Write, read back.
    advanceWatermark("pk", "sections", RELAY, 100);
    assert.equal(readWatermark("pk", "sections", RELAY), 100);
    // Advances on higher value; does not regress on lower or equal.
    advanceWatermark("pk", "sections", RELAY, 200);
    assert.equal(readWatermark("pk", "sections", RELAY), 200);
    advanceWatermark("pk", "sections", RELAY, 150);
    advanceWatermark("pk", "sections", RELAY, 200);
    assert.equal(readWatermark("pk", "sections", RELAY), 200);
    // Relay isolation: relay-B starts at 0 despite relay-A.
    advanceWatermark("pk", "sections", "wss://a.relay.test", 1700000100);
    assert.equal(readWatermark("pk", "sections", "wss://b.relay.test"), 0);
    advanceWatermark("pk", "sections", "wss://b.relay.test", 1700000200);
    assert.equal(
      readWatermark("pk", "sections", "wss://a.relay.test"),
      1700000100,
    );
    // URL normalization: trailing slash and case are normalized.
    advanceWatermark("pk2", "sections", "WSS://Relay.Example.Com/", 999);
    assert.equal(
      readWatermark("pk2", "sections", "wss://relay.example.com"),
      999,
    );
    assert.equal(
      readWatermark("pk2", "sections", "WSS://Relay.Example.Com/"),
      999,
    );
  }));

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

for (const {
  title,
  fetchResult,
  lastHead,
  localNonEmpty,
  action,
  wantPublish,
} of [
  {
    title: "fetch failed → hold, no publish",
    fetchResult: { status: "failed" },
    lastHead: 0,
    localNonEmpty: true,
    action: "hold",
    wantPublish: 0,
  },
  {
    title: "absent + prior head → hold, no publish (stale-dev-build)",
    fetchResult: { status: "absent" },
    lastHead: 1700000000,
    localNonEmpty: true,
    action: "hold",
    wantPublish: 0,
  },
  {
    title: "absent + head=0 + non-empty → seed once, hold",
    fetchResult: { status: "absent" },
    lastHead: 0,
    localNonEmpty: true,
    action: "hold",
    wantPublish: 1,
  },
  {
    title: "absent + head=0 + empty → no seed, hold",
    fetchResult: { status: "absent" },
    lastHead: 0,
    localNonEmpty: false,
    action: "hold",
    wantPublish: 0,
  },
]) {
  test(`runBootstrap: ${title}`, () => {
    const { args, publishCount } = makeBootstrapArgs({
      fetchResult,
      lastHead,
      localNonEmpty,
    });
    assert.equal(runBootstrap(args).action, action);
    assert.equal(publishCount(), wantPublish);
  });
}

// fetch found → apply-remote, no publish.
test("runBootstrap: fetch found returns apply-remote with data, no publish", () => {
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
  assert.equal(publishCount(), 0);
});
