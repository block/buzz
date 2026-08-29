import assert from "node:assert/strict";
import test from "node:test";

import {
  buildPersonaRuntimeDropdownOptions,
  getDefaultPersonaRuntime,
  getPersonaModelOptions,
  getPersonaProviderOptions,
  getProviderApiKeyLabel,
  isRuntimeReadyForNewSelection,
  isRuntimeSelectableForConfiguration,
  isRuntimeSetupFieldValueValid,
  resetConfigForHarnessChange,
  shouldPersistImplicitRuntimePreference,
  sortPersonaRuntimes,
  runtimeSupportsLlmProviderSelection,
  requiredCredentialEnvKeys,
  providerApiKeyUrl,
} from "./agentConfigOptions.tsx";
import { formatModelDiscoveryErrorStatus } from "./personaModelDiscoveryStatus.ts";

// ── helpers ──────────────────────────────────────────────────────────────────

function makeRuntime(id, availability = "available", overrides = {}) {
  return {
    id,
    label: id,
    command: id,
    defaultArgs: [],
    mcpCommand: null,
    availability,
    runtimeReadiness: "ready",
    ...overrides,
  };
}

const REMOTE_SETUP_FIELDS = [
  {
    envKey: "MANUAL_AGENT_BASE_URL",
    label: "Manual Agent Tailnet URL",
    placeholder: "https://<device>.<tailnet>.ts.net:8443",
    helperText: "Tailnet HTTPS origin",
    kind: "url",
    validation: "tailnet_https_origin",
    required: true,
  },
  {
    envKey: "MANUAL_AGENT_TOKEN",
    label: "Manual Agent app password",
    placeholder: "Paste the app password",
    helperText: "Local secret",
    kind: "secret",
    validation: "app_password",
    required: true,
  },
];

// ── getPersonaProviderOptions — hideProviderIds ───────────────────────────────

test("getPersonaProviderOptions returns databricks v1 and v2 when hideProviderIds is empty", () => {
  const options = getPersonaProviderOptions("", "buzz-agent", "", new Set());
  const ids = options.map((o) => o.id);
  assert.ok(ids.includes("databricks"), "databricks v1 present");
  assert.ok(ids.includes("databricks_v2"), "databricks v2 present");
});

test("getPersonaProviderOptions hides databricks v1 when it is in hideProviderIds", () => {
  const options = getPersonaProviderOptions(
    "",
    "buzz-agent",
    "",
    new Set(["databricks"]),
  );
  const ids = options.map((o) => o.id);
  assert.ok(!ids.includes("databricks"), "databricks v1 hidden");
  assert.ok(ids.includes("databricks_v2"), "databricks v2 still present");
});

test("getPersonaProviderOptions appends (current) tail for a saved databricks v1 value even when hidden", () => {
  // An agent already persisted with v1 must still render its saved value.
  const options = getPersonaProviderOptions(
    "databricks",
    "buzz-agent",
    "",
    new Set(["databricks"]),
  );
  const tail = options.at(-1);
  assert.equal(tail?.id, "databricks");
  assert.equal(tail?.label, "databricks (current)");
});

test("getPersonaProviderOptions with no hideProviderIds omits the tail for a known provider", () => {
  const options = getPersonaProviderOptions("anthropic", "buzz-agent");
  const tail = options.at(-1);
  // "anthropic" is a known id — no (current) tail appended
  assert.ok(
    tail?.id !== "anthropic" || tail?.label === "Anthropic",
    "no duplicate tail for known provider",
  );
});

test("getPersonaProviderOptions appends (current) tail for an unknown saved provider", () => {
  const options = getPersonaProviderOptions("my-custom-llm", "buzz-agent");
  const tail = options.at(-1);
  assert.equal(tail?.id, "my-custom-llm");
  assert.equal(tail?.label, "my-custom-llm (current)");
});

// ── getDefaultPersonaRuntime — OMPK first ────────────────────────────────────

test("getDefaultPersonaRuntime honors an available global preference", () => {
  const runtimes = [
    makeRuntime("ompk"),
    makeRuntime("buzz-agent"),
    makeRuntime("goose"),
    makeRuntime("claude"),
  ];
  assert.equal(getDefaultPersonaRuntime(runtimes, "claude")?.id, "claude");
});

test("getDefaultPersonaRuntime ignores an unavailable global preference", () => {
  const runtimes = [
    makeRuntime("ompk"),
    makeRuntime("buzz-agent"),
    makeRuntime("claude", "not_installed"),
  ];
  assert.equal(getDefaultPersonaRuntime(runtimes, "claude")?.id, "ompk");
});

test("getDefaultPersonaRuntime returns OMPK over bundled fallbacks", () => {
  const runtimes = [
    makeRuntime("goose"),
    makeRuntime("buzz-agent"),
    makeRuntime("ompk"),
    makeRuntime("claude"),
  ];
  const result = getDefaultPersonaRuntime(runtimes);
  assert.equal(result?.id, "ompk");
});

test("getDefaultPersonaRuntime falls back to buzz-agent when OMPK is unavailable", () => {
  const runtimes = [
    makeRuntime("ompk", "not_installed"),
    makeRuntime("goose"),
    makeRuntime("buzz-agent"),
  ];
  const result = getDefaultPersonaRuntime(runtimes);
  assert.equal(result?.id, "buzz-agent");
});

test("getDefaultPersonaRuntime falls back to goose when OMPK and buzz-agent are unavailable", () => {
  const runtimes = [
    makeRuntime("ompk", "not_installed"),
    makeRuntime("buzz-agent", "not_installed"),
    makeRuntime("goose"),
  ];
  const result = getDefaultPersonaRuntime(runtimes);
  assert.equal(result?.id, "goose");
});

test("getDefaultPersonaRuntime returns first available when preferred fallbacks are unavailable", () => {
  const runtimes = [
    makeRuntime("ompk", "not_installed"),
    makeRuntime("buzz-agent", "adapter_missing"),
    makeRuntime("goose", "cli_missing"),
    makeRuntime("claude"),
  ];
  const result = getDefaultPersonaRuntime(runtimes);
  assert.equal(result?.id, "claude");
});

test("sortPersonaRuntimes orders OMPK first within the same availability", () => {
  const runtimes = [
    makeRuntime("claude"),
    makeRuntime("goose"),
    makeRuntime("buzz-agent"),
    makeRuntime("ompk"),
  ];
  assert.deepEqual(
    sortPersonaRuntimes(runtimes).map((runtime) => runtime.id),
    ["ompk", "buzz-agent", "goose", "claude"],
  );
});

test("getDefaultPersonaRuntime returns null for an empty list", () => {
  assert.equal(getDefaultPersonaRuntime([]), null);
});

test("getDefaultPersonaRuntime returns null when no runtime is available", () => {
  const runtimes = [
    makeRuntime("ompk", "not_installed"),
    makeRuntime("buzz-agent", "not_installed"),
    makeRuntime("goose", "cli_missing"),
  ];
  assert.equal(getDefaultPersonaRuntime(runtimes), null);
});

// ── Runtime selection readiness ─────────────────────────────────────────────

test("new runtime selection requires availability and confirmed readiness", () => {
  assert.equal(
    isRuntimeReadyForNewSelection(
      makeRuntime("claude", "available", {
        runtimeReadiness: "authentication_required",
      }),
    ),
    false,
  );
  assert.equal(
    isRuntimeReadyForNewSelection(makeRuntime("claude", "cli_missing")),
    false,
  );
  assert.equal(isRuntimeReadyForNewSelection(makeRuntime("claude")), true);
});

test("implicit defaults persist only a ready runtime", () => {
  const unready = makeRuntime("ompk", "available", {
    runtimeReadiness: "model_unavailable",
  });
  assert.equal(shouldPersistImplicitRuntimePreference(null, unready), false);
  assert.equal(
    shouldPersistImplicitRuntimePreference(null, makeRuntime("ompk")),
    true,
  );
  assert.equal(shouldPersistImplicitRuntimePreference("ompk", unready), false);
});

test("setup-required runtime is selectable but never an implicit default", () => {
  const remote = makeRuntime("remote-agent-computer", "available", {
    runtimeReadiness: "configuration_required",
    setupFields: REMOTE_SETUP_FIELDS,
  });
  assert.equal(isRuntimeReadyForNewSelection(remote), false);
  assert.equal(isRuntimeSelectableForConfiguration(remote), true);
  assert.equal(shouldPersistImplicitRuntimePreference(null, remote), false);
  assert.equal(
    isRuntimeSelectableForConfiguration({ ...remote, setupFields: [] }),
    false,
  );
});

test("runtime setup validation enforces HTTPS origins and strong app passwords", () => {
  const [urlField, tokenField] = REMOTE_SETUP_FIELDS;
  assert.equal(
    isRuntimeSetupFieldValueValid(
      urlField,
      "https://device.example-tailnet.ts.net:8443",
    ),
    true,
  );
  for (const invalid of [
    "http://device.example-tailnet.ts.net:8443",
    "https://example.com",
    "https://device..ts.net:8443",
    "https://user:pass@device.example-tailnet.ts.net:8443",
    "https://device.example-tailnet.ts.net:8443/runs",
    "https://device.example-tailnet.ts.net:8443?token=secret",
  ]) {
    assert.equal(isRuntimeSetupFieldValueValid(urlField, invalid), false);
  }
  assert.equal(isRuntimeSetupFieldValueValid(tokenField, "short"), false);
  assert.equal(
    isRuntimeSetupFieldValueValid(
      tokenField,
      "test-app-password-0123456789-abcdef",
    ),
    true,
  );
  assert.deepEqual(
    requiredCredentialEnvKeys("remote-agent-computer", "", REMOTE_SETUP_FIELDS),
    ["MANUAL_AGENT_BASE_URL", "MANUAL_AGENT_TOKEN"],
  );
});

test("runtime dropdown disables unready new selections but retains an edit's current choice", () => {
  const unready = makeRuntime("ompk", "available", {
    runtimeReadiness: "authentication_required",
  });
  const ready = makeRuntime("goose");
  const createOptions = buildPersonaRuntimeDropdownOptions({
    defaultRuntimeId: "goose",
    isCreateMode: true,
    runtime: "",
    runtimes: [unready, ready],
    runtimesLoading: false,
  }).runtimeDropdownOptions;
  assert.equal(
    createOptions.find((option) => option.value === "ompk")?.disabled,
    true,
  );
  assert.equal(
    createOptions.find((option) => option.value === "goose")?.disabled,
    false,
  );

  const editOptions = buildPersonaRuntimeDropdownOptions({
    isCreateMode: false,
    runtime: "ompk",
    runtimes: [unready, ready],
    runtimesLoading: false,
  }).runtimeDropdownOptions;
  assert.equal(
    editOptions.find((option) => option.value === "ompk")?.disabled,
    false,
  );
});

// ── runtimeSupportsLlmProviderSelection — provider gating ────────────────────

test("runtimeSupportsLlmProviderSelection follows catalog providerEnvVar", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("BUZZ_AGENT_PROVIDER"),
    true,
  );
  assert.equal(runtimeSupportsLlmProviderSelection("GOOSE_PROVIDER"), true);
  assert.equal(runtimeSupportsLlmProviderSelection("  GOOSE_PROVIDER  "), true);
});

test("runtimeSupportsLlmProviderSelection is false without providerEnvVar", () => {
  assert.equal(runtimeSupportsLlmProviderSelection(null), false);
  assert.equal(runtimeSupportsLlmProviderSelection(undefined), false);
  assert.equal(runtimeSupportsLlmProviderSelection(""), false);
  assert.equal(runtimeSupportsLlmProviderSelection("   "), false);
  // ompk currently projects null provider_env_var in the Rust catalog.
  assert.equal(runtimeSupportsLlmProviderSelection(null), false);
});

test("resetConfigForHarnessChange clears harness-specific values", () => {
  const config = {
    env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high", KEEP_ME: "yes" },
    model: "claude-opus",
    preferred_runtime: "buzz-agent",
    provider: "anthropic",
  };

  assert.deepEqual(resetConfigForHarnessChange(config, "claude"), {
    env_vars: { KEEP_ME: "yes" },
    model: null,
    preferred_runtime: "claude",
    provider: null,
  });
});

test("resetConfigForHarnessChange preserves compatible provider selection", () => {
  const config = {
    env_vars: { KEEP_ME: "yes" },
    model: "old-model",
    preferred_runtime: "claude",
    provider: "anthropic",
  };

  assert.deepEqual(
    resetConfigForHarnessChange(config, "goose", "GOOSE_PROVIDER"),
    {
      env_vars: { KEEP_ME: "yes" },
      model: null,
      preferred_runtime: "goose",
      provider: "anthropic",
    },
  );
});

test("resetConfigForHarnessChange does not carry relay mesh to Goose", () => {
  const config = {
    env_vars: {},
    model: "auto",
    preferred_runtime: "buzz-agent",
    provider: "relay-mesh",
  };

  assert.equal(
    resetConfigForHarnessChange(config, "goose", "GOOSE_PROVIDER").provider,
    null,
  );
});

// ── getPersonaModelOptions — codex/claude do not use global provider ──────────
//
// The discovery call in AgentDefinitionDialog passes
// `runtimeSupportsLlmProviderSelection(runtime) ? effectiveProvider : ""`
// so codex/claude never receive the global provider. These tests verify that
// the static model options also stay provider-agnostic for those runtimes.

test("getPersonaModelOptions for codex returns only default model regardless of provider", () => {
  const withProvider = getPersonaModelOptions("codex", "anthropic", null);
  const withoutProvider = getPersonaModelOptions("codex", "", null);
  assert.deepEqual(withProvider, withoutProvider);
  assert.equal(withProvider.length, 1);
  assert.equal(withProvider[0]?.id, "");
});

test("getPersonaModelOptions for buzz-agent with anthropic filters out zero-value default", () => {
  // anthropic requires explicit model — zero-value option is filtered out
  const options = getPersonaModelOptions(
    "buzz-agent",
    "anthropic",
    "BUZZ_AGENT_PROVIDER",
  );
  const zeroValue = options.find((o) => o.id === "");
  assert.equal(
    zeroValue,
    undefined,
    "explicit-model provider must not allow zero-value selection",
  );
});

test("getPersonaModelOptions for buzz-agent with no provider returns default model", () => {
  const options = getPersonaModelOptions(
    "buzz-agent",
    "",
    "BUZZ_AGENT_PROVIDER",
  );
  assert.equal(options.length, 1);
  assert.equal(options[0]?.id, "");
});

// ── formatModelDiscoveryErrorStatus — runtime unavailable ────────────────────
//
// When selectedRuntime.availability !== "available", AgentDefinitionDialog and
// usePersonaModelDiscovery now call formatModelDiscoveryErrorStatus with a
// synthetic "Runtime not available: <availability>" error. Verify the status
// is non-null (so the UI surfaces the reason) for each unavailability reason.

test("formatModelDiscoveryErrorStatus returns a non-null status for runtime unavailable errors", () => {
  for (const availability of [
    "adapter_missing",
    "cli_missing",
    "not_installed",
  ]) {
    const status = formatModelDiscoveryErrorStatus(
      new Error(`Runtime not available: ${availability}`),
      "anthropic",
    );
    assert.ok(
      status !== null,
      `should return a status for availability=${availability}`,
    );
    assert.ok(typeof status?.message === "string", "status has a message");
    assert.ok(typeof status?.tone === "string", "status has a tone");
  }
});

// ── getProviderApiKeyLabel — provider-accurate credential field labels ────────
//
// Each provider with a secretEnvVar must have a distinct label. The helper
// is the single source of truth used by all three credential field surfaces;
// if it regresses the field labels diverge silently and the OpenRouter / compat
// mislabeling recurs.

test("getProviderApiKeyLabel_anthropic_returns_anthropic_label", () => {
  assert.equal(getProviderApiKeyLabel("anthropic"), "Anthropic API Key");
});

test("getProviderApiKeyLabel_openai_returns_openai_runtime_label", () => {
  assert.equal(getProviderApiKeyLabel("openai"), "OpenAI Runtime API Key");
});

test("getProviderApiKeyLabel_openai_compat_returns_distinct_label", () => {
  // openai and openai-compat must have distinct labels — both use
  // OPENAI_COMPAT_API_KEY but carry different semantic identities.
  assert.equal(
    getProviderApiKeyLabel("openai-compat"),
    "OpenAI-compatible Runtime API Key",
  );
});

test("getProviderApiKeyLabel_openrouter_returns_openrouter_label", () => {
  // Key fix: OpenRouter was mislabeled "OpenAI API Key" before this change.
  assert.equal(getProviderApiKeyLabel("openrouter"), "OpenRouter API Key");
});

test("getProviderApiKeyLabel_databricks_returns_null", () => {
  // Databricks uses OAuth PKCE — no typed-secret label.
  assert.equal(getProviderApiKeyLabel("databricks"), null);
});

test("getProviderApiKeyLabel_databricks_v2_returns_null", () => {
  assert.equal(getProviderApiKeyLabel("databricks_v2"), null);
});

test("getProviderApiKeyLabel_unknown_provider_returns_null", () => {
  assert.equal(getProviderApiKeyLabel("some-unknown-provider"), null);
});

test("getProviderApiKeyLabel_provider_id_trimmed_and_lowercased", () => {
  // Mirrors getProviderApiKeyEnvVar normalisation behaviour.
  assert.equal(getProviderApiKeyLabel(" Anthropic "), "Anthropic API Key");
});

test("cline is offered only by runtimes with the buzz-agent provider capability", () => {
  const buzzAgentOptions = getPersonaProviderOptions(
    "",
    "buzz-agent",
    "",
    new Set(),
    "BUZZ_AGENT_PROVIDER",
  );
  const option = buzzAgentOptions.find((candidate) => candidate.id === "cline");
  assert.equal(option?.label, "Cline");

  const gooseOptions = getPersonaProviderOptions(
    "",
    "goose",
    "",
    new Set(),
    "GOOSE_PROVIDER",
  );
  assert.ok(
    !gooseOptions.some((candidate) => candidate.id === "cline"),
    "Goose must not offer an unsupported named Cline provider",
  );

  const catalogCapabilityWinsOverRuntimeId = getPersonaProviderOptions(
    "",
    "custom-buzz-agent-build",
    "",
    new Set(),
    "BUZZ_AGENT_PROVIDER",
  );
  assert.ok(
    catalogCapabilityWinsOverRuntimeId.some(
      (candidate) => candidate.id === "cline",
    ),
    "providerEnvVar, not a hardcoded runtime id, controls availability",
  );

  assert.deepEqual(requiredCredentialEnvKeys("buzz-agent", "cline"), [
    "CLINE_API_KEY",
  ]);
});

test("saved Cline value remains visible on unsupported runtimes as current-only", () => {
  const options = getPersonaProviderOptions(
    "cline",
    "goose",
    "",
    new Set(),
    "GOOSE_PROVIDER",
  );
  assert.equal(options.at(-1)?.id, "cline");
  assert.equal(options.at(-1)?.label, "cline (current)");
  assert.equal(
    options.filter((candidate) => candidate.id === "cline").length,
    1,
  );
});

test("cline credential field links to where keys are issued", () => {
  // Billing questions (ClinePass vs credits) belong on Cline's page, not in
  // Pkzz copy, so the field links out instead of asserting a billing model.
  const url = providerApiKeyUrl("cline");
  assert.ok(url?.startsWith("https://app.cline.bot/"), `got ${url}`);

  // Providers without a self-serve page stay undefined rather than guessing.
  assert.equal(providerApiKeyUrl("relay-mesh"), undefined);
  assert.equal(providerApiKeyUrl(null), undefined);
  assert.equal(providerApiKeyUrl("not-a-provider"), undefined);
});
