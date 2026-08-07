import assert from "node:assert/strict";
import { test } from "node:test";

import {
  describeExperimentalEmpty,
  describeSplitEntry,
  deriveMeshExperimentalModel,
} from "./experimentalModel.ts";

function entry(overrides = {}) {
  return {
    name: "Model-Q4",
    sizeLabel: "20GB",
    sizeGb: 20,
    layerCount: 64,
    packageRepo: "meshllm/Model-layers",
    fitsLocally: false,
    description: null,
    ...overrides,
  };
}

function catalog(overrides = {}) {
  return { entries: [], vramGb: 96, stale: false, ...overrides };
}

test("only models beyond this machine are experimental", () => {
  // A split-capable model that fits locally is not an experimental choice --
  // Qwen3-8B is 5GB and split-capable, and mesh-llm returns SingleNodeFit for
  // it before ever considering a split. Offering it here would tell someone a
  // 5GB model needs a cluster.
  const model = deriveMeshExperimentalModel(
    catalog({
      entries: [
        entry({ name: "Fits", sizeGb: 5, fitsLocally: true }),
        entry({ name: "Beyond", sizeGb: 300, fitsLocally: false }),
      ],
    }),
  );
  assert.equal(model.options.length, 1);
  assert.equal(model.options[0].entry.name, "Beyond");
});

test("a machine that fits everything is told so, not shown a failure", () => {
  // The good outcome must not read like an error.
  const model = deriveMeshExperimentalModel(
    catalog({ entries: [entry({ fitsLocally: true })] }),
  );
  assert.equal(model.options.length, 0);
  assert.equal(model.emptyReason, "machineFitsEverything");
  assert.match(describeExperimentalEmpty(model.emptyReason), /on its own/);
});

test("empty has distinct causes that never read alike", () => {
  // "Could not read the catalog", "the catalog has no packages", and "this
  // machine is big enough" are three different facts.
  assert.equal(deriveMeshExperimentalModel(undefined).emptyReason, "loading");
  assert.equal(
    deriveMeshExperimentalModel(null).emptyReason,
    "catalogUnavailable",
  );
  assert.equal(
    deriveMeshExperimentalModel(catalog()).emptyReason,
    "noPackages",
  );
  assert.equal(
    deriveMeshExperimentalModel(catalog({ stale: true })).emptyReason,
    "catalogUnavailable",
  );

  const messages = new Set(
    [
      "loading",
      "catalogUnavailable",
      "noPackages",
      "machineFitsEverything",
    ].map(describeExperimentalEmpty),
  );
  assert.equal(messages.size, 4, "each cause needs its own sentence");
});

test("detail states size and layers, and never invents either", () => {
  assert.equal(describeSplitEntry(entry()), "20 GB · 64 layers");
  // Two upstream entries publish "30 layers" where bytes belong. Layers are the
  // unit actually distributed, so they stand alone; a size is never synthesized
  // from a layer count, because they are not convertible.
  assert.equal(
    describeSplitEntry(
      entry({ sizeGb: null, sizeLabel: "30 layers", layerCount: 30 }),
    ),
    "30 layers",
  );
  // Neither figure published: fall back to the raw label rather than inventing.
  assert.equal(
    describeSplitEntry(
      entry({ sizeGb: null, sizeLabel: "unknown", layerCount: null }),
    ),
    "unknown",
  );
});

test("a large model with no published size still reaches the list", () => {
  // Unknown size is reported as not fitting, which is the honest direction:
  // offering the split path is recoverable, promising a local start is not.
  const model = deriveMeshExperimentalModel(
    catalog({
      entries: [
        entry({
          sizeGb: null,
          sizeLabel: "61 layers",
          layerCount: 61,
          fitsLocally: false,
        }),
      ],
    }),
  );
  assert.equal(model.options.length, 1);
  assert.equal(model.options[0].detail, "61 layers");
});
