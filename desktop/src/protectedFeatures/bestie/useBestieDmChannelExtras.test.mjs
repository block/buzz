import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

/**
 * Assistant is always-on; useFeatureEnabled("bestie") was removed from this
 * hook. A leftover bare `enabled` throws Safari ReferenceError
 * ("Can't find variable: enabled") when opening the Assistant nav/DM.
 */
describe("useBestieDmChannelExtras always-on gate", () => {
  it("does not reference an unbound feature-flag enabled", () => {
    const source = readFileSync(
      fileURLToPath(new URL("./useBestieDmChannelExtras.tsx", import.meta.url)),
      "utf8",
    );
    assert.equal(/\buseFeatureEnabled\b/.test(source), false);
    assert.equal(/\bif\s*\(\s*!enabled\b/.test(source), false);
    assert.equal(/^\s*enabled,\s*$/m.test(source), false);
  });
});
