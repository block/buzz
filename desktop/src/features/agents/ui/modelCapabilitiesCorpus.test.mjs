import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
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

test("corpus has exactly 103 executable vectors", () => {
  // Locks the vector count so a silent corpus edit can't quietly drop coverage;
  // must equal the gate in the Rust suite (model_capabilities.rs).
  assert.equal(executable.length, 103);
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
