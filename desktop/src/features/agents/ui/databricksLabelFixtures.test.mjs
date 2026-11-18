import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  databricksRegistryLabel,
  generateDatabricksLabel,
} from "./modelCapabilities.ts";

// TS half of the shared display-label contract; the Rust suite
// (databricks_label_grammar.rs) replays the same fixture file.
const readScript = (name) =>
  JSON.parse(
    readFileSync(
      fileURLToPath(new URL(`../../../../../scripts/${name}`, import.meta.url)),
      "utf8",
    ),
  );
const fixtures = readScript("databricks-label-fixtures.json");
const records = readScript("model-capabilities.json").exact_records.filter(
  (rec) => rec.provider === "databricks_v2",
);
const hasExactRecord = (id) =>
  records.some((rec) => rec.raw_model_id.toLowerCase() === id.toLowerCase());

test("shared fixtures replay through the label helper", () => {
  for (const { id, tier, label } of fixtures.lookup) {
    assert.equal(
      databricksRegistryLabel(id),
      label,
      `id=${JSON.stringify(id)}`,
    );
    assert.equal(hasExactRecord(id), tier === "exact", `tier=${tier} id=${id}`);
    const curated = databricksRegistryLabel(id, { generate: false });
    if (tier === "exact" || tier === "alias") {
      assert.equal(curated, label, `id=${id}`);
    } else if (tier === "generated") {
      assert.equal(curated, null, `generated fixture is masked: ${id}`);
      assert.equal(generateDatabricksLabel(id), label, `id=${id}`);
    } else {
      assert.equal(tier, "raw", `unknown tier ${tier} for ${id}`);
      assert.equal(label, null, `raw fixture has a label: ${id}`);
      assert.equal(curated, null, `raw fixture is curated: ${id}`);
    }
  }
});

test("grammar reproduces every curated label except curator-only ones", () => {
  const misses = records
    .filter(
      (rec) => generateDatabricksLabel(rec.raw_model_id) !== rec.registry_label,
    )
    .map((rec) => rec.raw_model_id);
  assert.deepEqual(misses, fixtures.curator_only_labels);
});
