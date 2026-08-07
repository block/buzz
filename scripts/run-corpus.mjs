#!/usr/bin/env node
/**
 * Normative corpus runner — validates the generated TS interpreter against
 * scripts/normative-corpus.json.
 *
 * This is the JS side of the two-interpreter corpus check. The runner imports
 * resolveModelCapabilities() directly from the generated TypeScript module via
 * Node's --experimental-strip-types flag. The Rust side lives in
 * crates/buzz-agent/src/generated_model_capabilities.rs (shared corpus harness).
 *
 * Usage: node --experimental-strip-types scripts/run-corpus.mjs [--verbose]
 * Exits 0 on all pass, 1 on any failure.
 */

import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");
const VERBOSE = process.argv.includes("--verbose");

// Import the generated TypeScript interpreter directly.
// Node 22+ --experimental-strip-types strips type annotations at load time; no build step needed.
const { resolveModelCapabilities } = await import(
  join(repoRoot, "desktop", "src", "features", "agents", "ui", "modelCapabilities.ts")
);

const corpus = JSON.parse(
  readFileSync(join(repoRoot, "scripts", "normative-corpus.json"), "utf8"),
);

// ----- Provider alias canonicalization -----
// Mirrors production canonicalizeProvider() in desktop/src/features/agents/lib/formatAgentModelLabel.ts.
// Applied before every generated lookup so alias vectors (e.g. "openai-compat") pass both interpreters.
const PROVIDER_ALIASES = new Map([
  ["databricks-v2", "databricks_v2"],
  ["openai-compat", "openai"],
]);

function canonicalizeProvider(provider) {
  const normalized = (provider ?? "").trim().toLowerCase();
  return PROVIDER_ALIASES.get(normalized) ?? normalized;
}

// ----- Run corpus -----

let passed = 0;
let failed = 0;

const REQUIRED_AXES = [
  "thinking_mode",
  "supported_efforts",
  "default_effort",
  "databricks_v2_wire_route",
  "normalization_policy",
  "registry_label",
];

for (const entry of corpus) {
  // Skip group header entries
  if (entry._group) continue;
  if (!entry.expect) continue;

  // Require all 6 axes on every executable vector — sparse vectors hide divergences.
  const missingAxes = REQUIRED_AXES.filter((ax) => !(ax in entry.expect));
  if (missingAxes.length > 0) {
    failed++;
    console.error(`  FAIL  ${entry.id}: sparse vector — missing axes: ${missingAxes.join(", ")}`);
    console.error(`         All six axes are required: ${REQUIRED_AXES.join(", ")}`);
    continue;
  }

  // resolveModelCapabilities returns camelCase keys (registryLabel, thinkingMode, etc.)
  const result = resolveModelCapabilities(canonicalizeProvider(entry.provider), entry.raw_model_id);
  const expect = entry.expect;

  const failures = [];

  for (const [key, expectedVal] of Object.entries(expect)) {
    // Corpus uses snake_case; generated TS uses camelCase — convert for lookup.
    const camelKey = key.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
    const actualVal = camelKey in result ? result[camelKey] : result[key];

    if (Array.isArray(expectedVal)) {
      // Order-sensitive comparison for effort arrays
      const actualArr = Array.isArray(actualVal) ? actualVal : [];
      if (JSON.stringify(actualArr) !== JSON.stringify(expectedVal)) {
        failures.push(
          `  ${key}: expected [${expectedVal.join(", ")}] got [${actualArr.join(", ")}]`,
        );
      }
    } else {
      if (actualVal !== expectedVal) {
        failures.push(`  ${key}: expected ${JSON.stringify(expectedVal)} got ${JSON.stringify(actualVal)}`);
      }
    }
  }

  if (failures.length === 0) {
    passed++;
    if (VERBOSE) {
      console.log(`  PASS  ${entry.id}`);
    }
  } else {
    failed++;
    console.error(`  FAIL  ${entry.id} (${entry.provider} / "${entry.raw_model_id}")`);
    for (const f of failures) console.error(f);
    if (entry._note) console.error(`  note: ${entry._note}`);
  }
}

console.log(`\nCorpus: ${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
