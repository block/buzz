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

import { mockSearchHitMatches } from "./e2eBridge.ts";

const NO_FILTERS = { query: "", authorSet: null };

function hitAt(created_at) {
  return {
    event_id: "hit",
    content: "deploy notes",
    kind: 9,
    pubkey: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
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
      authorSet: new Set([
        "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
      ]),
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

test("web-search OR arms match canonical and legacy entity-link tokens", () => {
  const ownerNpub =
    "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
  const ownerHex =
    "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
  const query = `${ownerNpub} buzz-world OR ${ownerHex} buzz-world`;
  const canonical = {
    ...hitAt(1_700_000_000),
    content: `See buzz://repo?owner=${ownerNpub}&d=buzz-world`,
  };
  const legacy = {
    ...canonical,
    content: `See buzz://repo?owner=${ownerHex}&d=buzz-world`,
  };

  assert.equal(
    mockSearchHitMatches(canonical, {
      query,
      searchMode: "fullText",
      authorSet: null,
    }),
    true,
  );
  assert.equal(
    mockSearchHitMatches(legacy, {
      query,
      searchMode: "fullText",
      authorSet: null,
    }),
    true,
  );
  assert.equal(
    mockSearchHitMatches(
      {
        ...canonical,
        content: canonical.content.replace("buzz-world", "other"),
      },
      { query, searchMode: "fullText", authorSet: null },
    ),
    false,
  );
  assert.equal(
    mockSearchHitMatches(canonical, { query, authorSet: null }),
    false,
    "prefix mode must not silently interpret full-text OR syntax",
  );
});
