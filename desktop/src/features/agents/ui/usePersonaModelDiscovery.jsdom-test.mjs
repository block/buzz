/**
 * The real discovery hook passes the selected runtime to the label code:
 * switching Claude Code -> Codex -> Claude Code relabels the same cached
 * discovery response.
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
const tauriMock = {
  invoke(command) {
    calls.push(command);
    if (command === "discover_agent_models") return Promise.resolve(discovered);
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
});

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
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);

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
