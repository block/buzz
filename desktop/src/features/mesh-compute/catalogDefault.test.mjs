import assert from "node:assert/strict";
import { test } from "node:test";

import { defaultShareModelFromCatalog } from "./catalogDefault.ts";

function entry(name, overrides = {}) {
  return {
    name,
    description: name,
    size: "4B",
    fit: "comfortable",
    curated: false,
    recommended: false,
    installed: false,
    ...overrides,
  };
}

test("defaultShareModelFromCatalog prefers recommended usable entries", () => {
  assert.equal(
    defaultShareModelFromCatalog([
      entry("curated", { curated: true }),
      entry("recommended", { recommended: true }),
    ]),
    "recommended",
  );
});

test("defaultShareModelFromCatalog falls back to curated entries", () => {
  assert.equal(
    defaultShareModelFromCatalog([
      entry("plain"),
      entry("curated", { curated: true }),
    ]),
    "curated",
  );
});

test("defaultShareModelFromCatalog never picks too large entries", () => {
  assert.equal(
    defaultShareModelFromCatalog([
      entry("too-large-recommended", {
        recommended: true,
        fit: "too_large",
      }),
      entry("usable"),
    ]),
    "usable",
  );
});

test("defaultShareModelFromCatalog returns null when nothing is usable", () => {
  assert.equal(
    defaultShareModelFromCatalog([entry("too-large", { fit: "too_large" })]),
    null,
  );
});
