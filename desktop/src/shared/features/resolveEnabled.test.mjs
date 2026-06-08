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

  it("returns false when user explicitly opts out", () => {
    assert.equal(resolveEnabled("workflows", { workflows: false }), false);
  });

  it("ignores overrides for unrelated ids", () => {
    assert.equal(resolveEnabled("workflows", { pulse: true }), false);
  });
});
