#!/usr/bin/env node
/**
 * Schema-negative validator tests for the manifest generator.
 *
 * Every validator rule in generate-model-capabilities.mjs must have a
 * failing-input test here — if validation is missing, these tests would
 * not catch the defect.
 *
 * Uses Node.js built-in test runner (node --test).
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, unlinkSync, mkdirSync, mkdtempSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { execFileSync, spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");
const manifestPath = join(repoRoot, "scripts", "model-capabilities.json");
const generatorPath = join(repoRoot, "scripts", "generate-model-capabilities.mjs");

/** Load the real manifest so we can mutate copies. */
const BASE_MANIFEST = JSON.parse(readFileSync(manifestPath, "utf8"));

/**
 * Run the generator with a mutated manifest, returning { exitCode, stderr, stdout }.
 * Writes the mutated manifest to a temp file and overrides the manifest path via env.
 */
function runGeneratorWithManifest(manifestOverride) {
  // Write mutated manifest to a temp path
  const tmpDir = mkdtempSync(join(tmpdir(), "test-validator-"));
  const tmpManifest = join(tmpDir, "model-capabilities.json");
  const tmpOutputDir = join(tmpDir, "out");
  mkdirSync(tmpOutputDir, { recursive: true });

  writeFileSync(tmpManifest, JSON.stringify(manifestOverride));

  // Run the generator via node, pointing MANIFEST_PATH env at the temp file
  // The generator reads from process.env.MANIFEST_PATH if set (we add this support)
  const result = spawnSync(
    process.execPath,
    [generatorPath, "--manifest-path", tmpManifest, "--output-dir", tmpOutputDir],
    {
      encoding: "utf8",
      env: { ...process.env },
    },
  );

  // Cleanup
  try { unlinkSync(tmpManifest); } catch {}

  return { exitCode: result.status ?? 1, stderr: result.stderr, stdout: result.stdout };
}

/**
 * Assert that the generator REJECTS the given manifest (exits non-zero).
 * The optional `expectedMessage` is checked in stderr if provided.
 */
function assertRejects(label, manifest, expectedMessage) {
  const { exitCode, stderr, stdout } = runGeneratorWithManifest(manifest);
  assert.notEqual(exitCode, 0, `${label}: expected generator to fail but it succeeded.\nstdout: ${stdout}\nstderr: ${stderr}`);
  if (expectedMessage) {
    const combined = stderr + stdout;
    assert.ok(
      combined.includes(expectedMessage),
      `${label}: expected error message "${expectedMessage}" not found.\nstdout: ${stdout}\nstderr: ${stderr}`,
    );
  }
}

/** Deep clone the base manifest and apply a mutator function. */
function mutate(fn) {
  const clone = JSON.parse(JSON.stringify(BASE_MANIFEST));
  fn(clone);
  return clone;
}

// ---------------------------------------------------------------------------
// Rule: invalid enum value in family_rule.thinking_mode
// ---------------------------------------------------------------------------
test("schema-negative: invalid thinking_mode in family rule is rejected", () => {
  assertRejects(
    "invalid thinking_mode",
    mutate((m) => {
      m.family_rules[0].thinking_mode = "invalid-mode";
    }),
    "thinking_mode",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid enum value in family_rule.databricks_v2_wire_route
// ---------------------------------------------------------------------------
test("schema-negative: invalid databricks_v2_wire_route in family rule is rejected", () => {
  assertRejects(
    "invalid databricks_v2_wire_route",
    mutate((m) => {
      m.family_rules[0].databricks_v2_wire_route = "chat-completions";
    }),
    "databricks_v2_wire_route",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid enum value in family_rule.supported_efforts[]
// ---------------------------------------------------------------------------
test("schema-negative: invalid effort value in family rule supported_efforts is rejected", () => {
  assertRejects(
    "invalid supported_efforts value",
    mutate((m) => {
      m.family_rules[0].supported_efforts = ["low", "ultra-high"];
    }),
    "supported_efforts",
  );
});

// ---------------------------------------------------------------------------
// Rule: empty supported_efforts array in family rule
// ---------------------------------------------------------------------------
test("schema-negative: empty supported_efforts in family rule is rejected", () => {
  assertRejects(
    "empty supported_efforts",
    mutate((m) => {
      m.family_rules[0].supported_efforts = [];
    }),
    "supported_efforts",
  );
});

// ---------------------------------------------------------------------------
// Rule: default_effort not in supported_efforts (non-null)
// ---------------------------------------------------------------------------
test("schema-negative: default_effort not in supported_efforts is rejected", () => {
  assertRejects(
    "default_effort not in supported_efforts",
    mutate((m) => {
      m.family_rules[0].supported_efforts = ["low", "medium"];
      m.family_rules[0].default_effort = "high"; // not in list
    }),
    "default_effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid normalization_policy in family rule
// ---------------------------------------------------------------------------
test("schema-negative: invalid normalization_policy in family rule is rejected", () => {
  assertRejects(
    "invalid normalization_policy",
    mutate((m) => {
      m.family_rules[0].normalization_policy = "pass-through-all";
    }),
    "normalization_policy",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid match_kind in family rule
// ---------------------------------------------------------------------------
test("schema-negative: invalid match_kind in family rule is rejected", () => {
  assertRejects(
    "invalid match_kind",
    mutate((m) => {
      m.family_rules[0].match_kind = "regex";
    }),
    "match_kind",
  );
});

// ---------------------------------------------------------------------------
// Rule: duplicate family rule id
// ---------------------------------------------------------------------------
test("schema-negative: duplicate family rule id is rejected", () => {
  assertRejects(
    "duplicate family rule id",
    mutate((m) => {
      m.family_rules.push({ ...m.family_rules[0] }); // duplicate id
    }),
    "duplicate",
  );
});

// ---------------------------------------------------------------------------
// Rule: registry_label on a family rule is rejected (display rule (a))
// ---------------------------------------------------------------------------
test("schema-negative: registry_label on a family rule is rejected", () => {
  assertRejects(
    "family rule registry_label forbidden",
    mutate((m) => {
      m.family_rules[0].registry_label = "Family Masquerade";
    }),
    "registry_label is not allowed on family rules",
  );
});

// ---------------------------------------------------------------------------
// Rule: duplicate exact_record (provider, raw_model_id) key
// ---------------------------------------------------------------------------
test("schema-negative: duplicate exact_record key is rejected", () => {
  assertRejects(
    "duplicate exact_record key",
    mutate((m) => {
      m.exact_records.push({ ...m.exact_records[0] }); // duplicate
    }),
    "duplicate",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record missing provider
// ---------------------------------------------------------------------------
test("schema-negative: exact_record missing provider is rejected", () => {
  assertRejects(
    "exact_record missing provider",
    mutate((m) => {
      m.exact_records.push({ raw_model_id: "some-model" });
    }),
    "provider",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record missing raw_model_id
// ---------------------------------------------------------------------------
test("schema-negative: exact_record missing raw_model_id is rejected", () => {
  assertRejects(
    "exact_record missing raw_model_id",
    mutate((m) => {
      m.exact_records.push({ provider: "databricks_v2" });
    }),
    "raw_model_id",
  );
});

// ---------------------------------------------------------------------------
// Rule: provider fallback record missing blank state
// ---------------------------------------------------------------------------
test("schema-negative: provider fallback missing blank state is rejected", () => {
  assertRejects(
    "provider fallback missing blank",
    mutate((m) => {
      delete m.provider_fallbacks.anthropic.blank;
    }),
    "blank",
  );
});

// ---------------------------------------------------------------------------
// Rule: provider fallback record missing concrete_unknown state
// ---------------------------------------------------------------------------
test("schema-negative: provider fallback missing concrete_unknown state is rejected", () => {
  assertRejects(
    "provider fallback missing concrete_unknown",
    mutate((m) => {
      delete m.provider_fallbacks.anthropic.concrete_unknown;
    }),
    "concrete_unknown",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid thinking_mode in provider fallback
// ---------------------------------------------------------------------------
test("schema-negative: invalid thinking_mode in provider fallback is rejected", () => {
  assertRejects(
    "invalid thinking_mode in fallback",
    mutate((m) => {
      m.provider_fallbacks.anthropic.blank.thinking_mode = "always-on";
    }),
    "thinking_mode",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid databricks_v2_wire_route in provider fallback
// ---------------------------------------------------------------------------
test("schema-negative: invalid wire_route in provider fallback is rejected", () => {
  assertRejects(
    "invalid wire_route in fallback",
    mutate((m) => {
      m.provider_fallbacks.anthropic.blank.databricks_v2_wire_route = "http-sse";
    }),
    "databricks_v2_wire_route",
  );
});

// ---------------------------------------------------------------------------
// Rule: invalid default_effort in provider fallback (not in supported_efforts)
// ---------------------------------------------------------------------------
test("schema-negative: default_effort not in supported_efforts in fallback is rejected", () => {
  assertRejects(
    "default_effort not in supported_efforts in fallback",
    mutate((m) => {
      m.provider_fallbacks.openai.blank.supported_efforts = ["low", "medium"];
      m.provider_fallbacks.openai.blank.default_effort = "high"; // not in list
    }),
    "default_effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: family rule missing id
// ---------------------------------------------------------------------------
test("schema-negative: family rule missing id is rejected", () => {
  assertRejects(
    "family rule missing id",
    mutate((m) => {
      m.family_rules.push({
        match_kind: "prefix",
        match_value: "test-",
        providers: ["anthropic"],
        match_priority: 1,
        thinking_mode: "none",
        supported_efforts: ["low"],
        default_effort: null,
        databricks_v2_wire_route: "not-applicable",
        normalization_policy: "none",
        // id deliberately omitted
      });
    }),
    "id",
  );
});

// ---------------------------------------------------------------------------
// Rule: duplicate exact_record key (same provider + raw_model_id) is rejected
// ---------------------------------------------------------------------------
test("schema-negative: duplicate exact_record key is rejected", () => {
  assertRejects(
    "duplicate exact_record key",
    mutate((m) => {
      // Add a second record for the same (provider, raw_model_id) key
      const existing = m.exact_records.find(
        (r) => r.raw_model_id === "databricks-gpt-5-4-mini",
      );
      m.exact_records.push({ ...existing });
    }),
    "duplicate",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record registry_label with empty label is rejected
// ---------------------------------------------------------------------------
test("schema-negative: exact_record registry_label with empty string is rejected", () => {
  assertRejects(
    "exact_record registry_label empty string",
    mutate((m) => {
      const rec = m.exact_records.find(
        (r) => r.raw_model_id === "databricks-gpt-5-4-mini",
      );
      rec.registry_label = "";
    }),
    "nonempty",
  );
});

// ---------------------------------------------------------------------------
// Rule: duplicate databricks_v2_known_models IDs
// ---------------------------------------------------------------------------
test("schema-negative: duplicate databricks_v2_known_models ID is rejected", () => {
  assertRejects(
    "duplicate known model ID",
    mutate((m) => {
      m.databricks_v2_known_models = ["databricks-gpt-5-5", "databricks-gpt-5-5"];
    }),
    "duplicate",
  );
});

// ---------------------------------------------------------------------------
// Rule: unsafe characters in match_value (family rule)
// ---------------------------------------------------------------------------
test("schema-negative: family rule match_value with unsafe chars is rejected", () => {
  assertRejects(
    "family rule match_value with backslash",
    mutate((m) => {
      // Inject a backslash into an existing rule's match_value — would break Rust string literal
      const rule = m.family_rules.find((r) => r.id === "anthropic-manual-budget-claude3");
      rule.match_value = "claude-3\\evil";
    }),
    "unsafe",
  );
});

// ---------------------------------------------------------------------------
// Rule: unsafe characters in known-model ID
// ---------------------------------------------------------------------------
test("schema-negative: databricks_v2_known_models ID with unsafe chars is rejected", () => {
  assertRejects(
    "known-model ID with double-quote",
    mutate((m) => {
      m.databricks_v2_known_models = ['databricks-gpt-5-5', 'bad"id'];
    }),
    "unsafe",
  );
});

// ---------------------------------------------------------------------------
// Rule: unsafe characters in exact_record registry_label
// ---------------------------------------------------------------------------
test("schema-negative: exact_record registry_label with unsafe chars is rejected", () => {
  assertRejects(
    "exact_record registry_label with backslash",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.registry_label = "GPT-5.4 Mini\\injected";
    }),
    "unsafe",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record supported_efforts_override must be non-empty
// ---------------------------------------------------------------------------
test("schema-negative: exact_record empty supported_efforts_override is rejected", () => {
  assertRejects(
    "exact_record empty supported_efforts_override",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.supported_efforts_override = [];
    }),
    "supported_efforts_override",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record supported_efforts_override with invalid enum value
// ---------------------------------------------------------------------------
test("schema-negative: exact_record supported_efforts_override with bogus enum is rejected", () => {
  assertRejects(
    "exact_record bogus effort enum",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.supported_efforts_override = ["ultra-high"];
    }),
    "supported_efforts_override",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record supported_efforts_override with duplicate effort
// ---------------------------------------------------------------------------
test("schema-negative: exact_record supported_efforts_override with duplicate effort is rejected", () => {
  assertRejects(
    "exact_record duplicate effort",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.supported_efforts_override = ["low", "low"];
    }),
    "duplicate",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record supported_efforts_override must follow canonical order
// ---------------------------------------------------------------------------
test("schema-negative: exact_record supported_efforts_override out of canonical order is rejected", () => {
  assertRejects(
    "exact_record efforts out of order",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      // Reverse order — [high, medium, low] is not canonical [low, medium, high]
      rec.supported_efforts_override = ["high", "medium", "low"];
    }),
    "canonical order",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record default_effort not in supported_efforts_override
// ---------------------------------------------------------------------------
test("schema-negative: exact_record default_effort not in supported_efforts_override is rejected", () => {
  assertRejects(
    "exact_record default_effort outside override",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.supported_efforts_override = ["low", "medium"];
      rec.default_effort = "high"; // not in override
    }),
    "default_effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record match_priority must be a non-negative integer
// ---------------------------------------------------------------------------
test("schema-negative: exact_record match_priority non-integer is rejected", () => {
  assertRejects(
    "exact_record match_priority non-integer",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.match_priority = "five";
    }),
    "match_priority",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record thinking_mode must be a valid enum
// ---------------------------------------------------------------------------
test("schema-negative: exact_record invalid thinking_mode is rejected", () => {
  assertRejects(
    "exact_record invalid thinking_mode",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.thinking_mode = "turbo-thinking";
    }),
    "thinking_mode",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record databricks_v2_wire_route must be a valid enum
// ---------------------------------------------------------------------------
test("schema-negative: exact_record invalid databricks_v2_wire_route is rejected", () => {
  assertRejects(
    "exact_record invalid wire_route",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.databricks_v2_wire_route = "http-sse";
    }),
    "databricks_v2_wire_route",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record normalization_policy must be a valid enum
// ---------------------------------------------------------------------------
test("schema-negative: exact_record invalid normalization_policy is rejected", () => {
  assertRejects(
    "exact_record invalid normalization_policy",
    mutate((m) => {
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      rec.normalization_policy = "pass-through-all";
    }),
    "normalization_policy",
  );
});

// ---------------------------------------------------------------------------
// Rule: family_rule match_priority must be a non-negative integer
// ---------------------------------------------------------------------------
test("schema-negative: family_rule match_priority string (injection vector) is rejected", () => {
  assertRejects(
    "family_rule string match_priority",
    mutate((m) => {
      // Inject a string that contains a compile_error! macro — must be rejected before emission
      m.family_rules[0].match_priority = 'compile_error!("THUFIR_INJECTED")';
    }),
    "match_priority",
  );
});

test("schema-negative: family_rule match_priority negative integer is rejected", () => {
  assertRejects(
    "family_rule negative match_priority",
    mutate((m) => {
      m.family_rules[0].match_priority = -1;
    }),
    "match_priority",
  );
});

test("schema-negative: family_rule match_priority float is rejected", () => {
  assertRejects(
    "family_rule float match_priority",
    mutate((m) => {
      m.family_rules[0].match_priority = 1.5;
    }),
    "match_priority",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record uppercase duplicate key is rejected (after lowercasing)
// ---------------------------------------------------------------------------
test("schema-negative: exact_record uppercase duplicate key is rejected", () => {
  assertRejects(
    "exact_record uppercase duplicate key",
    mutate((m) => {
      // Add an uppercase copy of an existing exact record key
      const existing = m.exact_records[0];
      m.exact_records.push({
        ...existing,
        provider: existing.provider.toUpperCase(),
        raw_model_id: existing.raw_model_id.toUpperCase(),
      });
    }),
    "duplicate exact_record key",
  );
});

// ---------------------------------------------------------------------------
// Rule: exact_record inherited default_effort not in inherited supported_efforts
// ---------------------------------------------------------------------------
test("schema-negative: exact_record inherited default_effort outside materialized supported_efforts is rejected", () => {
  assertRejects(
    "exact_record inherited default out of materialized efforts",
    mutate((m) => {
      // Override supported_efforts_override to a single value that excludes the family default.
      // For any exact record that inherits family default_effort, override efforts to exclude it.
      const rec = m.exact_records.find((r) => r.raw_model_id === "databricks-gpt-5-4-mini");
      // Family default for the gpt5-4 rule is "medium". Override to only ["low"] to force mismatch.
      rec.supported_efforts_override = ["low"];
      // No explicit default_effort — inherits "medium" from family, but "medium" is not in ["low"]
    }),
    "materialized default_effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: family_rule supported_efforts must have no duplicates
// ---------------------------------------------------------------------------
test("schema-negative: family_rule supported_efforts with duplicate is rejected", () => {
  assertRejects(
    "family_rule duplicate effort",
    mutate((m) => {
      m.family_rules[0].supported_efforts = ["low", "low", "medium"];
    }),
    "duplicate effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: family_rule supported_efforts must follow canonical order
// ---------------------------------------------------------------------------
test("schema-negative: family_rule supported_efforts out of canonical order is rejected", () => {
  assertRejects(
    "family_rule efforts out of order",
    mutate((m) => {
      m.family_rules[0].supported_efforts = ["high", "low", "medium"];
    }),
    "canonical order",
  );
});

// ---------------------------------------------------------------------------
// Rule: provider_fallback supported_efforts must have no duplicates
// ---------------------------------------------------------------------------
test("schema-negative: provider_fallback supported_efforts with duplicate is rejected", () => {
  assertRejects(
    "provider_fallback duplicate effort",
    mutate((m) => {
      const provider = Object.keys(m.provider_fallbacks)[0];
      m.provider_fallbacks[provider].blank.supported_efforts = ["low", "low", "medium"];
    }),
    "duplicate effort",
  );
});

// ---------------------------------------------------------------------------
// Rule: provider_fallback supported_efforts must follow canonical order
// ---------------------------------------------------------------------------
test("schema-negative: provider_fallback supported_efforts out of canonical order is rejected", () => {
  assertRejects(
    "provider_fallback efforts out of order",
    mutate((m) => {
      const provider = Object.keys(m.provider_fallbacks)[0];
      m.provider_fallbacks[provider].blank.supported_efforts = ["high", "low", "medium"];
    }),
    "canonical order",
  );
});

console.log("\nSchema-negative validator tests complete.");
