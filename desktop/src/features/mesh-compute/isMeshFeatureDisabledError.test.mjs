import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { isMeshFeatureDisabledError } from "./isMeshFeatureDisabledError.ts";

describe("isMeshFeatureDisabledError", () => {
  it("matches the stub error and the clearer packaging copy", () => {
    assert.equal(
      isMeshFeatureDisabledError("mesh-llm feature not enabled"),
      true,
    );
    assert.equal(
      isMeshFeatureDisabledError(
        "Share Compute is not included in this build (mesh-llm feature off — typical for Linux/Windows release packages; macOS releases enable it).",
      ),
      true,
    );
  });

  it("ignores unrelated failures", () => {
    assert.equal(isMeshFeatureDisabledError(null), false);
    assert.equal(isMeshFeatureDisabledError("download failed"), false);
  });
});
