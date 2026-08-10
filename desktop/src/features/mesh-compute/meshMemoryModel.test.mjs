import assert from "node:assert/strict";
import test from "node:test";

import { deriveMeshMemoryModel } from "./meshMemoryModel.ts";

const catalog = {
  gpuName: "Apple GPU",
  vramDisplay: "32 GB",
  vramGb: 32,
  recommended: "gemma",
  entries: [
    {
      name: "gemma",
      size: "8GB",
      sizeGb: 8,
      description: "",
      fit: "comfortable",
      installed: true,
      recommended: true,
      curated: true,
    },
  ],
};

test("prefers the runtime's reported model footprint", () => {
  const model = deriveMeshMemoryModel({
    view: {
      connected: true,
      selfCapacityGb: 40,
      selfModelSizeGb: 10,
      peers: [],
    },
    catalog,
    modelRef: "gemma",
  });
  assert.equal(model.label, "This computer · model uses 10 GB of 40 GB");
  assert.equal(model.usedSegments, 3);
});

test("falls back to the selected catalog model before runtime status arrives", () => {
  const model = deriveMeshMemoryModel({
    view: null,
    catalog,
    modelRef: "gemma",
  });
  assert.equal(model.label, "This computer · model uses 8 GB of 32 GB");
  assert.equal(model.usedSegments, 3);
});

test("unknown model size shows available memory without inventing use", () => {
  const model = deriveMeshMemoryModel({
    view: null,
    catalog,
    modelRef: "custom/model",
  });
  assert.equal(model.label, "This computer · 32 GB AI memory");
  assert.equal(model.usedSegments, 0);
});

test("every label names this computer, never the mesh", () => {
  // The meter sits directly under a radar field showing the whole community.
  // An unqualified "10 of 40 GB" would read as mesh-wide utilization, which is
  // a number nobody has: members publish their own capacity, not each other's
  // live allocation.
  const cases = [
    {
      view: {
        connected: true,
        selfCapacityGb: 40,
        selfModelSizeGb: 10,
        peers: [],
      },
      catalog,
      modelRef: "gemma",
    },
    { view: null, catalog, modelRef: "gemma" },
    { view: null, catalog, modelRef: "custom/model" },
    { view: null, catalog: null, modelRef: null },
  ];
  for (const input of cases) {
    const model = deriveMeshMemoryModel(input);
    assert.match(model.label, /this computer/i, model.label);
  }
});
