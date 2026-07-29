import assert from "node:assert/strict";
import test from "node:test";

import {
  autoPickRemoteHarness,
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  hostFailureOf,
  providerConfigComplete,
  remoteHarnessOptions,
  remoteHarnessSummaryLabel,
  rememberProbedProviderName,
  remoteModelDiscoveryView,
  resolveBackendIntent,
  runTargetOptions,
  selectedRemoteHarness,
} from "./whereToRunIntent.ts";
import { TauriInvokeError } from "@/shared/api/tauri";

const probed = {
  ok: true,
  config_schema: {
    properties: { region: { type: "string" }, size: { type: "integer" } },
    required: ["region"],
  },
};

const gooseHarness = {
  id: "goose",
  label: "Goose",
  command: "/opt/host/bin/goose",
  args: ["acp"],
  env: { GOOSE_MODE: "auto" },
  available: true,
  binaryPath: "/opt/host/bin/goose",
  version: "1.2.0",
};

function providerDraft(overrides = {}) {
  return {
    ...emptyWhereToRunDraft,
    runOn: "blox",
    probedProvider: probed,
    providerConfig: { region: "us", size: "3" },
    remoteHarnesses: [gooseHarness],
    remoteHarnessId: "goose",
    ...overrides,
  };
}

test("provider selection blocks submit until the probe completes", () => {
  assert.equal(
    canSubmitWhereToRun(providerDraft({ probedProvider: null })),
    false,
  );
});

test("provider selection blocks submit while required config is missing", () => {
  const missing = providerDraft({ providerConfig: { size: "3" } });
  assert.equal(canSubmitWhereToRun(missing), false);
  assert.equal(providerConfigComplete(missing), false);
});

test("complete provider config allows submit", () => {
  assert.equal(canSubmitWhereToRun(providerDraft()), true);
});

test("local never gates submit", () => {
  assert.equal(canSubmitWhereToRun(emptyWhereToRunDraft), true);
});

test("local draft resolves to null intent", () => {
  assert.equal(resolveBackendIntent(emptyWhereToRunDraft), null);
});

test("provider draft resolves with coerced config values and the remote harness", () => {
  const intent = resolveBackendIntent(providerDraft());
  assert.deepEqual(intent, {
    type: "provider",
    id: "blox",
    config: { region: "us", size: 3 },
    harness: {
      id: "goose",
      command: "/opt/host/bin/goose",
      args: ["acp"],
      env: { GOOSE_MODE: "auto" },
    },
  });
});

// Correction C1: the harness pin is the only channel by which the choice
// reaches the host, so submit must stay blocked until one is picked rather
// than letting the create fall back to the locally-resolved default.
test("provider selection blocks submit until a remote harness is picked", () => {
  assert.equal(
    canSubmitWhereToRun(
      providerDraft({ remoteHarnesses: null, remoteHarnessId: null }),
    ),
    false,
  );
  assert.equal(
    canSubmitWhereToRun(providerDraft({ remoteHarnessId: null })),
    false,
  );
});

test("a harness id with no matching catalog entry does not unblock submit", () => {
  assert.equal(
    canSubmitWhereToRun(providerDraft({ remoteHarnessId: "codex" })),
    false,
  );
  assert.equal(
    selectedRemoteHarness(providerDraft({ remoteHarnessId: "codex" })),
    null,
  );
});

// An unavailable entry names a harness the host reported as NOT installed.
// Pinning it would ship a command that fails at deploy time, after the create
// has already reported success — so it can never become the selection, even
// though a re-check can turn a live pick unavailable while it is still set.
test("an unavailable harness can never become the pin", () => {
  const stale = providerDraft({
    remoteHarnesses: [{ ...gooseHarness, available: false }],
  });

  assert.equal(selectedRemoteHarness(stale), null);
  assert.equal(canSubmitWhereToRun(stale), false);
  assert.equal(resolveBackendIntent(stale).harness, undefined);
  assert.equal(remoteModelDiscoveryView(stale), null);
});

test("local drafts never carry a remote harness", () => {
  assert.equal(
    selectedRemoteHarness({ ...providerDraft(), runOn: "local" }),
    null,
  );
});

function modelsResponse(overrides = {}) {
  return {
    agentName: "Goose",
    agentVersion: "1.2.0",
    models: [{ id: "gpt-5", name: "GPT-5" }],
    agentDefaultModel: "gpt-5",
    selectedModel: null,
    supportsSwitching: true,
    ...overrides,
  };
}

// The whole point of the remote probe: a local draft, or one without a picked
// harness, has nothing to have probed, so the local discovery path keeps
// owning the Model control.
test("model discovery view is null without a picked remote harness", () => {
  assert.equal(remoteModelDiscoveryView(emptyWhereToRunDraft), null);
  assert.equal(
    remoteModelDiscoveryView(providerDraft({ remoteHarnessId: null })),
    null,
  );
  assert.equal(
    remoteModelDiscoveryView({ ...providerDraft(), runOn: "local" }),
    null,
  );
});

test("an unprobed harness leaves the model control to the local path", () => {
  assert.equal(
    remoteModelDiscoveryView(
      providerDraft({ remoteModelProbe: { status: "idle" } }),
    ),
    null,
  );
});

test("an in-flight probe reports loading with no options and no status", () => {
  assert.deepEqual(
    remoteModelDiscoveryView(
      providerDraft({ remoteModelProbe: { status: "loading" } }),
    ),
    {
      harnessId: "goose",
      discoveredModelOptions: null,
      modelDiscoveryLoading: true,
      modelDiscoveryStatus: null,
    },
  );
});

test("a loaded probe offers the host's models plus a default row", () => {
  const view = remoteModelDiscoveryView(
    providerDraft({
      remoteModelProbe: { status: "loaded", models: modelsResponse() },
    }),
  );
  assert.equal(view.harnessId, "goose");
  assert.equal(view.modelDiscoveryLoading, false);
  assert.equal(view.modelDiscoveryStatus, null);
  assert.deepEqual(view.discoveredModelOptions, [
    { id: "", label: "Default model (gpt-5)" },
    { id: "gpt-5", label: "GPT-5" },
  ]);
});

// A failed probe must not fall back to this computer's catalog: it would
// scope the picker to models the remote harness cannot run.
test("a failed probe surfaces host-specific copy and no options", () => {
  const view = remoteModelDiscoveryView(
    providerDraft({
      remoteModelProbe: { status: "failed", error: "ssh: connection refused" },
    }),
  );
  assert.equal(view.discoveredModelOptions, null);
  assert.equal(view.modelDiscoveryLoading, false);
  assert.equal(view.modelDiscoveryStatus.tone, "warning");
  assert.match(view.modelDiscoveryStatus.message, /ssh: connection refused/);
  // The probe reads env at call time, so typing a missing key afterwards does
  // not re-probe by itself. Name the retry or the auth case is a dead end.
  assert.match(view.modelDiscoveryStatus.message, /check the host again/);
});

test("a harness that reports no models warns about the host, not this machine", () => {
  const view = remoteModelDiscoveryView(
    providerDraft({
      remoteModelProbe: {
        status: "loaded",
        models: modelsResponse({ models: [], agentDefaultModel: null }),
      },
    }),
  );
  assert.equal(view.discoveredModelOptions, null);
  assert.equal(view.modelDiscoveryStatus.tone, "warning");
  assert.match(view.modelDiscoveryStatus.message, /Goose reported no models/);
  assert.match(view.modelDiscoveryStatus.message, /on the host/);
});

// ── The run-target question (first step of the create flow) ───────────────

const providers = [
  { id: "blox", binaryPath: "/usr/local/bin/buzz-backend-blox" },
  { id: "ssh", binaryPath: "/home/u/.local/bin/buzz-backend-ssh" },
];

test("this computer always leads the run-target list", () => {
  assert.deepEqual(runTargetOptions([], {}), [
    { label: "This computer", value: "local" },
  ]);
});

test("unprobed providers are labelled by id", () => {
  assert.deepEqual(runTargetOptions(providers, {}), [
    { label: "This computer", value: "local" },
    { label: "blox", value: "blox" },
    { label: "ssh", value: "ssh" },
  ]);
});

// info is a subprocess round-trip and only providers the user has actually
// selected have paid for one, so the friendlier name decorates the entries
// already probed rather than spawning every discovered binary on dialog open.
test("a probed provider's own name labels its entry", () => {
  assert.deepEqual(runTargetOptions(providers, { ssh: "SSH" }), [
    { label: "This computer", value: "local" },
    { label: "blox", value: "blox" },
    { label: "SSH", value: "ssh" },
  ]);
});

// The whole point of caching: a name, once paid for, is not surrendered when
// the user moves the selection elsewhere. Otherwise the same machine reads
// under two naming schemes depending on where the cursor is.
test("a probed name survives the selection moving to another provider", () => {
  const afterProbingSsh = rememberProbedProviderName({}, "ssh", {
    ok: true,
    name: "SSH",
  });
  const afterProbingBlox = rememberProbedProviderName(afterProbingSsh, "blox", {
    ok: true,
    name: "Blox",
  });
  assert.deepEqual(runTargetOptions(providers, afterProbingBlox), [
    { label: "This computer", value: "local" },
    { label: "Blox", value: "blox" },
    { label: "SSH", value: "ssh" },
  ]);
});

test("a blank probed name falls back to the id", () => {
  const names = rememberProbedProviderName({}, "ssh", {
    ok: true,
    name: "   ",
  });
  assert.deepEqual(names, {});
  assert.equal(runTargetOptions(providers, names)[2].label, "ssh");
});

// Identity matters: the cache feeds a setState, so a probe that adds nothing
// must not re-render the dialog (probes re-run on every provider selection).
test("remembering a name already known returns the same cache object", () => {
  const names = { ssh: "SSH" };
  assert.equal(
    rememberProbedProviderName(names, "ssh", { ok: true, name: "SSH" }),
    names,
  );
  assert.equal(rememberProbedProviderName(names, "ssh", null), names);
  assert.equal(
    rememberProbedProviderName(names, "local", { ok: true, name: "Local" }),
    names,
  );
});

// ── The harness summary label ─────────────────────────────────────────────

test("the summary label is the host's harness, versioned when known", () => {
  assert.equal(remoteHarnessSummaryLabel(providerDraft()), "Goose (1.2.0)");
  assert.equal(
    remoteHarnessSummaryLabel(
      providerDraft({
        remoteHarnesses: [{ ...gooseHarness, version: null }],
      }),
    ),
    "Goose",
  );
});

// null hands the label back to the local catalog — the same contract
// remoteModelDiscoveryView uses for the Model control.
test("the summary label defers to the local catalog with no remote pick", () => {
  assert.equal(remoteHarnessSummaryLabel(emptyWhereToRunDraft), null);
  assert.equal(
    remoteHarnessSummaryLabel(
      providerDraft({ remoteHarnesses: null, remoteHarnessId: null }),
    ),
    null,
  );
  assert.equal(
    remoteHarnessSummaryLabel(
      providerDraft({
        remoteHarnesses: [{ ...gooseHarness, available: false }],
      }),
    ),
    null,
    "an unavailable entry is not the pin, so it must not name the summary",
  );
});

// ── The harness picker rows ───────────────────────────────────────────────

const hermesDefault = {
  id: "hermes-default",
  label: "Hermes (default)",
  command: "hermes",
  args: ["--profile", "default", "acp"],
  env: {},
  available: true,
  binaryPath: "/usr/local/bin/hermes",
  version: null,
  exclusive: true,
};

const hermesMatt = {
  ...hermesDefault,
  id: "hermes-matt",
  label: "Hermes (matt)",
};

test("harness rows carry the label, the version, and nothing else by default", () => {
  const options = remoteHarnessOptions(
    [gooseHarness, hermesDefault],
    new Set(),
  );
  assert.deepEqual(options, [
    { label: "Goose (1.2.0)", value: "goose" },
    { label: "Hermes (default)", value: "hermes-default" },
  ]);
});

test("an added exclusive row stays visible but is disabled and annotated", () => {
  const options = remoteHarnessOptions(
    [gooseHarness, hermesDefault, hermesMatt],
    new Set(["hermes-default"]),
  );
  assert.deepEqual(options, [
    { label: "Goose (1.2.0)", value: "goose" },
    {
      label: "Hermes (default) (added)",
      value: "hermes-default",
      disabled: true,
    },
    { label: "Hermes (matt)", value: "hermes-matt" },
  ]);
});

test("unavailable entries are never offered", () => {
  const options = remoteHarnessOptions(
    [{ ...gooseHarness, available: false }, hermesDefault],
    new Set(),
  );
  assert.deepEqual(
    options.map((option) => option.value),
    ["hermes-default"],
  );
});

test("no catalog yields no rows", () => {
  assert.deepEqual(remoteHarnessOptions(null, new Set()), []);
  assert.deepEqual(remoteHarnessOptions([], new Set()), []);
});

// ── Auto-pick after a catalog read ────────────────────────────────────────

test("auto-pick keeps the previous choice when the host still offers it", () => {
  assert.equal(
    autoPickRemoteHarness(
      [gooseHarness, hermesDefault],
      new Set(),
      "hermes-default",
    )?.id,
    "hermes-default",
  );
});

test("auto-pick falls to the first selectable entry", () => {
  assert.equal(
    autoPickRemoteHarness([gooseHarness, hermesDefault], new Set(), null)?.id,
    "goose",
  );
  assert.equal(
    autoPickRemoteHarness([gooseHarness, hermesDefault], new Set(), "gone")?.id,
    "goose",
    "a stale previous id must not survive a re-check",
  );
});

// Auto-picking an added-exclusive entry would arm a create the picker itself
// refuses, and submitting it would put a second agent on one identity.
test("auto-pick skips added-exclusive entries, even a previous pick", () => {
  assert.equal(
    autoPickRemoteHarness(
      [hermesDefault, hermesMatt],
      new Set(["hermes-default"]),
      "hermes-default",
    )?.id,
    "hermes-matt",
  );
  assert.equal(
    autoPickRemoteHarness([hermesDefault], new Set(["hermes-default"]), null),
    null,
  );
});

test("auto-pick never returns an unavailable entry", () => {
  assert.equal(
    autoPickRemoteHarness(
      [{ ...gooseHarness, available: false }],
      new Set(),
      "goose",
    ),
    null,
  );
});

// ── hostFailureOf: the one conversion every host `catch` goes through ────────

test("hostFailureOf lifts a provider recovery alongside the message", () => {
  const failure = hostFailureOf(
    new TauriInvokeError("needs browser auth", {
      message: "needs browser auth",
      recovery: {
        action: "open_url",
        url: "https://login.tailscale.com/a/1a2b3c4d",
      },
    }),
  );
  assert.deepEqual(failure, {
    message: "needs browser auth",
    recovery: {
      action: "open_url",
      url: "https://login.tailscale.com/a/1a2b3c4d",
    },
  });
});

test("hostFailureOf reports an ordinary failure with no recovery", () => {
  // The common case: the message renders alone, with no button.
  assert.deepEqual(hostFailureOf(new Error("ssh failed (exit 255)")), {
    message: "ssh failed (exit 255)",
    recovery: null,
  });
  assert.deepEqual(hostFailureOf("host unreachable"), {
    message: "host unreachable",
    recovery: null,
  });
});
