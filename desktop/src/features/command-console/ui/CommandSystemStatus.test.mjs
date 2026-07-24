import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandSystemStatus } from "./CommandSystemStatus.tsx";

const status = {
  laterCapabilities: [
    {
      detail: "No runtime integration is configured in Phase 1.",
      id: "lm-studio",
      label: "LM Studio",
      state: "not_configured",
      statusLabel: "Not configured",
    },
    {
      detail: "No memory integration is configured in Phase 1.",
      id: "memory",
      label: "Memory",
      state: "not_configured",
      statusLabel: "Not configured",
    },
    {
      detail: "No retrieval integration is configured in Phase 1.",
      id: "rag",
      label: "RAG",
      state: "not_configured",
      statusLabel: "Not configured",
    },
    {
      detail: "No Calendar, Reminders, or Notes access is configured.",
      id: "apple-inputs",
      label: "Apple inputs",
      state: "not_configured",
      statusLabel: "Not configured",
    },
  ],
  liveServices: [
    {
      detail: "Authenticated relay connection is active.",
      id: "relay",
      label: "Buzz relay",
      state: "connected",
      statusLabel: "Connected",
    },
    {
      detail: "Worker heartbeat is delayed.",
      id: "local-compute",
      label: "Local compute",
      state: "degraded",
      statusLabel: "Degraded",
    },
  ],
};

test("renders the composed read-only service status with explicit labels", () => {
  const html = renderToStaticMarkup(
    React.createElement(CommandSystemStatus, { status }),
  );

  assert.match(html, /data-testid="command-system-status"/);
  assert.match(html, />System status</);
  assert.match(html, />Buzz relay</);
  assert.match(html, />Connected</);
  assert.match(html, />Local compute</);
  assert.match(html, />Degraded</);
  assert.match(html, /Worker heartbeat is delayed\./);
});

test("renders every later capability as not configured", () => {
  const html = renderToStaticMarkup(
    React.createElement(CommandSystemStatus, { status }),
  );

  for (const label of ["LM Studio", "Memory", "RAG", "Apple inputs"]) {
    assert.match(html, new RegExp(`>${label}<`));
  }
  assert.equal(html.match(/>Not configured</g)?.length, 4);
  assert.doesNotMatch(html, /simulated/i);
});
