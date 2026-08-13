/**
 * Unit tests for selectArchiveEvictionKeys — the pure LRU-with-pins policy that
 * bounds the channel-scoped observer archive.
 *
 * The policy evicts at channel granularity (all agent entries for a channel go
 * together) while respecting pins (a pinned channel is never evicted). These
 * tests exercise the selection logic directly, with no React/Tauri runtime.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { selectArchiveEvictionKeys } from "./observerArchiveEviction.ts";

/** Build one archive entry meta. */
function meta(agent, channelId, accessSeq) {
  return { key: `${agent}:${channelId}`, channelId, accessSeq };
}

const NO_PINS = new Set();

describe("selectArchiveEvictionKeys", () => {
  it("test_empty_entries_returns_no_eviction", () => {
    assert.deepEqual(selectArchiveEvictionKeys([], NO_PINS, 8), []);
  });

  it("test_channel_count_below_cap_returns_no_eviction", () => {
    const entries = [
      meta("agentA", "chan-1", 1),
      meta("agentA", "chan-2", 2),
      meta("agentA", "chan-3", 3),
    ];
    assert.deepEqual(selectArchiveEvictionKeys(entries, NO_PINS, 8), []);
  });

  it("test_channel_count_exactly_at_cap_returns_no_eviction", () => {
    const entries = Array.from({ length: 8 }, (_, i) =>
      meta("agentA", `chan-${i}`, i),
    );
    assert.deepEqual(selectArchiveEvictionKeys(entries, NO_PINS, 8), []);
  });

  it("test_over_cap_evicts_least_recently_accessed_channels", () => {
    // 10 channels, cap 8 → evict the 2 lowest-recency (chan-0, chan-1).
    const entries = Array.from({ length: 10 }, (_, i) =>
      meta("agentA", `chan-${i}`, i),
    );
    const evicted = selectArchiveEvictionKeys(entries, NO_PINS, 8);
    assert.deepEqual(evicted.sort(), ["agentA:chan-0", "agentA:chan-1"].sort());
  });

  it("test_channel_recency_is_max_access_across_its_agent_entries", () => {
    // chan-old has two agent entries; its most-recent access (seq 100) keeps it
    // alive even though its other entry (seq 0) is the global minimum.
    const entries = [
      meta("agentA", "chan-old", 0),
      meta("agentB", "chan-old", 100),
      meta("agentA", "chan-1", 1),
      meta("agentA", "chan-2", 2),
    ];
    // cap 2 → keep the 2 most-recent channels (chan-old@100, chan-2@2),
    // evict chan-1@1.
    const evicted = selectArchiveEvictionKeys(entries, NO_PINS, 2);
    assert.deepEqual(evicted, ["agentA:chan-1"]);
  });

  it("test_evicting_a_channel_removes_all_its_agent_entries", () => {
    // chan-evict has 3 agent entries; all three keys must be returned together.
    const entries = [
      meta("agentA", "chan-evict", 0),
      meta("agentB", "chan-evict", 0),
      meta("agentC", "chan-evict", 0),
      meta("agentA", "chan-keep", 9),
    ];
    const evicted = selectArchiveEvictionKeys(entries, NO_PINS, 1);
    assert.deepEqual(
      evicted.sort(),
      ["agentA:chan-evict", "agentB:chan-evict", "agentC:chan-evict"].sort(),
    );
  });

  it("test_pinned_channel_is_never_evicted_even_when_least_recent", () => {
    // chan-pinned is the least-recently accessed but pinned → must survive;
    // chan-1 (next lowest) is evicted instead.
    const entries = [
      meta("agentA", "chan-pinned", 0),
      meta("agentA", "chan-1", 1),
      meta("agentA", "chan-2", 2),
      meta("agentA", "chan-3", 3),
    ];
    const evicted = selectArchiveEvictionKeys(
      entries,
      new Set(["chan-pinned"]),
      3,
    );
    assert.deepEqual(evicted, ["agentA:chan-1"]);
  });

  it("test_pins_exceeding_cap_evict_all_unpinned_but_keep_all_pinned", () => {
    // 3 pinned + 2 unpinned, cap 2. Pins alone exceed the cap, so the unpinned
    // budget is 0 → both unpinned channels evicted, all pinned retained.
    const entries = [
      meta("agentA", "pin-1", 10),
      meta("agentA", "pin-2", 11),
      meta("agentA", "pin-3", 12),
      meta("agentA", "free-1", 1),
      meta("agentA", "free-2", 2),
    ];
    const evicted = selectArchiveEvictionKeys(
      entries,
      new Set(["pin-1", "pin-2", "pin-3"]),
      2,
    );
    assert.deepEqual(evicted.sort(), ["agentA:free-1", "agentA:free-2"].sort());
  });

  it("test_pinned_channels_count_against_the_cap_for_unpinned_budget", () => {
    // 1 pinned + 3 unpinned, cap 2 → unpinned budget = 2 - 1 = 1. Keep the
    // single most-recent unpinned channel (free-3), evict free-1 and free-2.
    const entries = [
      meta("agentA", "pin-1", 5),
      meta("agentA", "free-1", 1),
      meta("agentA", "free-2", 2),
      meta("agentA", "free-3", 3),
    ];
    const evicted = selectArchiveEvictionKeys(entries, new Set(["pin-1"]), 2);
    assert.deepEqual(evicted.sort(), ["agentA:free-1", "agentA:free-2"].sort());
  });

  it("test_tie_broken_deterministically_by_channel_id", () => {
    // Two channels share recency 1 at the eviction boundary. The keep-sort
    // breaks ties by ascending channelId, so "chan-a" is retained and the
    // higher "chan-b" is evicted — deterministic regardless of input order.
    const entries = [
      meta("agentA", "chan-b", 1),
      meta("agentA", "chan-a", 1),
      meta("agentA", "chan-keep", 9),
    ];
    const evicted = selectArchiveEvictionKeys(entries, NO_PINS, 2);
    assert.deepEqual(evicted, ["agentA:chan-b"]);
  });

  it("test_zero_cap_evicts_all_unpinned_channels", () => {
    const entries = [meta("agentA", "chan-1", 1), meta("agentA", "chan-2", 2)];
    const evicted = selectArchiveEvictionKeys(entries, NO_PINS, 0);
    assert.deepEqual(evicted.sort(), ["agentA:chan-1", "agentA:chan-2"].sort());
  });
});
