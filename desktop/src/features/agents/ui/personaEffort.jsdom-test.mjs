import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
const runtime = (id) => ({
  id,
  label: id === "codex" ? "Codex" : "Claude Code",
  command: `${id}-acp`,
  availability: "available",
  defaultArgs: [],
  modelEnvVar: null,
  providerEnvVar: null,
  thinkingEnvVar: "BUZZ_ACP_EFFORT_LEVEL",
  effortCanonicalValues: [],
  requiredEnvVars: [],
});
const runtimes = [
  runtime("codex"),
  runtime("claude"),
  { ...runtime("goose"), thinkingEnvVar: "GOOSE_THINKING_EFFORT" },
];
let globalConfig;
const mock = {
  async invoke(command, args) {
    if (command === "get_global_agent_config") return globalConfig;
    if (command === "get_runtime_file_config") return null;
    if (command === "discover_agent_models")
      return {
        agentName: "fixture",
        agentVersion: "test",
        supportsSwitching: true,
        agentDefaultModel: "gpt-6-astra",
        selectedModel: args.input.model,
        models: ["gpt-6-astra", "gpt-6-luna", "sonnet"].map((id) => ({
          id,
          name: id,
          description: null,
        })),
        effortOption: {
          id: "effort",
          category: "thought_level",
          options: ["default", "low", "medium", "high", "ultra"].map(
            (value) => ({ value }),
          ),
        },
      };
    if (command === "get_agent_access_owner_only") return false;
    return [];
  },
  transformCallback: () => 1,
};
globalThis.__TAURI_INTERNALS__ = mock;
window.__TAURI_INTERNALS__ = mock;
const React = await import("react");
const { act, render, fireEvent, screen, cleanup, waitFor } = await import(
  "@testing-library/react"
);
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { AgentDefinitionDialog } = await import("./AgentDefinitionDialog.tsx");
let client;
afterEach(() => {
  cleanup();
  client?.clear();
});
const settle = async () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 380));
  });
async function mount(overrides = {}) {
  globalConfig = {
    preferred_runtime: "codex",
    model: "gpt-6-astra",
    provider: null,
    env_vars: { BUZZ_ACP_EFFORT_LEVEL: "high" },
  };
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  let saved;
  render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(AgentDefinitionDialog, {
        open: true,
        embedded: true,
        title: "Edit agent",
        description: "Fixture",
        submitLabel: "Save changes",
        isPending: false,
        error: null,
        runtimes,
        runtimeCatalogStatus: "ready",
        onOpenChange: () => {},
        initialValues: {
          id: "fixture",
          displayName: "Test Agent",
          systemPrompt: "Fixture instructions",
          runtime: "codex",
          model: "gpt-6-astra",
          envVars: { BUZZ_ACP_EFFORT_LEVEL: "ultra" },
          ...overrides,
        },
        onSubmit: async (input) => {
          saved = input;
        },
      }),
    ),
  );
  await settle();
  return async () => {
    const save = screen.getByRole("button", { name: "Save changes" });
    await waitFor(() => assert.equal(save.disabled, false));
    await act(async () => fireEvent.click(save));
    return saved;
  };
}
test("profile model change saves without the previous model's effort", async () => {
  const save = await mount();
  await act(async () =>
    fireEvent.click(screen.getByRole("combobox", { name: "Model" })),
  );
  await act(async () =>
    fireEvent.click(
      screen.getByRole("button", { name: "gpt-6-luna", exact: true }),
    ),
  );
  await settle();
  const saved = await save();
  assert.equal(saved.model, "gpt-6-luna");
  assert.equal(saved.envVars.BUZZ_ACP_EFFORT_LEVEL, undefined);
});
test("profile can explicitly choose adapter default over inherited high", async () => {
  const save = await mount({ runtime: "claude", model: "sonnet", envVars: {} });
  const effort = await screen.findByTestId("persona-effort");
  await waitFor(() => assert.equal(effort.disabled, false));
  await act(async () => fireEvent.click(effort));
  const option = screen.getByTestId("persona-effort-option-default");
  await act(async () => fireEvent.click(option));
  const saved = await save();
  assert.equal(saved.envVars.BUZZ_ACP_EFFORT_LEVEL, "default");
});
test("inherited Edit follows updated global harness before Customize", async () => {
  const save = await mount({ runtime: null, model: null, envVars: {} });
  globalConfig = {
    ...globalConfig,
    preferred_runtime: "claude",
    model: "sonnet",
  };
  await act(async () =>
    client.setQueryData(["globalAgentConfig"], globalConfig),
  );
  await settle();
  assert.match(
    screen.getByTestId("agent-harness-defaults-notice").textContent,
    /Claude Code/,
  );
  const customTab = screen.getByRole("tab", {
    name: "Customize for this agent",
  });
  await act(async () =>
    fireEvent.mouseDown(customTab, { button: 0, ctrlKey: false }),
  );
  assert.equal(customTab.getAttribute("aria-selected"), "true");
  await settle();
  const saved = await save();
  assert.equal(saved.runtime, "claude");
  assert.equal(saved.model, "sonnet");
});

test("using defaults removes both native and legacy Goose profile effort", async () => {
  const save = await mount({
    runtime: "goose",
    provider: "openai",
    envVars: {
      GOOSE_THINKING_EFFORT: "high",
      BUZZ_AGENT_THINKING_EFFORT: "medium",
    },
  });
  await act(async () =>
    fireEvent.mouseDown(
      screen.getByRole("tab", { name: /^Use .* defaults$/ }),
      { button: 0, ctrlKey: false },
    ),
  );
  await settle();
  const saved = await save();
  assert.equal(saved.envVars.GOOSE_THINKING_EFFORT, undefined);
  assert.equal(saved.envVars.BUZZ_AGENT_THINKING_EFFORT, undefined);
});
