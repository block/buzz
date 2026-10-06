import assert from "node:assert/strict";
import test from "node:test";
import { discoveredEffortValues, withEffortValue } from "./discoveredEffort.ts";
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
    ["default", "low", "medium", "high"],
  );
  assert.deepEqual(discoveredEffortValues({ options: [{ value: "ultra" }] }), [
    "ultra",
  ]);
});

test("effort edits replace case-insensitive native and legacy aliases", () => {
  const env = {
    buzz_acp_effort_level: "ultra",
    BUZZ_AGENT_THINKING_EFFORT: "high",
    KEEP: "value",
  };
  const keys = ["BUZZ_ACP_EFFORT_LEVEL", "BUZZ_AGENT_THINKING_EFFORT"];
  assert.deepEqual(withEffortValue(env, keys, "default"), {
    BUZZ_ACP_EFFORT_LEVEL: "default",
    KEEP: "value",
  });
  assert.deepEqual(withEffortValue(env, keys), { KEEP: "value" });
  assert.equal(env.buzz_acp_effort_level, "ultra");
});
