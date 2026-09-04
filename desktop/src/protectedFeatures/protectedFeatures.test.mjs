import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveEnabled } from "../shared/features/resolveEnabled.ts";
import { protectedFeatureDefinitions as internalDefinitions } from "./internal.ts";
import { protectedFeatureDefinitions as publicDefinitions } from "./public.ts";

describe("protected feature build variants", () => {
  it("keeps protected definitions out of the OSS module", () => {
    assert.deepEqual(publicDefinitions, []);
  });

  it("adds Bestie as a default-off experiment only through the internal module", () => {
    assert.deepEqual(
      internalDefinitions.map((feature) => feature.id),
      ["bestie"],
    );
    const bestie = internalDefinitions[0];
    assert.ok(bestie);
    assert.equal(resolveEnabled(bestie.id, {}, bestie.defaultEnabled), false);
  });
});
