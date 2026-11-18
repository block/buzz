/**
 * Exercises the real discovery hook: runtime switches relabel the same cached
 * response, and a failed discovery always settles instead of loading forever.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const discovered = {
  agentName: "@agentclientprotocol/claude-agent-acp",
  agentVersion: "0.36.1",
  agentDefaultModel: "claude-sonnet-5",
  selectedModel: null,
  supportsSwitching: true,
  models: [{ id: "claude-sonnet-5", name: "Sonnet", description: null }],
};
const calls = [];
let discover = () => Promise.resolve(discovered);
const tauriMock = {
  invoke(command) {
    calls.push(command);
    if (command === "discover_agent_models") return discover();
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { usePersonaModelDiscovery } = await import(
  "./usePersonaModelDiscovery.ts"
);

const runtime = (id) => ({
  id,
  label: id,
  command: "shared-acp",
  availability: "available",
});
const envVars = [];

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  discover = () => Promise.resolve(discovered);
});

function mountRoot() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
}

let labels = null;
function Probe({ selectedRuntime }) {
  const { discoveredModelOptions } = usePersonaModelDiscovery({
    envVars,
    isCustomProviderEditing: false,
    modelFieldVisible: true,
    open: true,
    provider: "",
    selectedRuntime,
  });
  labels = discoveredModelOptions?.map((option) => option.label) ?? null;
  return null;
}

async function render(id) {
  await act(async () => {
    root.render(React.createElement(Probe, { selectedRuntime: runtime(id) }));
  });
}

test("hook relabels discovered models when the runtime switches", async () => {
  mountRoot();

  await render("claude");
  assert.deepEqual(labels, [
    "Default model (Claude Sonnet 5)",
    "Claude Sonnet 5",
  ]);
  await render("codex");
  assert.deepEqual(labels, ["Default model (Sonnet)", "Sonnet"]);
  await render("claude");
  assert.deepEqual(labels, [
    "Default model (Claude Sonnet 5)",
    "Claude Sonnet 5",
  ]);
  assert.equal(
    calls.filter((command) => command === "discover_agent_models").length,
    1,
  );
});

test("a missing Databricks host settles discovery instead of loading forever", async () => {
  discover = () =>
    Promise.reject(
      "buzz-acp models failed (exit 1): config: DATABRICKS_MODEL required",
    );
  let state = null;
  function DatabricksProbe() {
    state = usePersonaModelDiscovery({
      envVars,
      isCustomProviderEditing: false,
      modelFieldVisible: true,
      open: true,
      provider: "databricks_v2",
      selectedRuntime: runtime("buzz-agent"),
    });
    return null;
  }
  mountRoot();

  await act(async () => {
    root.render(React.createElement(DatabricksProbe));
  });
  // databricks_v2 discovery is debounced before the IPC call.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 400));
  });

  assert.equal(state.modelDiscoveryLoading, false);
  assert.match(state.modelDiscoveryStatus?.message ?? "", /DATABRICKS_HOST/);
});
