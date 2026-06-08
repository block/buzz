import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveEnabled } from "./resolveEnabled.ts";

describe("resolveEnabled", () => {
  describe("stable tier", () => {
    it("always returns true regardless of overrides", () => {
      assert.equal(resolveEnabled("stable", "doctor", {}), true);
      assert.equal(resolveEnabled("stable", "doctor", { doctor: false }), true);
    });
  });

  describe("preview tier", () => {
    it("returns false by default (no override)", () => {
      assert.equal(resolveEnabled("preview", "workflows", {}), false);
    });

    it("returns true when user opts in", () => {
      assert.equal(
        resolveEnabled("preview", "workflows", { workflows: true }),
        true,
      );
    });

    it("returns false when user explicitly opts out", () => {
      assert.equal(
        resolveEnabled("preview", "workflows", { workflows: false }),
        false,
      );
    });
  });

  describe("unknown tier", () => {
    it("returns false for unrecognized tier values", () => {
      // @ts-expect-error — testing invalid input
      assert.equal(resolveEnabled("unknown", "foo", {}), false);
    });
  });
});
