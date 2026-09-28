import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetBestiePopoverSizeForTests,
  BESTIE_POPOVER_DEFAULT_MAX_HEIGHT_PX,
  BESTIE_POPOVER_DEFAULT_WIDTH_PX,
  BESTIE_POPOVER_MAX_MAX_HEIGHT_PX,
  BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX,
  BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX,
  BESTIE_POPOVER_MIN_WIDTH_PX,
  BESTIE_POPOVER_SIZE_STORAGE_KEY,
  BESTIE_POPOVER_VIEWPORT_GUTTER_PX,
  BESTIE_POPOVER_VIEWPORT_HEIGHT_RATIO,
  bestiePopoverMinHeightPx,
  bestiePopoverViewportMaxHeightPx,
  setBestiePopoverSize,
} from "./bestiePopoverSizePreference.ts";

function memoryWindow(innerHeight) {
  const memory = new Map();
  globalThis.window = {
    innerHeight: innerHeight ?? undefined,
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };
  return memory;
}

test("defaults are taller and wider than the old fixed popover", () => {
  memoryWindow();
  __resetBestiePopoverSizeForTests();
  assert.equal(BESTIE_POPOVER_DEFAULT_WIDTH_PX, 448);
  assert.equal(BESTIE_POPOVER_DEFAULT_MAX_HEIGHT_PX, 720);
  assert.ok(BESTIE_POPOVER_DEFAULT_WIDTH_PX > 320);
  assert.ok(BESTIE_POPOVER_DEFAULT_MAX_HEIGHT_PX > 512);
});

test("setBestiePopoverSize clamps and persists", () => {
  memoryWindow();
  __resetBestiePopoverSizeForTests();
  const next = setBestiePopoverSize({ widthPx: 100, maxHeightPx: 9999 });
  assert.equal(next.widthPx, BESTIE_POPOVER_MIN_WIDTH_PX);
  assert.equal(next.maxHeightPx, 960);
  const again = setBestiePopoverSize({ widthPx: 500 });
  assert.equal(again.widthPx, 500);
  assert.equal(again.maxHeightPx, 960);
});

test("Lists-open min height is higher than chat-only min", () => {
  assert.equal(bestiePopoverMinHeightPx(true), BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX);
  assert.equal(
    bestiePopoverMinHeightPx(false),
    BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX,
  );
  assert.ok(
    BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX > BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX,
  );
});

test("setBestiePopoverSize respects Lists-open minHeightPx option", () => {
  memoryWindow();
  __resetBestiePopoverSizeForTests();
  const next = setBestiePopoverSize(
    { maxHeightPx: 200 },
    { minHeightPx: BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX },
  );
  assert.equal(next.maxHeightPx, BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX);
});

test("viewport max is 85% vh and stronger gutter, under hard cap", () => {
  assert.equal(BESTIE_POPOVER_VIEWPORT_GUTTER_PX, 96);
  assert.equal(BESTIE_POPOVER_VIEWPORT_HEIGHT_RATIO, 0.85);
  assert.equal(bestiePopoverViewportMaxHeightPx(1000), 850);
  assert.equal(
    bestiePopoverViewportMaxHeightPx(1000),
    Math.floor(1000 * BESTIE_POPOVER_VIEWPORT_HEIGHT_RATIO),
  );
  // Gutter wins when tighter than 85% (short laptop heights).
  const short = BESTIE_POPOVER_VIEWPORT_GUTTER_PX + 50;
  assert.equal(
    bestiePopoverViewportMaxHeightPx(short),
    Math.floor(short - BESTIE_POPOVER_VIEWPORT_GUTTER_PX),
  );
  // On a 800px viewport, gutter (704) beats 85% (680)? No — 85% is tighter.
  assert.equal(bestiePopoverViewportMaxHeightPx(800), 680);
  // Gutter beats ratio when vh is small enough that vh-gutter < 85% vh.
  // vh - 96 < 0.85*vh  =>  0.15*vh < 96  => vh < 640
  assert.equal(bestiePopoverViewportMaxHeightPx(600), 504);
  // Hard cap still applies on huge viewports.
  assert.equal(
    bestiePopoverViewportMaxHeightPx(5000),
    BESTIE_POPOVER_MAX_MAX_HEIGHT_PX,
  );
});

test("setBestiePopoverSize clamps to 85% viewport and persists", () => {
  const memory = memoryWindow(1000);
  __resetBestiePopoverSizeForTests();
  const next = setBestiePopoverSize(
    { maxHeightPx: 9999 },
    { viewportHeightPx: 1000 },
  );
  assert.equal(next.maxHeightPx, 850);
  const stored = JSON.parse(memory.get(BESTIE_POPOVER_SIZE_STORAGE_KEY));
  assert.equal(stored.maxHeightPx, 850);
});
