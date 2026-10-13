import assert from "node:assert/strict";
import test from "node:test";

import { parsePinnedSitesSeparateSection } from "./separateSectionPreference.ts";

test("pinned sites separate section defaults off", () => {
  assert.equal(parsePinnedSitesSeparateSection(null), false);
  assert.equal(parsePinnedSitesSeparateSection("junk"), false);
  assert.equal(parsePinnedSitesSeparateSection("true"), true);
  assert.equal(parsePinnedSitesSeparateSection("false"), false);
});
