import assert from "node:assert/strict";
import test from "node:test";

import { shouldUseComposerPaddingFallback } from "./useComposerHeightPadding.ts";

test("composer padding fallback follows initial measurement and growth", () => {
  assert.equal(
    shouldUseComposerPaddingFallback({
      nextPadding: 100,
      previousPadding: null,
    }),
    true,
  );
  assert.equal(
    shouldUseComposerPaddingFallback({
      nextPadding: 120,
      previousPadding: 100,
    }),
    true,
  );
});

test("composer padding fallback does not force-scroll on shrink", () => {
  assert.equal(
    shouldUseComposerPaddingFallback({
      nextPadding: 100,
      previousPadding: 120,
    }),
    false,
  );
});
