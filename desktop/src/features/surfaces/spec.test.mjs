import assert from "node:assert/strict";
import test from "node:test";

import { formatScalar, parseSurfaceSpec } from "./spec.ts";

const validSpec = JSON.stringify({
  version: 1,
  fallbackText: "Deploy v2.4.1: 2/2 pods running",
  title: "Deployment — api-gateway",
  nodes: [
    { type: "badge", text: "HEALTHY", tone: "success" },
    { type: "heading", text: "Pods" },
    { type: "text", text: "All pods running." },
    {
      type: "keyValue",
      items: [{ label: "Version", value: "v2.4.1", tone: "info" }],
    },
    {
      type: "statGrid",
      stats: [
        { label: "Pods", value: 2, delta: "+1", tone: "success" },
        { label: "Errors", value: 0 },
      ],
    },
    {
      type: "table",
      columns: ["Pod", "Status"],
      rows: [
        ["web-7d9f", "Running"],
        ["web-a1c2", 3],
      ],
    },
    { type: "progress", label: "Rollout", value: 100 },
  ],
});

test("parseSurfaceSpec_validSpec_allNodesInOrder", () => {
  const result = parseSurfaceSpec(validSpec);
  assert.equal(result.outcome, "card");
  assert.equal(result.spec.nodes.length, 7);
  assert.deepEqual(
    result.spec.nodes.map((n) => n.type),
    ["badge", "heading", "text", "keyValue", "statGrid", "table", "progress"],
  );
  assert.equal(result.spec.title, "Deployment — api-gateway");
});

test("parseSurfaceSpec_numericValues_keptAndFormatted", () => {
  const result = parseSurfaceSpec(validSpec);
  assert.equal(result.outcome, "card");
  const statGrid = result.spec.nodes.find((n) => n.type === "statGrid");
  assert.equal(statGrid.stats[0].value, 2);
  assert.equal(formatScalar(statGrid.stats[0].value), "2");
  const table = result.spec.nodes.find((n) => n.type === "table");
  assert.equal(formatScalar(table.rows[1][1]), "3");
});

test("parseSurfaceSpec_unknownTone_coercesToDefault", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 1,
      fallbackText: "x",
      nodes: [{ type: "badge", text: "B", tone: "sparkly" }],
    }),
  );
  assert.equal(result.outcome, "card");
  assert.equal(result.spec.nodes[0].tone, "default");
});

test("parseSurfaceSpec_progress_clampedTo0100", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 1,
      fallbackText: "x",
      nodes: [
        { type: "progress", value: 250 },
        { type: "progress", value: -10 },
      ],
    }),
  );
  assert.equal(result.outcome, "card");
  assert.equal(result.spec.nodes[0].value, 100);
  assert.equal(result.spec.nodes[1].value, 0);
});

test("parseSurfaceSpec_invalidNode_dropsButSiblingsSurvive", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 1,
      fallbackText: "x",
      nodes: [
        { type: "badge", text: "OK", tone: "success" },
        { type: "iframe", src: "https://evil.example" },
        { type: "table", columns: ["A"], rows: [["ragged", "extra"]] },
        { type: "text", text: "still here" },
      ],
    }),
  );
  assert.equal(result.outcome, "card");
  assert.deepEqual(
    result.spec.nodes.map((n) => n.type),
    ["badge", "text"],
  );
});

test("parseSurfaceSpec_zeroValidNodes_fallsBackToFallbackText", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 1,
      fallbackText: "Deployment 80% complete",
      nodes: [{ type: "iframe", src: "https://evil.example" }],
    }),
  );
  assert.deepEqual(result, {
    outcome: "fallback",
    text: "Deployment 80% complete",
  });
});

test("parseSurfaceSpec_unknownVersion_fallsBackToFallbackText", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 2,
      fallbackText: "future card",
      nodes: [{ type: "badge", text: "B" }],
    }),
  );
  assert.deepEqual(result, { outcome: "fallback", text: "future card" });
});

test("parseSurfaceSpec_brokenJson_isRaw", () => {
  assert.deepEqual(parseSurfaceSpec("{not json"), { outcome: "raw" });
});

test("parseSurfaceSpec_missingFallbackText_isRaw", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({ version: 1, nodes: [{ type: "text", text: "y" }] }),
  );
  assert.deepEqual(result, { outcome: "raw" });
});

test("parseSurfaceSpec_nonScalarCell_dropsTableOnly", () => {
  const result = parseSurfaceSpec(
    JSON.stringify({
      version: 1,
      fallbackText: "x",
      nodes: [
        { type: "table", columns: ["A"], rows: [[true]] },
        { type: "badge", text: "B" },
      ],
    }),
  );
  assert.equal(result.outcome, "card");
  assert.deepEqual(
    result.spec.nodes.map((n) => n.type),
    ["badge"],
  );
});

test("parseSurfaceSpec_nodesBeyondCap_truncatedNotFatal", () => {
  const nodes = Array.from({ length: 40 }, (_, i) => ({
    type: "text",
    text: `node ${i}`,
  }));
  const result = parseSurfaceSpec(
    JSON.stringify({ version: 1, fallbackText: "x", nodes }),
  );
  assert.equal(result.outcome, "card");
  assert.equal(result.spec.nodes.length, 32);
});
