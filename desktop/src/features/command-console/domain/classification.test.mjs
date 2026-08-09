import assert from "node:assert/strict";
import test from "node:test";

import {
  CLASSIFICATIONS,
  isClassification,
  resolveClassification,
} from "./classification.ts";

test("classification is exactly PUBLIC or OFFICIAL", () => {
  assert.deepEqual(CLASSIFICATIONS, ["PUBLIC", "OFFICIAL"]);
  assert.equal(isClassification("PUBLIC"), true);
  assert.equal(isClassification("OFFICIAL"), true);

  for (const rejected of [
    "OFFICIAL: Sensitive",
    "PROTECTED",
    "SECRET",
    "TOP SECRET",
    "UNCLASSIFIED",
    null,
  ]) {
    assert.equal(isClassification(rejected), false);
  }
});

test("classification defaults to OFFICIAL but preserves explicit PUBLIC", () => {
  assert.equal(resolveClassification(), "OFFICIAL");
  assert.equal(resolveClassification("PUBLIC"), "PUBLIC");
  assert.equal(resolveClassification("OFFICIAL"), "OFFICIAL");
});

test("PUBLIC composites stay PUBLIC only while every nested artefact is PUBLIC", () => {
  assert.equal(resolveClassification("PUBLIC", ["PUBLIC"]), "PUBLIC");
  assert.equal(
    resolveClassification("PUBLIC", ["PUBLIC", "OFFICIAL"]),
    "OFFICIAL",
  );
  assert.equal(
    resolveClassification("OFFICIAL", ["PUBLIC", "PUBLIC"]),
    "OFFICIAL",
  );
});
