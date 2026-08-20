import assert from "node:assert/strict";
import test from "node:test";

import { fibreKindMeta } from "./fibreKinds.ts";

test("fibreKindMeta labels known kinds", () => {
  assert.equal(fibreKindMeta("blocker").label, "Blocker");
  assert.equal(fibreKindMeta("ask").label, "Ask");
  assert.equal(fibreKindMeta("blocker").color, "#E88170");
  assert.equal(fibreKindMeta("decision").color, "#A79AE8");
  assert.equal(fibreKindMeta("idea").color, "#5FBE94");
});

test("fibreKindMeta falls back for unknown kinds", () => {
  assert.equal(fibreKindMeta("mystery").label, "mystery");
});
