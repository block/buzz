import assert from "node:assert/strict";
import test from "node:test";

import { cn } from "./cn.ts";

test("semantic type roles merge as font sizes rather than colors", () => {
  assert.equal(cn("text-sm", "text-body-medium"), "text-body-medium");
  assert.equal(cn("text-body-medium", "text-sm"), "text-sm");
  assert.equal(
    cn("text-content-subtle", "text-body-medium"),
    "text-content-subtle text-body-medium",
  );
});
