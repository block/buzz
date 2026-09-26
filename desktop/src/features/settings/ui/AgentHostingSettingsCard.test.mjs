import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  MutationObserver: dom.window.MutationObserver,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
const preferred = [
  {
    relay_url: "https://relay.example",
    owner_pubkey: "a".repeat(64),
    name: "Scout",
    pubkey: "b".repeat(64),
  },
];
let state, calls, rejectSave;
dom.window.__TAURI_INTERNALS__ = {
  invoke: async (command, payload) => {
    calls.push({ command, payload });
    if (command === "get_agent_device_policy") return state;
    if (command === "set_agent_device_policy") {
      if (rejectSave) throw "Disk is not writable";
      state = { ...state, saved: payload.policy, restartRequired: true };
      return state;
    }
    throw new Error(`Unexpected side effect: ${command}`);
  },
};
let React,
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
  QueryClient,
  QueryClientProvider,
  Card;
before(async () => {
  React = await import("react");
  ({ render, screen, fireEvent, waitFor, cleanup } = await import(
    "@testing-library/react"
  ));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ AgentHostingSettingsCard: Card } = await import(
    "./AgentHostingSettingsCard.tsx"
  ));
});
afterEach(() => cleanup());
function mount(overrides = {}) {
  state = {
    activeClientOnly: false,
    saved: { client_only: false, preferred_agents: preferred },
    restartRequired: false,
    loadError: null,
    ...overrides,
  };
  calls = [];
  rejectSave = false;
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false, gcTime: 0 },
    },
  });
  render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(Card),
    ),
  );
}

test("unique-name mode discloses when no existing identities are protected", async () => {
  mount({
    activeUniqueNames: true,
    saved: { client_only: false, unique_names: true, preferred_agents: [] },
  });
  await screen.findByText(/No existing agent identities are protected/);
  await screen.findByText(/does not stop existing local agents/);
});

test("unique-name hosting enables local agents without clearing the remote bindings", async () => {
  mount({
    activeClientOnly: true,
    saved: { client_only: true, preferred_agents: preferred },
  });
  const toggle = await screen.findByRole("switch", {
    name: "Unique agent names",
  });
  await waitFor(() => assert.equal(toggle.disabled, false));
  fireEvent.click(toggle);
  await screen.findByText(/Restart Buzz to apply/);
  assert.deepEqual(state.saved, {
    client_only: false,
    unique_names: true,
    preferred_agents: preferred,
  });
  assert.equal(state.activeClientOnly, true);
});

test("discovery visibility cannot remove protected identities in unique-name mode", async () => {
  mount({
    activeUniqueNames: true,
    saved: {
      client_only: false,
      unique_names: true,
      preferred_agents: preferred,
    },
  });
  await screen.findByText(/Preferred existing agents/);
  assert.equal(
    screen.queryByRole("button", { name: "Show all existing identities" }),
    null,
  );
});

test("client-only Save preserves exact preferred identities and discloses restart", async () => {
  mount();
  const toggle = await screen.findByRole("switch", {
    name: "Client-only mode",
  });
  await waitFor(() => assert.equal(toggle.disabled, false));
  fireEvent.click(toggle);
  await screen.findByText(/Restart Buzz to apply/);
  assert.equal(toggle.getAttribute("aria-checked"), "true");
  assert.deepEqual(
    calls.filter((c) => c.command.startsWith("set_")).map((c) => c.payload),
    [{ policy: { client_only: true, preferred_agents: preferred } }],
  );
  assert.equal(
    state.activeClientOnly,
    false,
    "the current process must not claim the new policy yet",
  );
});

test("failed Save leaves the old setting visible and retryable", async () => {
  mount();
  rejectSave = true;
  const toggle = await screen.findByRole("switch", {
    name: "Client-only mode",
  });
  await waitFor(() => assert.equal(toggle.disabled, false));
  fireEvent.click(toggle);
  await screen.findByText("Disk is not writable");
  assert.equal(toggle.getAttribute("aria-checked"), "false");
  rejectSave = false;
  fireEvent.click(toggle);
  await screen.findByText(/Restart Buzz to apply/);
  assert.equal(toggle.getAttribute("aria-checked"), "true");
});

test("clearing discovery preferences preserves client hosting policy and never deletes agents", async () => {
  mount();
  const button = await screen.findByRole("button", {
    name: "Show all existing identities",
  });
  fireEvent.click(button);
  await screen.findByText(/Restart Buzz to apply/);
  assert.deepEqual(state.saved, { client_only: false, preferred_agents: [] });
  assert.deepEqual(
    calls.map((c) => c.command),
    ["get_agent_device_policy", "set_agent_device_policy"],
  );
});
