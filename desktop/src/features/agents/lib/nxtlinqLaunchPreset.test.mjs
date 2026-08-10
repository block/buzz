import assert from "node:assert/strict";
import test from "node:test";

import {
  buildNxtlinqWrapperArgs,
  deriveNxtlinqOperatorDefaults,
  deriveNxtlinqReceiptDirectory,
  isNxtlinqGatewayCommand,
  nxtlinqLaunchPresetMatches,
  parseNxtlinqLaunchPreset,
  shouldBlockNxtlinqLaunchSave,
} from "./nxtlinqLaunchPreset.ts";

test("recognizes a resolved or PATH-based Nxtlinq wrapper", () => {
  assert.equal(isNxtlinqGatewayCommand("nxtlinq-authorization-gateway"), true);
  assert.equal(
    isNxtlinqGatewayCommand("/opt/nxtlinq/bin/nxtlinq-authorization-gateway"),
    true,
  );
  assert.equal(isNxtlinqGatewayCommand("codex-acp"), false);
});

test("derives operator state beside, never inside, the agent workspace", () => {
  assert.deepEqual(deriveNxtlinqOperatorDefaults("/projects/company-api"), {
    trustStore: "/projects/.company-api-operator/trusted-signers.json",
    receiptDirectory: "/projects/.company-api-operator/receipts",
  });
});

test("derives an isolated receipt directory from the global root", () => {
  assert.equal(
    deriveNxtlinqReceiptDirectory("/operator/receipts/", "agent:one"),
    "/operator/receipts/agent-one",
  );
});

test("builds deterministic shell-free wrapper argv and preserves env names", () => {
  const args = buildNxtlinqWrapperArgs({
    project: "/projects/company-api",
    trustStore: "/operator/trust.json",
    receiptDirectory: "/operator/receipts",
    passEnvironment: ["OPENAI_COMPAT_API_KEY", "BUZZ_AGENT_PROVIDER"],
  });
  assert.deepEqual(args.slice(0, 11), [
    "--adapter",
    "acp",
    "--project",
    "/projects/company-api",
    "--trust-store",
    "/operator/trust.json",
    "--receipt-dir",
    "/operator/receipts",
    "--mode",
    "acp-enforce",
    "--pass-env",
  ]);
  assert.equal(args.at(-1), "--");
  assert.equal(
    args.filter((value) => value === "BUZZ_AGENT_PROVIDER").length,
    1,
  );
  assert.ok(args.includes("OPENAI_COMPAT_API_KEY"));
  assert.ok(args.includes("BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE"));
  assert.equal(args.includes("BUZZ_ACP_TRUST_NXTLINQ_GATEWAY"), false);
});

test("parses the policy paths back from wrapper argv", () => {
  assert.deepEqual(
    parseNxtlinqLaunchPreset([
      "--project",
      "/project",
      "--trust-store",
      "/operator/trust.json",
      "--receipt-dir",
      "/operator/receipts",
      "--",
    ]),
    {
      project: "/project",
      trustStore: "/operator/trust.json",
      receiptDirectory: "/operator/receipts",
    },
  );
});

test("detects when checked draft paths differ from the applied wrapper", () => {
  const applied = {
    project: "/projects/old",
    trustStore: "/operator/old-trust.json",
    receiptDirectory: "/operator/old-receipts",
  };
  const draft = {
    project: "/projects/new",
    trustStore: "/operator/trust.json",
    receiptDirectory: "/operator/receipts",
  };

  assert.equal(nxtlinqLaunchPresetMatches(applied, applied), true);
  assert.equal(nxtlinqLaunchPresetMatches(applied, draft), false);
  assert.equal(
    shouldBlockNxtlinqLaunchSave({
      enabled: true,
      appliedPreset: applied,
      draftPreset: draft,
      draftVerified: false,
    }),
    true,
  );
  assert.equal(
    shouldBlockNxtlinqLaunchSave({
      enabled: true,
      appliedPreset: applied,
      draftPreset: draft,
      draftVerified: true,
    }),
    false,
  );
  assert.equal(
    shouldBlockNxtlinqLaunchSave({
      enabled: false,
      appliedPreset: applied,
      draftPreset: draft,
      draftVerified: false,
    }),
    false,
  );
});

test("fails closed when the comma transport cannot preserve a path", () => {
  assert.throws(
    () =>
      buildNxtlinqWrapperArgs({
        project: "/projects/with,comma",
        trustStore: "/operator/trust.json",
        receiptDirectory: "/operator/receipts",
        passEnvironment: [],
      }),
    /cannot contain commas/,
  );
});
