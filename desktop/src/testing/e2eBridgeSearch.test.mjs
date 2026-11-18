/**
 * Unit tests for the e2e bridge's mock `search_messages` predicate.
 *
 * The mock stands in for the relay in identity-less E2E runs, so its `since` /
 * `until` comparators must match the relay's inclusive NIP-01 bounds
 * (`crates/buzz-core/src/filter.rs` keeps `since <= created_at <= until`).
 * `before:YYYY-MM-DD` gets its exclusivity from `parseSearchOperators`, which
 * emits `localMidnight - 1`; a mock that also excluded its own bound would
 * drop the last second of the range that production returns.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { mockSearchHitMatches } from "./e2eBridgeSearch.ts";

const NO_FILTERS = { query: "", authorSet: null };

function hitAt(created_at) {
  return {
    event_id: "hit",
    content: "deploy notes",
    kind: 9,
    pubkey: "a".repeat(64),
    channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    channel_name: "general",
    created_at,
    score: 1,
  };
}

test("until is inclusive, so a before: bound keeps its own second", () => {
  const localMidnight = Math.floor(new Date(2024, 1, 1).getTime() / 1000);
  // What parseSearchOperators emits for `before:2024-02-01`.
  const until = localMidnight - 1;

  assert.equal(
    mockSearchHitMatches(hitAt(until), { ...NO_FILTERS, until }),
    true,
  );
  assert.equal(
    mockSearchHitMatches(hitAt(localMidnight), { ...NO_FILTERS, until }),
    false,
  );
});

test("since is inclusive and excludes only earlier timestamps", () => {
  const since = Math.floor(new Date(2024, 0, 15).getTime() / 1000);

  assert.equal(
    mockSearchHitMatches(hitAt(since), { ...NO_FILTERS, since }),
    true,
  );
  assert.equal(
    mockSearchHitMatches(hitAt(since - 1), { ...NO_FILTERS, since }),
    false,
  );
});

test("channel, author, and text filters still narrow results", () => {
  const hit = hitAt(1_700_000_000);

  assert.equal(
    mockSearchHitMatches(hit, { ...NO_FILTERS, channelId: "other-channel" }),
    false,
  );
  assert.equal(
    mockSearchHitMatches(hit, {
      ...NO_FILTERS,
      authorSet: new Set(["b".repeat(64)]),
    }),
    false,
  );
  assert.equal(
    mockSearchHitMatches(hit, { query: "zzz", authorSet: null }),
    false,
  );
  assert.equal(
    mockSearchHitMatches(hit, { query: "deploy", authorSet: null }),
    true,
  );
});
