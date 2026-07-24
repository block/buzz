import assert from "node:assert/strict";
import test from "node:test";

import {
  CLASSIFICATIONS,
  highestClassification,
  isClassification,
  resolveClassification,
} from "./classification.ts";

test("classification exposes the supported ordered security labels", () => {
  assert.deepEqual(CLASSIFICATIONS, [
    "OFFICIAL",
    "OFFICIAL: Sensitive",
    "PROTECTED",
    "SECRET",
    "TOP SECRET",
  ]);

  for (const classification of CLASSIFICATIONS) {
    assert.equal(isClassification(classification), true);
  }
  assert.equal(isClassification("UNCLASSIFIED"), false);
  assert.equal(isClassification(null), false);
});

test("classification resolution defaults to OFFICIAL", () => {
  assert.equal(resolveClassification(), "OFFICIAL");
  assert.equal(highestClassification([]), "OFFICIAL");
});

test("classification resolution never silently downgrades an upstream artefact", () => {
  assert.equal(
    resolveClassification("OFFICIAL", ["PROTECTED", "OFFICIAL: Sensitive"]),
    "PROTECTED",
  );
  assert.equal(
    resolveClassification("SECRET", ["OFFICIAL", "PROTECTED"]),
    "SECRET",
  );
  assert.equal(
    highestClassification(["OFFICIAL", "TOP SECRET", "SECRET"]),
    "TOP SECRET",
  );
});
