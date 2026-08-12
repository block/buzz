import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AgentPermissionPolicyField } from "./AgentPermissionPolicyField.tsx";

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

// A remotely deployed agent: backend is a provider and a backendAgentId exists.
// The drift row only appears for this shape, so most cases build on it.
function deployedAgent(overrides) {
  return {
    backend: { type: "provider", id: "openclaw", config: {} },
    backendAgentId: "backend-1",
    permissionPolicy: "reject",
    permissionPolicySource: "global_default",
    appliedPermissionPolicy: "allow",
    ...overrides,
  };
}

function render(agent) {
  return renderToStaticMarkup(
    React.createElement(AgentPermissionPolicyField, { agent, disabled: false }),
  );
}

// ---------------------------------------------------------------------------
// Remote + drift: applied ≠ desired ⇒ amber drift row with both values
// ---------------------------------------------------------------------------

test("test_remote_drift_shows_applied_desired_and_redeploy_required", () => {
  const html = render(deployedAgent());

  assert.ok(
    html.includes("Applied policy:"),
    "drift row must label the applied policy",
  );
  assert.ok(html.includes("allow"), "drift row must show the applied value");
  assert.ok(html.includes("reject"), "drift row must show the desired value");
  assert.ok(
    html.includes("redeploy required"),
    "drift row must prompt a redeploy",
  );
});

// ---------------------------------------------------------------------------
// Remote, applied == desired: no drift row
// ---------------------------------------------------------------------------

test("test_remote_no_drift_when_applied_equals_desired_renders_no_drift_row", () => {
  const html = render(
    deployedAgent({
      permissionPolicy: "allow",
      appliedPermissionPolicy: "allow",
    }),
  );

  assert.ok(
    !html.includes("Applied policy:"),
    "no drift row when applied equals desired",
  );
});

// ---------------------------------------------------------------------------
// Remote, applied absent (pre-feature / never-redeployed): no drift row
// ---------------------------------------------------------------------------

test("test_remote_absent_applied_renders_no_drift_row", () => {
  const html = render(deployedAgent({ appliedPermissionPolicy: null }));

  assert.ok(
    !html.includes("Applied policy:"),
    "absent applied policy must fail quiet — no drift row",
  );
});

// ---------------------------------------------------------------------------
// Local agent: editable select, never a drift row even if applied differs
// ---------------------------------------------------------------------------

test("test_local_agent_renders_editable_select_and_no_drift_row", () => {
  const html = render({
    backend: { type: "local" },
    backendAgentId: null,
    permissionPolicy: "reject",
    permissionPolicySource: "global_default",
    appliedPermissionPolicy: "allow",
  });

  assert.ok(
    html.includes("<select"),
    "local agent must render the editable policy select",
  );
  assert.ok(
    !html.includes("Applied policy:"),
    "local agent must never show a drift row",
  );
});

// ---------------------------------------------------------------------------
// Provider selected but not yet deployed (backendAgentId null): no drift row
// ---------------------------------------------------------------------------

test("test_provider_not_yet_deployed_renders_no_drift_row", () => {
  const html = render(deployedAgent({ backendAgentId: null }));

  assert.ok(
    !html.includes("Applied policy:"),
    "an undeployed provider agent has no confirmed receipt — no drift row",
  );
});
