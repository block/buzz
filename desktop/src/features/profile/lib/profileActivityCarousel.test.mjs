import assert from "node:assert/strict";
import test from "node:test";

import { resolveActivityChannelId } from "./profileActivityCarousel.ts";

test("preserves the selected channel when slides reorder", () => {
  assert.equal(
    resolveActivityChannelId(
      ["engineering", "general"],
      "general",
      "engineering",
    ),
    "general",
  );
});

test("falls back to the preferred channel when the selection disappears", () => {
  assert.equal(
    resolveActivityChannelId(["recent", "general"], "live", "general"),
    "general",
  );
});

test("falls back to the first slide when neither selection is available", () => {
  assert.equal(
    resolveActivityChannelId(["recent"], "live", "general"),
    "recent",
  );
  assert.equal(resolveActivityChannelId([], "live", "general"), null);
});
