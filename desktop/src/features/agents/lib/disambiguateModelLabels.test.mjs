import assert from "node:assert/strict";
import test from "node:test";

import { disambiguateModelLabels } from "./formatAgentModelLabel.ts";

const labels = (rows, provider = "databricks_v2") =>
  disambiguateModelLabels(rows, provider).map((row) => row.label);

test("namespace collision gets the catalog.schema suffix", () => {
  assert.deepEqual(
    labels([
      { id: "system.ai.gpt-6-astra", label: "GPT-6 Astra" },
      { id: "sandbox.public.gpt-6-astra", label: "GPT-6 Astra" },
    ]),
    ["GPT-6 Astra (system.ai)", "GPT-6 Astra (sandbox.public)"],
  );
});

test("workspace wrapper collision gets the wrapper suffix", () => {
  assert.deepEqual(
    labels([
      { id: "databricks-gpt-5-5", label: "GPT-5.5" },
      { id: "goose-gpt-5-5", label: "GPT-5.5" },
      { id: "kgoose-gpt-5-5", label: "GPT-5.5" },
    ]),
    ["GPT-5.5 (databricks)", "GPT-5.5 (goose)", "GPT-5.5 (kgoose)"],
  );
});

test("rows still colliding after the source suffix get the full id", () => {
  assert.deepEqual(
    labels([
      { id: "system.ai.gemini-3-7-flash", label: "Gemini 3.7 Flash" },
      { id: "system.ai.gemini-3-7-flash-0925", label: "Gemini 3.7 Flash" },
      { id: "goose-gemini-3-7-flash", label: "Gemini 3.7 Flash" },
    ]),
    [
      "Gemini 3.7 Flash (system.ai.gemini-3-7-flash)",
      "Gemini 3.7 Flash (system.ai.gemini-3-7-flash-0925)",
      "Gemini 3.7 Flash (goose)",
    ],
  );
});

test("unique, raw-id, and default rows are unchanged", () => {
  assert.deepEqual(
    labels([
      { id: "", label: "Default model" },
      { id: "system.ai.gpt-6-sol", label: "GPT-6 Sol" },
      { id: "builderbot-pr-reviews", label: "builderbot-pr-reviews" },
      { id: "prod.ai.rag-agent-3", label: "prod.ai.rag-agent-3" },
    ]),
    [
      "Default model",
      "GPT-6 Sol",
      "builderbot-pr-reviews",
      "prod.ai.rag-agent-3",
    ],
  );
});

test("non-Databricks providers are unchanged", () => {
  const rows = [
    { id: "a", label: "Same" },
    { id: "b", label: "Same" },
  ];
  assert.deepEqual(labels(rows, "openai"), ["Same", "Same"]);
  assert.deepEqual(labels(rows, null), ["Same", "Same"]);
});

test("grouping ignores case and the alias provider name", () => {
  assert.deepEqual(
    labels(
      [
        { id: "goose-gpt-5-4-mini", label: "GPT-5.4 mini" },
        { id: "databricks-gpt-5-4-mini", label: "GPT-5.4 Mini" },
      ],
      "DATABRICKS_V2",
    ),
    ["GPT-5.4 mini (goose)", "GPT-5.4 Mini (databricks)"],
  );
});
