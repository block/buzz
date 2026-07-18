import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveEnabled } from "./resolveEnabled.ts";

describe("resolveEnabled (preview-only)", () => {
  it("returns false by default (no override)", () => {
    assert.equal(resolveEnabled("workflows", {}), false);
  });

  it("returns true when user opts in", () => {
    assert.equal(resolveEnabled("workflows", { workflows: true }), true);
  });

  it("uses an enabled manifest default when no override exists", () => {
    assert.equal(resolveEnabled("defaultOnFeature", {}, true), true);
  });

  it("lets an explicit opt-out override an enabled default", () => {
    assert.equal(
      resolveEnabled("defaultOnFeature", { defaultOnFeature: false }, true),
      false,
    );
  });

  it("returns false when user explicitly opts out", () => {
    assert.equal(resolveEnabled("workflows", { workflows: false }), false);
  });

  it("ignores overrides for unrelated ids", () => {
    assert.equal(resolveEnabled("workflows", { pulse: true }), false);
  });
});
