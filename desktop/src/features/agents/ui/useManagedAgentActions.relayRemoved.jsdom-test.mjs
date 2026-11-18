import assert from "node:assert/strict";
import test from "node:test";

const PUBKEY = "ab".repeat(32);
const RELAY = "wss://removed.example";
const UNRELATED_RELAY = "wss://unrelated.example";
const calls = [];
let releaseStop;
let createSpawnError = null;
const rawAgent = {
  pubkey: PUBKEY,
  name: "Scout",
  persona_id: null,
  relay_url: RELAY,
  acp_command: "acp",
  agent_command: "agent",
  agent_args: [],
  mcp_command: "mcp",
  turn_timeout_seconds: 60,
  idle_timeout_seconds: 60,
  max_turn_duration_seconds: 60,
  parallelism: 1,
  system_prompt: null,
  model: null,
  status: "running",
  pid: 1,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  last_started_at: null,
  last_stopped_at: null,
  last_exit_code: null,
  last_error: null,
  log_path: "/tmp/log",
  start_on_app_launch: false,
  backend: { type: "local" },
  backend_agent_id: null,
};
const tauriMock = {
  invoke(command) {
    calls.push(command);
    if (command === "list_managed_agents") return Promise.resolve([rawAgent]);
    if (command === "stop_managed_agent") {
      return new Promise((resolve) => {
        releaseStop = () => resolve({ ...rawAgent, status: "stopped" });
      });
    }
    if (command === "start_managed_agent") return Promise.resolve(rawAgent);
    if (command === "discover_acp_providers") {
      return Promise.resolve([
        {
          id: "buzz-agent",
          label: "Buzz Agent",
          avatar_url: "",
          availability: "available",
          command: "buzz-agent",
          binary_path: "/bin/buzz-agent",
          default_args: [],
          mcp_command: null,
        },
      ]);
    }
    if (command === "create_managed_agent") {
      return Promise.resolve({
        agent: { ...rawAgent, status: "stopped" },
        private_key_nsec: "nsec1test",
        profile_sync_error: null,
        spawn_error: createSpawnError,
      });
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
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { useManagedAgentActions } = await import("./useManagedAgentActions.ts");
const { markRelayRemoved } = await import("../managedAgentRelayCleanup.ts");
const { CommunitiesProvider } = await import(
  "../../communities/useCommunities.tsx"
);
const { saveActiveCommunityId, saveCommunities } = await import(
  "../../communities/communityStorage.ts"
);
const { RELAY_REMOVED_ERROR } = await import("../managedAgentRelayCleanup.ts");

// The active community is the workspace pair the restart targets.
saveCommunities([
  { id: "b", name: "B", relayUrl: RELAY },
  { id: "a", name: "A", relayUrl: UNRELATED_RELAY },
]);
saveActiveCommunityId("b");

async function renderActions() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const root = createRoot(document.createElement("div"));
  const latest = {};
  function Harness() {
    Object.assign(latest, useManagedAgentActions());
    return null;
  }
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(Harness),
        ),
      ),
    );
  });
  for (let i = 0; i < 20 && !latest.managedAgents?.length; i += 1) {
    await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  }
  assert.equal(latest.managedAgents?.length, 1, "agent list loaded");
  return { latest, unmount: () => act(async () => root.unmount()) };
}

async function restartAcrossStop(duringStop) {
  calls.length = 0;
  const { latest, unmount } = await renderActions();
  let restart;
  await act(async () => {
    restart = latest.handleRestart(PUBKEY);
  });
  for (let i = 0; i < 20 && !releaseStop; i += 1) {
    await act(() => new Promise((resolve) => setTimeout(resolve, 5)));
  }
  duringStop();
  await act(async () => {
    releaseStop();
    await restart;
  });
  releaseStop = undefined;
  const result = {
    started: calls.includes("start_managed_agent"),
    error: latest.actionErrorMessage,
  };
  await unmount();
  return result;
}

test("a Settings restart whose relay is removed and re-added during its stop starts nothing and shows no error", async () => {
  // Re-adding never resets the counter, so only the removal matters here.
  const result = await restartAcrossStop(() => markRelayRemoved(RELAY));
  assert.equal(result.started, false);
  assert.equal(result.error, null);
});

test("removing an unrelated community during the stop still restarts the agent", async () => {
  const result = await restartAcrossStop(() =>
    markRelayRemoved(UNRELATED_RELAY),
  );
  assert.equal(result.started, true);
  assert.equal(result.error, null);
});

test("a Settings restart with no removal during its stop starts the agent", async () => {
  const result = await restartAcrossStop(() => {});
  assert.equal(result.started, true);
  assert.equal(result.error, null);
});

async function startPersonaWithSpawnError(spawnError) {
  createSpawnError = spawnError;
  calls.length = 0;
  const { latest, unmount } = await renderActions();
  await act(async () => {
    await latest.handleStartPersona({
      id: "persona-1",
      displayName: "Scout",
      systemPrompt: "help",
      avatarUrl: null,
      runtime: null,
    });
  });
  const result = {
    created: calls.includes("create_managed_agent"),
    error: latest.actionErrorMessage,
  };
  await unmount();
  createSpawnError = null;
  return result;
}

test("a persona Start whose spawn a relay removal cancelled shows no error", async () => {
  const result = await startPersonaWithSpawnError(RELAY_REMOVED_ERROR);
  assert.equal(result.created, true);
  assert.equal(result.error, null);
});

test("a persona Start with a genuine spawn error still shows it", async () => {
  const result = await startPersonaWithSpawnError(
    "runtime executable not found",
  );
  assert.equal(result.created, true);
  assert.equal(result.error, "runtime executable not found");
});
