import assert from "node:assert/strict";
import test from "node:test";

import {
  getFromHandleLookupQuery,
  isRelaySearchInputSafe,
  prepareRelaySearchInput,
  redactUnsafeRelaySearchInput,
  shouldEnableRelaySearch,
} from "./searchInputSafety.ts";

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const NSEC = "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5";

test("from lookup sends handles but never public keys or nsec-shaped text", () => {
  assert.equal(getFromHandleLookupQuery("@alice"), "alice");
  assert.equal(getFromHandleLookupQuery(HEX), "");
  assert.equal(getFromHandleLookupQuery(NPUB), "");

  for (const value of [
    NSEC,
    NSEC.toUpperCase(),
    `nostr:${NSEC}`,
    `NOSTR:${NSEC.toUpperCase()}`,
    "nSeC1truncated",
  ]) {
    assert.equal(getFromHandleLookupQuery(value), "", value);
  }
});

test("plain relay search blocks nsec-shaped tokens anywhere in the query", () => {
  for (const value of [
    NSEC,
    `find ${NSEC}`,
    `from:${NSEC}`,
    `find NOSTR:${NSEC.toUpperCase()}`,
    "find nSeC1malformed",
  ]) {
    assert.equal(isRelaySearchInputSafe(value), false, value);
  }
  assert.equal(isRelaySearchInputSafe("find npub1someone"), true);
  assert.equal(isRelaySearchInputSafe("find alice"), true);
});

test("the shared relay-query gate disables user and message search for secrets", () => {
  for (const input of [NSEC, `find ${NSEC}`, `from:${NSEC}`]) {
    assert.equal(
      shouldEnableRelaySearch({ enabled: true, hasSearchQuery: true, input }),
      false,
      input,
    );
  }
  assert.equal(
    shouldEnableRelaySearch({
      enabled: true,
      hasSearchQuery: true,
      input: "find alice",
    }),
    true,
  );
});

test("user-search preparation redacts secrets before request and cache keys", () => {
  for (const value of [
    NSEC,
    NSEC.toUpperCase(),
    `nostr:${NSEC}`,
    `NOSTR:${NSEC.toUpperCase()}`,
    `find nSeC1malformed`,
  ]) {
    assert.deepEqual(
      prepareRelaySearchInput(value),
      { normalizedQuery: "", safe: false },
      value,
    );
  }
  assert.deepEqual(prepareRelaySearchInput("  Alice  "), {
    normalizedQuery: "alice",
    safe: true,
  });
  assert.deepEqual(prepareRelaySearchInput(""), {
    normalizedQuery: "",
    safe: true,
  });
});

test("message-search preparation preserves prose case but redacts secrets", () => {
  assert.deepEqual(redactUnsafeRelaySearchInput("  Find Alice  "), {
    trimmedQuery: "Find Alice",
    safe: true,
  });
  for (const value of [
    NSEC,
    `find ${NSEC}`,
    `NOSTR:${NSEC.toUpperCase()}`,
    "nSeC1truncated",
  ]) {
    assert.deepEqual(
      redactUnsafeRelaySearchInput(value),
      { trimmedQuery: "", safe: false },
      value,
    );
  }
});
