import assert from "node:assert/strict";
import test from "node:test";

import {
  parseSelfProfileCache,
  resolveAvatarDataUrl,
  shouldFetchAvatar,
  storageKey,
} from "./selfProfileStorage.ts";

test("storageKey: includes pubkey in result", () => {
  const key = storageKey("https://relay.example.com", "deadbeef");
  assert.ok(
    key.includes("deadbeef"),
    `expected key to contain pubkey, got: ${key}`,
  );
});

test("storageKey: strips trailing slash from relay URL", () => {
  const withSlash = storageKey("https://relay.example.com/", "abc");
  const withoutSlash = storageKey("https://relay.example.com", "abc");
  assert.equal(withSlash, withoutSlash);
});

test("storageKey: strips multiple trailing slashes", () => {
  const key = storageKey("https://relay.example.com///", "abc");
  assert.ok(
    !key.includes("///"),
    `expected trailing slashes stripped, got: ${key}`,
  );
});

test("storageKey: lowercases relay URL", () => {
  const upper = storageKey("HTTPS://Relay.Example.COM", "abc");
  const lower = storageKey("https://relay.example.com", "abc");
  assert.equal(upper, lower);
});

test("storageKey: trims whitespace from relay URL", () => {
  const padded = storageKey("  https://relay.example.com  ", "abc");
  const clean = storageKey("https://relay.example.com", "abc");
  assert.equal(padded, clean);
});

test("storageKey: different pubkeys produce different keys", () => {
  const a = storageKey("https://relay.example.com", "pubkey-a");
  const b = storageKey("https://relay.example.com", "pubkey-b");
  assert.notEqual(a, b);
});

test("parseSelfProfileCache: valid v1 payload round-trips", () => {
  const payload = {
    version: 1,
    displayName: "Alice",
    avatarUrl: "https://relay.example.com/media/abc.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/9j/4A==",
    updatedAt: 1700000000000,
  };
  const result = parseSelfProfileCache(payload);
  assert.deepEqual(result, payload);
});

test("parseSelfProfileCache: null fields are preserved", () => {
  const payload = {
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 0,
  };
  const result = parseSelfProfileCache(payload);
  assert.deepEqual(result, payload);
});

test("parseSelfProfileCache: wrong version returns null", () => {
  assert.equal(
    parseSelfProfileCache({
      version: 2,
      displayName: "Bob",
      avatarUrl: null,
      avatarDataUrl: null,
      updatedAt: 0,
    }),
    null,
  );
});

test("parseSelfProfileCache: missing version returns null", () => {
  assert.equal(
    parseSelfProfileCache({
      displayName: "Bob",
      avatarUrl: null,
      avatarDataUrl: null,
      updatedAt: 0,
    }),
    null,
  );
});

test("parseSelfProfileCache: null input returns null", () => {
  assert.equal(parseSelfProfileCache(null), null);
});

test("parseSelfProfileCache: string input returns null", () => {
  assert.equal(parseSelfProfileCache("garbage"), null);
});

test("parseSelfProfileCache: array input returns null", () => {
  assert.equal(parseSelfProfileCache([1, 2, 3]), null);
});

test("parseSelfProfileCache: number input returns null", () => {
  assert.equal(parseSelfProfileCache(42), null);
});

test("parseSelfProfileCache: non-string displayName is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: 123,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 0,
  });
  assert.notEqual(result, null);
  assert.equal(result?.displayName, null);
});

test("parseSelfProfileCache: non-finite updatedAt is coerced to 0", () => {
  const resultNaN = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: NaN,
  });
  assert.equal(resultNaN?.updatedAt, 0);

  const resultInf = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: Infinity,
  });
  assert.equal(resultInf?.updatedAt, 0);
});

test("parseSelfProfileCache: non-number updatedAt is coerced to 0", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: "yesterday",
  });
  assert.equal(result?.updatedAt, 0);
});

test("parseSelfProfileCache: valid data:image/ avatarDataUrl is preserved", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "data:image/png;base64,iVBORw0KGgo=",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, "data:image/png;base64,iVBORw0KGgo=");
});

test("parseSelfProfileCache: javascript: avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "javascript:alert(1)",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

test("parseSelfProfileCache: bare base64 avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "iVBORw0KGgoAAAANSUhEUgAAAAUA",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

test("parseSelfProfileCache: data:text/html avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "data:text/html,<h1>xss</h1>",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

/** Minimal SelfProfileCache fixture for policy helper tests. */
function makeCache(overrides = {}) {
  return {
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 0,
    ...overrides,
  };
}

test("shouldFetchAvatar: URL changed → fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/new.jpg", existing),
    true,
  );
});

test("shouldFetchAvatar: URL unchanged but avatarDataUrl null → fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: null,
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/same.jpg", existing),
    true,
  );
});

test("shouldFetchAvatar: URL unchanged and avatarDataUrl present → no fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/existing",
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/same.jpg", existing),
    false,
  );
});

test("shouldFetchAvatar: nextAvatarUrl null → no fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(shouldFetchAvatar(null, existing), false);
});

test("resolveAvatarDataUrl: nextAvatarUrl null → null", () => {
  const existing = makeCache({ avatarDataUrl: "data:image/jpeg;base64,/old" });
  assert.equal(resolveAvatarDataUrl(null, null, existing), null);
});

test("resolveAvatarDataUrl: fetch succeeded → use fetched value", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    resolveAvatarDataUrl(
      "https://relay.example.com/new.jpg",
      "data:image/jpeg;base64,/new",
      existing,
    ),
    "data:image/jpeg;base64,/new",
  );
});

test("resolveAvatarDataUrl: fetch failed, URL unchanged → preserve existing", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/existing",
  });
  assert.equal(
    resolveAvatarDataUrl("https://relay.example.com/same.jpg", null, existing),
    "data:image/jpeg;base64,/existing",
  );
});

test("resolveAvatarDataUrl: fetch failed, URL changed → null", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    resolveAvatarDataUrl("https://relay.example.com/new.jpg", null, existing),
    null,
  );
});
