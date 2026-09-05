/**
 * Unit tests for the Moderation nav resolver's cache key.
 *
 * The resolver's verdict depends on NIP-11 discovery, which is relay-specific.
 * The cache key MUST therefore include the connected relay origin so a
 * workspace switch busts the cached verdict instead of serving the previous
 * relay's answer for up to `staleTime`.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { moderationNavResolutionQueryKey } from "./hooks.ts";

test("query-key-scoped-to-relay: switching relay origin yields a distinct cache key", () => {
  const pubkey = "a".repeat(64);
  const keyA = moderationNavResolutionQueryKey(
    pubkey,
    "https://relay-a.example",
  );
  const keyB = moderationNavResolutionQueryKey(
    pubkey,
    "https://relay-b.example",
  );
  assert.notDeepEqual(
    keyA,
    keyB,
    "same pubkey on a different relay must not reuse the cached verdict",
  );
});

test("query-key-stable-per-relay: same pubkey and relay yields an equal key", () => {
  const pubkey = "b".repeat(64);
  const origin = "https://relay.example";
  assert.deepEqual(
    moderationNavResolutionQueryKey(pubkey, origin),
    moderationNavResolutionQueryKey(pubkey, origin),
    "a stable identity+relay pair must hit the same cache entry",
  );
});

test("query-key-distinguishes-unresolved-relay: a null origin is its own key dimension", () => {
  const pubkey = "c".repeat(64);
  assert.notDeepEqual(
    moderationNavResolutionQueryKey(pubkey, null),
    moderationNavResolutionQueryKey(pubkey, "https://relay.example"),
    "an unresolved relay must not share a cache entry with a resolved one",
  );
});
