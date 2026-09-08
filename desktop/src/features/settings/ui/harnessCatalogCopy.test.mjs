import assert from "node:assert/strict";
import test from "node:test";

import { harnessDescription } from "./harnessCatalogCopy.ts";

test("Pi catalog entry has its curated product description", () => {
  assert.equal(
    harnessDescription("pi"),
    "A minimal terminal coding harness, connected through the pi-acp adapter.",
  );
});
