import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { GuardianPolicyField } from "./GuardianPolicyField.tsx";
import {
  applyGuardianPolicy,
  GUARDIAN_POLICY_ENV,
  GUARDIAN_POLICY_OPTIONS,
} from "./guardianPolicy.ts";

test("Guardian policy maps monitor and lockdown to safe harness modes", () => {
  assert.equal(GUARDIAN_POLICY_ENV, "BUZZ_ACP_PERMISSION_MODE");
  assert.deepEqual(
    GUARDIAN_POLICY_OPTIONS.map(({ value }) => value),
    ["default", "dont-ask"],
  );
  assert.deepEqual(
    GUARDIAN_POLICY_OPTIONS.map(({ label }) => label),
    ["Monitor", "Lockdown"],
  );
});

test("Guardian policy control preserves config and writes the explicit override", () => {
  const config = {
    env_vars: { EXISTING: "kept" },
    provider: "openai",
    model: "gpt-test",
    preferred_runtime: null,
  };

  assert.deepEqual(applyGuardianPolicy(config, "dont-ask"), {
    ...config,
    env_vars: {
      EXISTING: "kept",
      BUZZ_ACP_PERMISSION_MODE: "dont-ask",
    },
  });
});

function renderField(config, onConfigChange = () => {}) {
  return renderToStaticMarkup(
    React.createElement(GuardianPolicyField, { config, onConfigChange }),
  );
}

test("Guardian policy renders the monitor default with accessible consequence copy", () => {
  const html = renderField({
    env_vars: {},
    provider: null,
    model: null,
    preferred_runtime: null,
  });

  assert.match(html, />Tool permission policy</);
  assert.match(html, />Monitor</);
  assert.match(html, /data-value="default"/);
  assert.match(
    html,
    /aria-describedby="global-agent-guardian-policy-description"/,
  );
  assert.match(
    html,
    /id="global-agent-guardian-policy-description"[^>]*>Monitor allows permission requests and records each decision\. Lockdown denies permission requests before the tool runs\./,
  );
});

test("Guardian policy renders persisted lockdown and emits the selected config", () => {
  const config = {
    env_vars: { EXISTING: "kept", BUZZ_ACP_PERMISSION_MODE: "dont-ask" },
    provider: "openai",
    model: "gpt-test",
    preferred_runtime: null,
  };
  const changes = [];
  const field = GuardianPolicyField({
    config,
    onConfigChange: (next) => changes.push(next),
  });
  const select = React.Children.toArray(field.props.children)[1];
  const html = renderField(config);

  assert.match(html, />Lockdown</);
  assert.match(html, /data-value="dont-ask"/);

  select.props.onValueChange("default");
  assert.deepEqual(changes, [
    {
      ...config,
      env_vars: {
        EXISTING: "kept",
        BUZZ_ACP_PERMISSION_MODE: "default",
      },
    },
  ]);
});
