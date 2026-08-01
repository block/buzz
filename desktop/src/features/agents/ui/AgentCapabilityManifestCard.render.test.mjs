import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AgentCapabilityManifestView } from "./AgentCapabilityManifestCard.tsx";

function manifest(overrides = {}) {
  return {
    overallStatus: "ready",
    freshness: "fresh",
    lastVerifiedAt: "2026-07-26T02:00:00.000Z",
    runtime: { id: "codex", label: "Codex ACP", version: "1.2.3" },
    protocolVersion: "2",
    model: { value: "gpt-5.4", source: "observed" },
    provider: { value: "openai", source: "configured" },
    readiness: [
      {
        id: "installation",
        label: "Installation",
        status: "ready",
        detail: "Runtime and adapter available",
      },
      {
        id: "observer",
        label: "Observer",
        status: "ready",
        detail: "Connected",
      },
    ],
    features: [
      {
        id: "image-input",
        label: "Image input",
        state: "reported",
        source: "runtime",
      },
      {
        id: "audio-input",
        label: "Audio input",
        state: "unavailable",
        source: "runtime",
      },
      {
        id: "embedded-context",
        label: "Embedded context",
        state: "unknown",
        source: "runtime",
      },
    ],
    commands: ["create_plan"],
    commandsState: "reported",
    toolSources: ["github"],
    toolSourcesState: "reported",
    tools: [
      {
        name: "read_file",
        source: "filesystem",
        riskClass: "read",
        availability: "reported",
      },
      {
        name: "mystery_tool",
        source: "runtime",
        riskClass: "unknown",
        availability: "unknown",
      },
    ],
    toolsState: "reported",
    permissionMode: {
      requested: "bypassPermissions",
      effective: "perToolAutoDecision",
      source: "buzzHarness",
    },
    limitations: ["Runtime audio output is unreported."],
    ...overrides,
  };
}

test("renders readiness, evidence semantics, permission divergence, and tool risk", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentCapabilityManifestView, {
      manifest: manifest(),
    }),
  );

  assert.match(html, /data-testid="agent-capability-manifest"/);
  assert.match(html, />Ready</);
  assert.match(html, /Codex ACP 1\.2\.3/);
  assert.match(html, /data-state="reported"/);
  assert.match(html, /data-state="unavailable"/);
  assert.match(html, /data-state="unknown"/);
  assert.match(html, /bypassPermissions/);
  assert.match(html, /perToolAutoDecision/);
  assert.match(html, /mystery_tool/);
  assert.match(html, />unknown</);
  assert.match(html, /Runtime audio output is unreported/);
});

test("renders stopped and stale state without a ready claim", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentCapabilityManifestView, {
      manifest: manifest({
        overallStatus: "stopped",
        freshness: "stale",
      }),
    }),
  );

  assert.match(html, />Stopped</);
  assert.match(html, /Live evidence is stale/);
  assert.doesNotMatch(html, />Ready</);
});
