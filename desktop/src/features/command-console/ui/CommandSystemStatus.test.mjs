import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CommandSystemStatus } from "./CommandSystemStatus.tsx";

const status = {
  degradedSections: ["apple-reminders", "memory-conflicts"],
  liveServices: [
    {
      detail: "Authenticated relay connection is active.",
      id: "relay",
      label: "Command workspace",
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
    {
      detail: "2 unresolved conflicts are excluded from unattended context.",
      diagnostics: [
        "Resolve Memory conflicts before unattended brief generation.",
      ],
      facts: [
        { label: "Node", value: "node:command" },
        { label: "Replication cursor", value: "41" },
        { label: "Conflicts", value: "2" },
        { label: "Permissions", value: "get_entity, recall_for_entity" },
      ],
      id: "memory",
      label: "Memory",
      state: "degraded",
      statusLabel: "Degraded",
    },
    {
      detail: "A fresh signed active snapshot is verified.",
      facts: [
        { label: "Active snapshot", value: "f".repeat(64) },
        { label: "Freshness", value: "Fresh" },
        { label: "Validation", value: "Verified" },
      ],
      id: "rag",
      label: "RAG",
      state: "connected",
      statusLabel: "Connected",
    },
    {
      detail: "One read-only source is denied.",
      facts: [
        { label: "Calendar", value: "Authorized" },
        { label: "Reminders", value: "Denied" },
      ],
      id: "apple-inputs",
      label: "Apple inputs",
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
  assert.match(html, />Command workspace</);
  assert.match(html, />Connected</);
  assert.match(html, />Local compute</);
  assert.match(html, />Degraded</);
  assert.match(html, /Worker heartbeat is delayed\./);
});

test("renders live knowledge facts, permissions, and actionable diagnostics", () => {
  const html = renderToStaticMarkup(
    React.createElement(CommandSystemStatus, { status }),
  );

  for (const label of ["Memory", "RAG", "Apple inputs"]) {
    assert.match(html, new RegExp(`>${label}<`));
  }
  assert.match(html, />Active snapshot</);
  assert.match(html, new RegExp(`>${"f".repeat(64)}<`));
  assert.match(html, />Replication cursor</);
  assert.match(html, />41</);
  assert.match(html, />Validation</);
  assert.match(html, />Verified</);
  assert.match(html, />Reminders</);
  assert.match(html, />Denied</);
  assert.match(html, />Degraded sections:</);
  assert.match(html, /apple-reminders, memory-conflicts/);
  assert.match(
    html,
    /Resolve Memory conflicts before unattended brief generation\./,
  );
  assert.doesNotMatch(html, /Later capabilities|intentionally not connected/i);
});
