import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveEnabled } from "../shared/features/resolveEnabled.ts";
import { protectedFeatureDefinitions as internalDefinitions } from "./internal.ts";
import { protectedFeatureDefinitions as publicDefinitions } from "./public.ts";

describe("protected feature build variants", () => {
  it("keeps protected definitions out of the OSS module", () => {
    assert.deepEqual(publicDefinitions, []);
  });

  it("adds Assistant as always-on only through the internal module", () => {
    const bestie = internalDefinitions.find((feature) => feature.id === "bestie");
    assert.ok(bestie);
    assert.equal(bestie.name, "Assistant");
    assert.equal(bestie.defaultEnabled, true);
    assert.equal(resolveEnabled(bestie.id, {}, bestie.defaultEnabled), true);
    assert.equal(
      publicDefinitions.some((feature) => feature.id === "bestie"),
      false,
    );
  });
});
