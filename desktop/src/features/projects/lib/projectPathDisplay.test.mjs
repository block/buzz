import assert from "node:assert/strict";
import test from "node:test";

import { shortenProjectPath } from "./projectPathDisplay.ts";

test("keeps short repository paths intact", () => {
  assert.equal(shortenProjectPath("repos/buzz"), "repos/buzz");
});

test("shortens long repository paths to their trailing segments", () => {
  assert.equal(
    shortenProjectPath("/Users/thomasp/sprout/projects/buzz"),
    "…/sprout/projects/buzz",
  );
});

test("normalizes Windows separators for display", () => {
  assert.equal(
    shortenProjectPath("C:\\Users\\thomasp\\repos\\buzz"),
    "…/thomasp/repos/buzz",
  );
});
