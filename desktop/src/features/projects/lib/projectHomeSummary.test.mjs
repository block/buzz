import assert from "node:assert/strict";
import test from "node:test";

import { presentContextCount } from "./projectHomeSummary.ts";

test("presentContextCount hides empty values", () => {
  assert.equal(presentContextCount(undefined), undefined);
  assert.equal(presentContextCount(0), undefined);
  assert.equal(presentContextCount(3), 3);
});
