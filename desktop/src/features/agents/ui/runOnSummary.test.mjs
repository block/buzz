import assert from "node:assert/strict";
import test from "node:test";

import { humanizeConfigKey, summarizeRunOn } from "./runOnSummary.ts";

test("local backend summarizes to the local location with no rows", () => {
  assert.deepEqual(summarizeRunOn({ type: "local" }), { location: "local" });
});

test("provider backend carries the provider id", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "kubernetes",
    config: {},
  });
  assert.equal(summary.location, "provider");
  assert.equal(summary.providerId, "kubernetes");
  assert.deepEqual(summary.rows, []);
});

test("a saved kubernetes config renders labeled scalar rows", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "kubernetes",
    config: {
      namespace: "buzz-agents-x7k2mp",
      image: "ghcr.io/block/buzz-sprig@sha256:17facfc7",
      cpu_request: "1",
      inactivity_seconds: 7200,
    },
  });
  assert.equal(summary.location, "provider");
  const byKey = Object.fromEntries(summary.rows.map((r) => [r.key, r]));
  assert.equal(byKey.namespace.label, "Namespace");
  assert.equal(byKey.namespace.value, "buzz-agents-x7k2mp");
  assert.equal(byKey.cpu_request.label, "CPU request");
  assert.equal(byKey.cpu_request.value, "1");
  assert.equal(byKey.inactivity_seconds.label, "Inactivity seconds");
  assert.equal(byKey.inactivity_seconds.value, "7200");
  assert.equal(byKey.image.value, "ghcr.io/block/buzz-sprig@sha256:17facfc7");
  assert.ok(summary.rows.every((r) => r.redacted === false));
});

test("rows follow the provider-schema preferred order, spillover alphabetical", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "kubernetes",
    config: {
      // Deliberately shuffled persisted order.
      memory_limit: "1Gi",
      zeta_extra: "z",
      namespace: "n",
      cpu_limit: "1",
      alpha_extra: "a",
      image: "i",
      inactivity_seconds: 7200,
      cpu_request: "1",
      context: "c",
      memory_request: "1Gi",
    },
  });
  assert.deepEqual(
    summary.rows.map((r) => r.key),
    [
      "context",
      "namespace",
      "image",
      "cpu_request",
      "memory_request",
      "cpu_limit",
      "memory_limit",
      "inactivity_seconds",
      "alpha_extra",
      "zeta_extra",
    ],
  );
});

test("null, empty-string, boolean, and number values all display honestly", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "p",
    config: { a_null: null, b_empty: "", c_flag: true, d_num: 0, e_off: false },
  });
  const values = Object.fromEntries(summary.rows.map((r) => [r.key, r.value]));
  assert.equal(values.a_null, "Not set");
  assert.equal(values.b_empty, "Not set");
  assert.equal(values.c_flag, "true");
  // Falsy-but-present values must never read as missing: coerceConfigValues
  // emits real numbers/booleans, so `value || fallback` would lie about 0/false.
  assert.equal(values.d_num, "0");
  assert.equal(values.e_off, "false");
});

test("every row value is a string — objects must never reach React children", () => {
  // React throws on an object child, taking down the whole edit dialog.
  // A hand-edited managed-agents.json with nested config is exactly the
  // record the create-time scalar gate never saw.
  const summary = summarizeRunOn({
    type: "provider",
    id: "p",
    config: {
      resources: { limits: { cpu: "2" } },
      flag: true,
      count: 3,
      name: "x",
      missing: null,
    },
  });
  for (const row of summary.rows) {
    assert.equal(typeof row.value, "string", `${row.key} is not a string`);
  }
});

test("arrays and objects are summarized, never serialized", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "p",
    config: {
      tolerations: [{ key: "gpu" }, { key: "spot" }],
      node_selector: { disktype: "ssd" },
    },
  });
  const values = Object.fromEntries(summary.rows.map((r) => [r.key, r.value]));
  assert.equal(values.tolerations, "List (2 items)");
  assert.equal(values.node_selector, "Structured value");
  // The nested content must not leak through.
  assert.ok(!JSON.stringify(values).includes("ssd"));
});

test("secret-shaped keys are redacted, fail-safe for unknown providers", () => {
  const summary = summarizeRunOn({
    type: "provider",
    id: "future-provider",
    config: {
      api_token: "abc123",
      registry_password: "hunter2",
      clientSecret: "s3cr3t",
      authHeader: "Bearer xyz",
      privateKey: "nsec1...",
      namespace: "safe-to-show",
    },
  });
  const byKey = Object.fromEntries(summary.rows.map((r) => [r.key, r]));
  for (const key of [
    "api_token",
    "registry_password",
    "clientSecret",
    "authHeader",
    "privateKey",
  ]) {
    assert.equal(byKey[key].redacted, true, `${key} must be redacted`);
    assert.equal(byKey[key].value, "••••••••");
  }
  assert.equal(byKey.namespace.redacted, false);
  assert.equal(byKey.namespace.value, "safe-to-show");
  // Raw secret bytes must be absent from the whole summary.
  const dump = JSON.stringify(summary);
  for (const secret of ["abc123", "hunter2", "s3cr3t", "Bearer xyz", "nsec1"]) {
    assert.ok(!dump.includes(secret), `${secret} leaked`);
  }
});

test("humanizeConfigKey handles snake_case, camelCase, and acronyms", () => {
  assert.equal(humanizeConfigKey("cpu_request"), "CPU request");
  assert.equal(humanizeConfigKey("memory_limit"), "Memory limit");
  assert.equal(humanizeConfigKey("serviceAccount"), "Service account");
  assert.equal(humanizeConfigKey("image"), "Image");
  assert.equal(humanizeConfigKey("api_url"), "API URL");
  assert.equal(humanizeConfigKey(""), "");
});
