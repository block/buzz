/**
 * Unit tests for selectMessageWindowEvictionUnits — the pure LRU-with-pins
 * policy that bounds the message query-cache windows. Exercises the selection
 * logic directly, with no React/React-Query runtime.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { selectMessageWindowEvictionUnits } from "./messageWindowEviction.ts";

/** Build one eviction unit. */
function unit(unitId, recency, pinned = false) {
  return { unitId, recency, pinned };
}

describe("selectMessageWindowEvictionUnits", () => {
  it("test_empty_units_returns_no_eviction", () => {
    assert.deepEqual(selectMessageWindowEvictionUnits([], 8), []);
  });

  it("test_count_below_cap_returns_no_eviction", () => {
    const units = [unit("a", 1), unit("b", 2), unit("c", 3)];
    assert.deepEqual(selectMessageWindowEvictionUnits(units, 8), []);
  });

  it("test_count_exactly_at_cap_returns_no_eviction", () => {
    const units = Array.from({ length: 8 }, (_, i) => unit(`u-${i}`, i));
    assert.deepEqual(selectMessageWindowEvictionUnits(units, 8), []);
  });

  it("test_over_cap_evicts_least_recently_updated_units", () => {
    // 10 units, cap 8 → evict the 2 lowest-recency (u-0, u-1).
    const units = Array.from({ length: 10 }, (_, i) => unit(`u-${i}`, i));
    const evicted = selectMessageWindowEvictionUnits(units, 8);
    assert.deepEqual(evicted.sort(), ["u-0", "u-1"].sort());
  });

  it("test_pinned_unit_is_never_evicted_even_when_least_recent", () => {
    // u-pin is least recent but pinned → survives; u-1 evicted instead.
    const units = [
      unit("u-pin", 0, true),
      unit("u-1", 1),
      unit("u-2", 2),
      unit("u-3", 3),
    ];
    const evicted = selectMessageWindowEvictionUnits(units, 3);
    assert.deepEqual(evicted, ["u-1"]);
  });

  it("test_pins_exceeding_cap_evict_all_unpinned_but_keep_all_pinned", () => {
    // 3 pinned + 2 unpinned, cap 2. Pins alone exceed cap → unpinned budget 0.
    const units = [
      unit("pin-1", 10, true),
      unit("pin-2", 11, true),
      unit("pin-3", 12, true),
      unit("free-1", 1),
      unit("free-2", 2),
    ];
    const evicted = selectMessageWindowEvictionUnits(units, 2);
    assert.deepEqual(evicted.sort(), ["free-1", "free-2"].sort());
  });

  it("test_pinned_units_count_against_cap_for_unpinned_budget", () => {
    // 1 pinned + 3 unpinned, cap 2 → unpinned budget = 1. Keep newest unpinned.
    const units = [
      unit("pin-1", 5, true),
      unit("free-1", 1),
      unit("free-2", 2),
      unit("free-3", 3),
    ];
    const evicted = selectMessageWindowEvictionUnits(units, 2);
    assert.deepEqual(evicted.sort(), ["free-1", "free-2"].sort());
  });

  it("test_tie_broken_deterministically_by_unit_id", () => {
    // unit-a and unit-b share recency 1 at the boundary; keep-sort breaks ties
    // by ascending unitId, so unit-a is retained and unit-b evicted.
    const units = [unit("unit-b", 1), unit("unit-a", 1), unit("unit-keep", 9)];
    const evicted = selectMessageWindowEvictionUnits(units, 2);
    assert.deepEqual(evicted, ["unit-b"]);
  });

  it("test_zero_cap_evicts_all_unpinned_units", () => {
    const units = [unit("a", 1), unit("b", 2), unit("pin", 3, true)];
    const evicted = selectMessageWindowEvictionUnits(units, 0);
    assert.deepEqual(evicted.sort(), ["a", "b"].sort());
  });
});
