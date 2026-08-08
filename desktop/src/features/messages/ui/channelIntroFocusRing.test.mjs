/**
 * Regression guard for #2392 — focus rings on channel intro action cards must
 * not be clipped by the horizontal scroll container.
 *
 * History: the action-card row in ChannelIntroBlock.tsx used
 * `overflow-x-auto pb-1` (padding-bottom only). Setting `overflow-x` without
 * `overflow-y` makes browsers compute `overflow-y: auto` too, which turns the
 * container into a clipping scroll context. The 4px `pb-1` was enough cover
 * for the ring on the bottom edge only — the ring was still cut off on top,
 * left, and right of every card.
 *
 * Fix: replace `pb-1` with a full-axis `-mx-1 p-1`, giving the focus ring a
 * 4px room on every side while the negative horizontal margin cancels the
 * visual shift in the row's alignment.
 *
 * This test reads ChannelIntroBlock.tsx and asserts the scroll container
 * carries `-mx-1 p-1` (full-axis ring room with the cancel-out margin), not
 * the one-sided `pb-1` layout that produced the clipped focus ring.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "ChannelIntroBlock.tsx"), "utf8");

test("channel intro action-card scroller gives the focus ring room on all sides", () => {
  const scrollerPattern = /<div className="mt-4 -mx-1 flex max-w-full flex-nowrap gap-3 overflow-x-auto p-1">/;
  assert.match(
    source,
    scrollerPattern,
    "scroll container must use `-mx-1 p-1` so `focus-visible:ring-2` has " +
      "4px of room on all four sides; see #2392",
  );
});

test("channel intro does not regress to a one-sided padding scroller", () => {
  // Catch a refactor that drops the negative-margin trick and goes back to a
  // one-sided `pb-1` padding that only protects the bottom of the ring.
  const regressedPattern = /overflow-x-auto pb-1/;
  assert.doesNotMatch(
    source,
    regressedPattern,
    "scroller regressed to `overflow-x-auto pb-1` — focus rings on the " +
      "action cards will be clipped on top/left/right (see #2392)",
  );
});
