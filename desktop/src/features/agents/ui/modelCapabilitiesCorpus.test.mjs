import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  databricksRegistryLabel,
  databricksRegistryLabelForRecords,
  ManifestSchema,
  resolveModelCapabilities,
} from "./modelCapabilities.ts";

// The normative corpus (`scripts/normative-corpus.json`) is the cross-language
// contract: the Rust interpreter's test suite runs the same executable vectors
// through its `resolve`, so a green run here proves the TS interpreter agrees
// with Rust axis-for-axis. Loaded by relative path — the corpus never passes
// through vite/tsc, so it needs no import alias.
const corpusUrl = new URL(
  "../../../../../scripts/normative-corpus.json",
  import.meta.url,
);
const corpus = JSON.parse(readFileSync(fileURLToPath(corpusUrl), "utf8"));

// A vector is executable iff it carries an `expect` block; section markers
// (`_group`) are skipped. Mirrors the Rust corpus filter.
const executable = corpus.filter((entry) => entry.expect != null);

test("corpus has exactly 140 executable vectors", () => {
  // Locks the vector count so a silent corpus edit can't quietly drop coverage;
  // must equal the gate in the Rust suite (model_capabilities.rs).
  assert.equal(executable.length, 140);
});

test("registry label aliases refuse an unprefixed query", () => {
  const records = [
    {
      provider: "databricks_v2",
      raw_model_id: "databricks-gpt-5",
      registry_label: "GPT-5",
    },
  ];
  assert.equal(
    databricksRegistryLabelForRecords("gpt-5", records, ["gpt-"]),
    null,
  );
});

test("UC model-family FQNs humanize onto their base records", () => {
  // #6918 follow-up: the shared UC-FQN (`system.ai.…`) and goose- alias forms
  // must resolve onto the same base databricks_v2 records via the new family
  // tokens. Mirrors the Rust `test_databricks_registry_label_lookup` coverage.
  const cases = [
    ["goose-claude-4-6-sonnet", "Claude Sonnet 4.6"],
    ["goose-claude-4-7-opus", "Claude Opus 4.7"],
    ["goose-kimi-2-7", "Kimi 2.7"],
    ["system.ai.gemini-3-5-flash", "Gemini 3.5 Flash"],
    ["system.ai.gemini-3-pro-image", "Gemini 3 Pro Image"],
    ["system.ai.deepseek-v4-pro-0813", "DeepSeek V4 Pro"],
    ["system.ai.glm-5-3-flash", "GLM-5.3 Flash"],
    ["system.ai.grok-4-6", "Grok 4.6"],
    ["system.ai.llama-4-maverick", "Llama 4 Maverick"],
    ["system.ai.meta-llama-3-3-70b-instruct", "Llama 3.3 70B Instruct"],
    ["system.ai.qwen3-next-80b-a3b-instruct", "Qwen3 Next 80B A3B Instruct"],
    ["system.ai.qwen35-122b-a10b", "Qwen3.5 122B A10B"],
    ["system.ai.gemma-3-12b", "Gemma 3 12B"],
    ["system.ai.inkling", "Inkling"],
    ["system.ai.deepseek-v4-flash-0731", "DeepSeek V4 Flash"],
    ["system.ai.glm-5-3", "GLM-5.3"],
    ["system.ai.glm-5-3-flash", "GLM-5.3 Flash"],
    ["system.ai.grok-4-6", "Grok 4.6"],
  ];
  for (const [fqn, label] of cases) {
    assert.equal(databricksRegistryLabel(fqn), label, `fqn=${fqn}`);
  }
});

test("registry label aliases refuse ambiguous stripped record keys", () => {
  const records = [
    {
      provider: "databricks_v2",
      raw_model_id: "databricks-gpt-5-6",
      registry_label: "Databricks GPT-5.6",
    },
    {
      provider: "databricks_v2",
      raw_model_id: "partner-gpt-5-6",
      registry_label: "Partner GPT-5.6",
    },
  ];
  assert.equal(
    databricksRegistryLabelForRecords("goose-gpt-5-6", records, ["gpt-"]),
    null,
  );
});

test("Unity Catalog FQNs use neutral concrete-unknown capabilities", () => {
  const fqn = resolveModelCapabilities("databricks_v2", "system.ai.kimi-k3");
  const fallback = resolveModelCapabilities(
    "databricks_v2",
    "some-unknown-xyz",
  );
  assert.deepEqual(fqn, fallback);
});

test("every executable corpus vector resolves to its expected six-axis profile", () => {
  for (const entry of executable) {
    const id = entry.id ?? "<no-id>";
    const got = resolveModelCapabilities(
      entry.provider ?? "",
      entry.raw_model_id ?? "",
    );
    const want = entry.expect;
    assert.equal(got.thinkingMode, want.thinking_mode, `${id}: thinkingMode`);
    assert.deepEqual(
      [...got.supportedEfforts],
      want.supported_efforts,
      `${id}: supportedEfforts`,
    );
    assert.equal(
      got.defaultEffort,
      want.default_effort,
      `${id}: defaultEffort`,
    );
    assert.equal(
      got.databricksV2WireRoute,
      want.databricks_v2_wire_route,
      `${id}: databricksV2WireRoute`,
    );
    assert.equal(
      got.normalizationPolicy,
      want.normalization_policy,
      `${id}: normalizationPolicy`,
    );
    assert.equal(
      got.registryLabel,
      want.registry_label,
      `${id}: registryLabel`,
    );
  }
});

test("registryLabel axis is exercised by at least 12 exact-record vectors", () => {
  // The registryLabel axis only populates on an exact-record hit; guard that
  // the corpus keeps covering it so a regression there can't pass unnoticed.
  const labeled = executable.filter((e) => e.expect.registry_label != null);
  assert.ok(
    labeled.length >= 12,
    `expected >=12 labeled vectors, got ${labeled.length}`,
  );
  for (const entry of labeled) {
    const got = resolveModelCapabilities(
      entry.provider ?? "",
      entry.raw_model_id ?? "",
    );
    assert.equal(
      got.registryLabel,
      entry.expect.registry_label,
      `${entry.id ?? "<no-id>"}: registryLabel`,
    );
  }
});

// The TS manifest schema mirrors Rust's `#[serde(deny_unknown_fields)]`: a
// misspelled key must fail in BOTH languages, not pass silently on desktop.
// Loaded by relative path — same rationale as the corpus above.
const manifestUrl = new URL(
  "../../../../../scripts/model-capabilities.json",
  import.meta.url,
);
const manifestJson = JSON.parse(
  readFileSync(fileURLToPath(manifestUrl), "utf8"),
);

test("ManifestSchema accepts the committed manifest verbatim", () => {
  // The strict schema must model every documented key the manifest actually
  // ships (`_comment`, `_provenance`, `source`, `_sources`, …); a green parse
  // here proves strictness didn't over-reach and break the real data.
  assert.doesNotThrow(() => ManifestSchema.parse(manifestJson));
});

test("ManifestSchema rejects an unknown top-level field", () => {
  // Mirrors Rust `deny_unknown_fields`: a typo'd root key is a hard error, not
  // an ignored no-op. Without `.strict()` this passed on desktop while Rust
  // failed — the exact drift this alignment closes.
  const withTypo = { ...manifestJson, faimly_rules: [] };
  assert.throws(() => ManifestSchema.parse(withTypo));
});

test("ManifestSchema rejects an unknown field inside an exact record", () => {
  // Strictness must reach nested objects too, not just the root — an exact
  // record with a stray key is where a hand-edit typo most plausibly lands.
  const mutated = structuredClone(manifestJson);
  mutated.exact_records[0].raw_modle_id = "typo";
  assert.throws(() => ManifestSchema.parse(mutated));
});
