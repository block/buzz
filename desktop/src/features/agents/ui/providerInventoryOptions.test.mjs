import assert from "node:assert/strict";
import test from "node:test";

import { withDiscoveredProviderOptions } from "./agentConfigOptions.tsx";

// ── withDiscoveredProviderOptions ────────────────────────────────────────────
//
// Harness-published provider inventories (goose) must extend the built-in
// provider rows without duplicating them and without losing the raw id, which
// is the value the harness reads off its provider env var.

const base = [
  { label: "Select a provider…", value: "__auto_provider__" },
  { label: "Anthropic", value: "anthropic" },
  { label: "OpenAI", value: "openai" },
];

function provider(id, name, extra = {}) {
  return {
    id,
    name,
    configured: false,
    defaultModel: null,
    acp: false,
    ...extra,
  };
}

test("appends discovered providers the built-in catalog does not list", () => {
  const options = withDiscoveredProviderOptions(base, [
    provider("anthropic", "Anthropic"),
    provider("ollama", "Ollama"),
    provider("together", "Together AI"),
  ]);

  // De-duplicated: `anthropic` already exists, so it is not appended again.
  assert.deepEqual(
    options.map((option) => option.value),
    ["__auto_provider__", "anthropic", "openai", "ollama", "together"],
  );
  // Discovered rows keep the harness's own order (not re-sorted).
  assert.deepEqual(
    options.slice(3).map((option) => option.label),
    ["Ollama", "Together AI"],
  );
});

test("keeps the raw provider id as the option value", () => {
  // The id is what lands in GOOSE_PROVIDER; a prettified value would select a
  // provider goose cannot resolve.
  const options = withDiscoveredProviderOptions(
    [],
    [provider("aws_bedrock", "Amazon Bedrock")],
  );

  assert.deepEqual(options, [
    { label: "Amazon Bedrock", value: "aws_bedrock" },
  ]);
});

test("marks configured providers and falls back to the id when unnamed", () => {
  const options = withDiscoveredProviderOptions(
    [],
    [
      provider("aws_bedrock", "Amazon Bedrock", { configured: true }),
      provider("lynkr", null),
    ],
  );

  assert.equal(options[0].description, "Configured");
  assert.equal(options[1].label, "lynkr");
  assert.equal(options[1].description, undefined);
});

test("drops blank ids and leaves the base list untouched when nothing is discovered", () => {
  // A blank id would render a nameless option that persists an empty provider.
  const options = withDiscoveredProviderOptions(base, [
    provider("   ", "Nameless"),
  ]);
  assert.deepEqual(options, base);

  const none = withDiscoveredProviderOptions(base, []);
  assert.deepEqual(none, base);
  // The caller's base array is never mutated in place.
  assert.equal(base.length, 3);
});
