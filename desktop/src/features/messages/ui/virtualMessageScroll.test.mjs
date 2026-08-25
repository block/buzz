import assert from "node:assert/strict";
import test from "node:test";

import { getVirtualMessageScrollOptions } from "./virtualMessageScroll.ts";

test("maps smooth message navigation to Virtua's smooth option", () => {
  assert.deepEqual(getVirtualMessageScrollOptions("smooth"), {
    align: "center",
    smooth: true,
  });
  assert.deepEqual(getVirtualMessageScrollOptions("auto"), {
    align: "center",
    smooth: false,
  });
});
