/**
 * Production recovery regression for a lost first-attest acknowledgement.
 *
 * Creation keeps the registered identity but skips channel attachment when
 * attest returns an error. Relay presence can still be Online/Away if the
 * provider accepted that attest. The Agents surface must therefore keep a
 * scoped, idempotent retry that does not use ordinary duplicate-spawn guards.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const AGENT_PUBKEY = "b".repeat(64);
const OWNER_PUBKEY = "a".repeat(64);
const RELAY_URL = "wss://community.example";
const clients = [];
const commands = [];
const handlers = new Map();

let act;
let cleanup;
let CommunitiesProvider;
let createElement;
let fireEvent;
let fromRawManagedAgent;
let QueryClient;
let QueryClientProvider;
let render;
let renderHook;
let screen;
let waitFor;
let UnifiedAgentsSection;
let useCreatedAgentChannelAttachment;
let useAgentLifecycleActions;
let useManagedAgentActions;

function rawPendingAgent() {
  return {
    pubkey: AGENT_PUBKEY,
    name: "Pending Provider Agent",
    persona_id: null,
    relay_url: null,
    acp_command: "",
    agent_command: "",
    agent_args: [],
    mcp_command: "",
    turn_timeout_seconds: 320,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: null,
    model: null,
    env_vars: {},
    status: "not_deployed",
    pid: null,
    created_at: "2026-09-09T00:00:00Z",
    updated_at: "2026-09-09T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: "attest reply lost",
    last_error_code: null,
    log_path: null,
    start_on_app_launch: false,
    auto_restart_on_config_change: false,
    backend: { type: "provider", id: "remote", config: {} },
    key_custody: "provider",
    backend_agent_id: "registered-agent-id",
    respond_to: "owner-only",
    respond_to_allowlist: [],
  };
}

function sectionProps(actions) {
  return {
    actionErrorMessage: actions.actionErrorMessage,
    actionNoticeMessage: actions.actionNoticeMessage,
    agents: actions.managedAgents,
    agentsError: null,
    defaultModel: "gpt-x",
    getAvailability: actions.getAvailability,
    isActionPending: actions.isPending,
    isAgentsLoading: false,
    isPersonasLoading: false,
    isPersonasPending: false,
    onDeactivatePersona() {},
    onDeletePersona() {},
    onDuplicatePersona() {},
    onEditPersona() {},
    onOpenAgentProfile() {},
    onOpenCatalog() {},
    onOpenPersonaProfile() {},
    onRestartAgent: actions.handleRestart,
    onSharePersona() {},
    onStartAgent: actions.handleStart,
    onStartPersona: actions.handleStartPersona,
    personaFeedbackErrorMessage: null,
    personaFeedbackNoticeMessage: null,
    personas: [],
    personasError: null,
    restartingAgentPubkey: actions.restartingAgentPubkey,
    startingAgentPubkey: actions.startingAgentPubkey,
    startingPersonaIds: actions.startingPersonaIds,
  };
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.requestAnimationFrame = (callback) =>
    dom.window.setTimeout(() => callback(Date.now()), 0);
  dom.window.cancelAnimationFrame = (id) => dom.window.clearTimeout(id);
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      commands.push([command, args]);
      const handler = handlers.get(command);
      if (handler) return handler(args);
      throw new Error(`Unexpected IPC: ${command}`);
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render, renderHook, screen, waitFor } =
    await import("@testing-library/react"));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ fromRawManagedAgent } = await import("@/shared/api/tauri"));
  ({ UnifiedAgentsSection } = await import("./UnifiedAgentsSection.tsx"));
  ({ useCreatedAgentChannelAttachment } = await import(
    "@/features/agents/useCreatedAgentChannelAttachment.ts"
  ));
  ({ useAgentLifecycleActions } = await import(
    "@/features/profile/ui/useAgentLifecycleActions.ts"
  ));
  ({ useManagedAgentActions } = await import("./useManagedAgentActions.ts"));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  commands.length = 0;
  handlers.clear();
  localStorage.clear();
});

after(() => dom.window.close());

async function exerciseRecovery(availability) {
  localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Community A",
        relayUrl: RELAY_URL,
        addedAt: "2026-09-09T00:00:00Z",
      },
    ]),
  );
  localStorage.setItem("buzz-active-community-id", "community-a");
  const agent = fromRawManagedAgent(rawPendingAgent());
  const creation = renderHook(() => useCreatedAgentChannelAttachment());
  await act(async () => {
    await creation.result.current.presentCreatedAgent(
      {
        agent,
        privateKeyNsec: "",
        profileSyncError: null,
        spawnError: "attest reply lost",
      },
      { id: "target-channel", name: "target" },
    );
  });
  assert.equal(
    commands.some(([command]) => command === "add_channel_member"),
    false,
    "the real create-result path leaves a failed first attest unattached",
  );

  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0, retry: false },
      queries: { gcTime: 0, retry: false, staleTime: Infinity },
    },
  });
  clients.push(client);
  client.setQueryData(["identity"], {
    pubkey: OWNER_PUBKEY,
    displayName: "Owner",
  });
  client.setQueryData(["managed-agents"], [agent]);
  client.setQueryData(["relay-agents"], []);
  client.setQueryData(["channels"], []);
  client.setQueryData(["globalAgentConfig"], { env_vars: {} });
  handlers.set("get_presence", () => ({ [AGENT_PUBKEY]: availability }));
  handlers.set("list_archived_identities", () => []);
  handlers.set("get_user_profile", () => ({
    pubkey: AGENT_PUBKEY,
    display_name: null,
    avatar_url: null,
    about: null,
    nip05_handle: null,
    owner_pubkey: OWNER_PUBKEY,
  }));
  handlers.set("start_managed_agent", () => rawPendingAgent());

  function Surface() {
    const actions = useManagedAgentActions();
    return createElement(UnifiedAgentsSection, sectionProps(actions));
  }

  render(
    createElement(
      QueryClientProvider,
      { client },
      createElement(CommunitiesProvider, null, createElement(Surface)),
    ),
  );

  const retry = await screen.findByRole("button", {
    name: "Retry Agent Enrollment",
  });
  await act(async () => fireEvent.click(retry));

  await waitFor(() =>
    assert.equal(
      commands.filter(([command]) => command === "start_managed_agent").length,
      1,
    ),
  );
  const starts = commands.filter(
    ([command]) => command === "start_managed_agent",
  );
  assert.deepEqual(starts[0][1], {
    expectedRelayUrl: RELAY_URL,
    expectedSignerPubkey: OWNER_PUBKEY,
    pubkey: AGENT_PUBKEY,
    replayFloorUnix: null,
  });

  const profileStarts = [];
  const profile = renderHook(
    () =>
      useAgentLifecycleActions({
        availability,
        channels: [],
        managedAgent: agent,
        relayAgents: [],
        startManagedAgent: async (input) => {
          profileStarts.push(input);
        },
        stopManagedAgent: async () => {},
      }),
    {
      wrapper: ({ children }) =>
        createElement(
          QueryClientProvider,
          { client },
          createElement(CommunitiesProvider, null, children),
        ),
    },
  );
  await act(async () => profile.result.current.handleAgentPrimaryAction());
  assert.deepEqual(profileStarts, [
    {
      expectedRelayUrl: RELAY_URL,
      expectedSignerPubkey: OWNER_PUBKEY,
      pubkey: AGENT_PUBKEY,
    },
  ]);
}

for (const availability of ["online", "away"]) {
  test(`unattached ${availability} provider identity keeps scoped attest recovery`, async () => {
    await exerciseRecovery(availability);
  });
}
