import assert from "node:assert/strict";
import { test } from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  RuntimeSetupFields,
  runtimeSetupFieldState,
} from "./RuntimeSetupFields.tsx";

const TOKEN_FIELD = {
  envKey: "MANUAL_AGENT_TOKEN",
  label: "Manual Agent app password",
  placeholder: "Paste the app password",
  helperText: "Stored as a local secret and never added to messages.",
  kind: "secret",
  validation: "app_password",
  required: true,
};

test("stored secret status renders no token-shaped placeholder or value", () => {
  const html = renderToStaticMarkup(
    React.createElement(RuntimeSetupFields, {
      configuredByKeys: ["MANUAL_AGENT_TOKEN"],
      fields: [TOKEN_FIELD],
      onChange: () => {},
      value: {},
    }),
  );
  assert.match(html, /type="password"/);
  assert.match(html, /Stored securely on this device/);
  assert.equal(html.includes("test-app-password"), false);
  assert.equal(html.includes("MANUAL_AGENT_TOKEN&quot;:"), false);
});

test("invalid local setup value shadows a valid inherited value", () => {
  assert.deepEqual(
    runtimeSetupFieldState({
      field: TOKEN_FIELD,
      inheritedConfiguredByKeys: ["MANUAL_AGENT_TOKEN"],
      value: { MANUAL_AGENT_TOKEN: "short" },
    }),
    {
      configured: false,
      hasLocalValue: true,
      inherited: false,
      localValue: "short",
      valid: false,
    },
  );
});

test("secure-store failure stays visible even with a newly typed value", () => {
  const html = renderToStaticMarkup(
    React.createElement(RuntimeSetupFields, {
      fields: [TOKEN_FIELD],
      onChange: () => {},
      secureStorageUnavailable: true,
      value: {
        MANUAL_AGENT_TOKEN: "test-app-password-0123456789-abcdef",
      },
    }),
  );
  assert.match(html, /Secure credential storage is unavailable/);
});
