import assert from "node:assert/strict";
import test from "node:test";
import { discoveredEffortValues } from "./discoveredEffort.ts";
test("effort vocabulary follows each adapter/model and handles grouped options", () => {
  assert.deepEqual(discoveredEffortValues(undefined), []);
  assert.deepEqual(
    discoveredEffortValues({
      options: [
        { value: "default" },
        { value: "low" },
        { options: [{ value: "medium" }, { value: "high" }, { value: "low" }] },
      ],
    }),
    ["low", "medium", "high"],
  );
  assert.deepEqual(discoveredEffortValues({ options: [{ value: "ultra" }] }), [
    "ultra",
  ]);
});
