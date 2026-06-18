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

import { resolveTimelineLoadingLatch } from "./timelineLoadingState.ts";

test("latch: loading on first entry to a channel", () => {
  const r = resolveTimelineLoadingLatch(null, "chan-a", true);
  assert.equal(r.isLoading, true);
  assert.equal(r.settledChannelId, null);
});

test("latch: settles when loadingNow turns false, recording the channel", () => {
  const r = resolveTimelineLoadingLatch(null, "chan-a", false);
  assert.equal(r.isLoading, false);
  assert.equal(r.settledChannelId, "chan-a");
});

test("latch: background refetch blip stays loaded once settled", () => {
  // settled for chan-a, then loadingNow blips true (isFetching) — must NOT
  // re-show the skeleton (the bounce Wes reported).
  const r = resolveTimelineLoadingLatch("chan-a", "chan-a", true);
  assert.equal(r.isLoading, false);
  assert.equal(r.settledChannelId, "chan-a");
});

test("latch: switching channels resets and loads the new one", () => {
  const r = resolveTimelineLoadingLatch("chan-a", "chan-b", true);
  assert.equal(r.isLoading, true);
  assert.equal(r.settledChannelId, "chan-a"); // not yet settled for b
});

test("latch: no active channel passes loadingNow through untouched", () => {
  assert.equal(
    resolveTimelineLoadingLatch("chan-a", null, true).isLoading,
    true,
  );
  assert.equal(
    resolveTimelineLoadingLatch("chan-a", null, false).isLoading,
    false,
  );
});
