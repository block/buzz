import assert from "node:assert/strict";
import test from "node:test";

import { selectTimelineLoadingState } from "./timelineLoadingState.ts";

const settled = {
  isPending: false,
  isFetching: false,
  isPlaceholderData: false,
  dataLength: null,
};

test("pending first fetch with no cache is loading", () => {
  assert.equal(
    selectTimelineLoadingState({ ...settled, isPending: true }),
    true,
  );
});

test("stale placeholder while refetching is loading", () => {
  // Revisited within gcTime: placeholderData hands back a cached array while the
  // authoritative fetch runs. Must keep the skeleton up, not flash the intro.
  assert.equal(
    selectTimelineLoadingState({
      ...settled,
      isFetching: true,
      isPlaceholderData: true,
      dataLength: 0,
    }),
    true,
  );
});

test("subscription-seeded empty cache while fetching is loading", () => {
  // The live subscription's setQueryData seeds [] before history settles, so
  // data is defined but empty and a fetch is still in flight.
  assert.equal(
    selectTimelineLoadingState({
      ...settled,
      isFetching: true,
      isPlaceholderData: false,
      dataLength: 0,
    }),
    true,
  );
});

test("settled with rows is not loading", () => {
  assert.equal(
    selectTimelineLoadingState({ ...settled, dataLength: 5 }),
    false,
  );
});

test("settled and genuinely empty is not loading (real empty channel)", () => {
  assert.equal(
    selectTimelineLoadingState({ ...settled, dataLength: 0 }),
    false,
  );
});

test("background refetch of a populated channel is not loading", () => {
  // staleTime expiry can trigger a background refetch; with rows already present
  // we are loaded and must not re-show the skeleton.
  assert.equal(
    selectTimelineLoadingState({
      ...settled,
      isFetching: true,
      dataLength: 12,
    }),
    false,
  );
});
