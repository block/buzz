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
const requests = [];
let discover = () => Promise.resolve(discovered);
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "discover_agent_models") {
      requests.push(args.input);
      return discover();
    }
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
  calls.length = 0;
  requests.length = 0;
  discover = () => Promise.resolve(discovered);
});

let labels = null;
function Probe({ selectedRuntime, model }) {
  const { discoveredModelOptions } = usePersonaModelDiscovery({
    envVars,
    model,
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

const pause = async () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 400));
  });
async function modelRender(model) {
  await act(async () =>
    root.render(
      React.createElement(Probe, { selectedRuntime: runtime("codex"), model }),
    ),
  );
}
function setupRoot() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
}
test("selected-model typing is debounced before native discovery", async () => {
  setupRoot();
  await modelRender("g");
  await modelRender("gpt");
  await modelRender("gpt-6-luna");
  assert.equal(
    requests.length,
    0,
    "typing must not immediately spawn native probes",
  );
  await pause();
  assert.deepEqual(
    requests.map((input) => input.model),
    ["gpt-6-luna"],
  );
});
test("a changed model waits for the active probe and skips superseded models", async () => {
  setupRoot();
  let finish;
  discover = () =>
    new Promise((resolve) => {
      finish = resolve;
    });
  await modelRender("gpt-6-astra");
  await pause();
  assert.equal(requests.length, 1);
  await modelRender("gpt-6-sol");
  await pause();
  await modelRender("gpt-6-luna");
  await pause();
  assert.equal(requests.length, 1, "at most one native probe may be in flight");
  discover = () => Promise.resolve(discovered);
  await act(async () => finish(discovered));
  await pause();
  assert.deepEqual(
    requests.map((input) => input.model),
    ["gpt-6-astra", "gpt-6-luna"],
  );
});
