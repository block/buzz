import assert from "node:assert/strict";
import test from "node:test";

import { resolveShareComputeModel } from "./shareComputePreferences.ts";

test("the explicit Settings model wins over the hardware recommendation", () => {
  assert.equal(
    resolveShareComputeModel("Qwen3-8B-Q4_K_M", "Gemma-4-26B-Q4_K_M"),
    "Qwen3-8B-Q4_K_M",
  );
});

test("recommendation is only the empty first-run fallback", () => {
  assert.equal(
    resolveShareComputeModel("  ", "Gemma-4-26B-Q4_K_M"),
    "Gemma-4-26B-Q4_K_M",
  );
  assert.equal(resolveShareComputeModel("", null), null);
});
