/**
 * Production-seam regression for team deploy (#5694 / PR review).
 *
 * Helper tests can stay green while the dialog ignores their outputs. This
 * suite renders AddTeamToChannelDialog, drives Deploy, and asserts the
 * create_managed_agent IPC that provisionChannelManagedAgent sends.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const PINNED_ARGS = ["--acp", "--profile", "pinned"];
const UNKNOWN_PIN = "/home/user/.local/bin/openclaw-acp-buzz";

const CHANNEL = {
  id: "chan-1",
  name: "general",
  channel_type: "stream",
  visibility: "open",
  description: "",
  topic: null,
  purpose: null,
  member_count: 1,
  member_pubkeys: [],
  last_message_at: null,
  archived_at: null,
  participants: [],
  participant_pubkeys: [],
  is_member: true,
  ttl_seconds: null,
  ttl_deadline: null,
};

const TEAM = {
  id: "team-1",
  name: "Ops",
  description: "",
  personaIds: ["p-1"],
  avatarUrl: null,
};

const PERSONA = {
  id: "p-1",
  displayName: "Dolomite",
  systemPrompt: "prompt",
  model: null,
  runtime: "goose",
  avatarUrl: null,
  envVars: {},
  isBuiltIn: false,
};

const GOOSE_RUNTIME = {
  id: "goose",
  label: "Goose",
  avatar_url: "",
  availability: "available",
  command: "goose-cmd",
  binary_path: "/bin/goose",
  default_args: ["--acp"],
  mcp_command: "goose-mcp",
  install_hint: "",
  install_instructions_url: "",
  can_auto_install: false,
  requires_external_cli: false,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "not_required" },
  source: "builtin",
};

const BUZZ_AGENT_RUNTIME = {
  ...GOOSE_RUNTIME,
  id: "buzz-agent",
  label: "Buzz Agent",
  command: "buzz-agent-cmd",
  mcp_command: null,
};

const CLAUDE_RUNTIME = {
  ...GOOSE_RUNTIME,
  id: "claude",
  label: "Claude",
  command: "claude-agent-acp",
  mcp_command: null,
};

function rawPinnedAgent({
  commandOverride = "goose-cmd",
  args = PINNED_ARGS,
} = {}) {
  return {
    pubkey: "aa".repeat(32),
    name: "Dolomite",
    persona_id: "p-1",
    runtime: "goose",
    team_id: null,
    relay_url: "wss://relay.example",
    acp_command: "buzz-acp",
    agent_command: "goose-cmd",
    agent_command_override: commandOverride,
    agent_args: args,
    mcp_command: "ignored-by-create",
    turn_timeout_seconds: 120,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: "prompt",
    avatar_url: null,
    model: null,
    provider: null,
    persona_out_of_date: false,
    persona_orphaned: false,
    needs_restart: false,
    env_vars: {},
    status: "stopped",
    pid: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    last_error_code: null,
    log_path: "/tmp/agent.log",
    start_on_app_launch: false,
    backend: { type: "local" },
    backend_agent_id: null,
    respond_to: "owner-only",
    respond_to_allowlist: [],
  };
}

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

let act;
let cleanup;
let createElement;
let fireEvent;
let render;
let waitFor;
let screen;
let ThemeProvider;
let CommunitiesProvider;
let AddTeamToChannelDialog;

let createManagedAgentCalls;
let listManagedAgentsDeferred;
let listManagedAgentsResult;

before(async () => {
  const skipWindowKeys = new Set([
    "window",
    "document",
    "self",
    "top",
    "parent",
    "frames",
    "globalThis",
    "location",
    "navigator",
  ]);
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (skipWindowKeys.has(key) || key in globalThis) {
      continue;
    }
    try {
      Object.defineProperty(globalThis, key, {
        configurable: true,
        get: () => dom.window[key],
      });
    } catch {
      // Some window properties are not configurable.
    }
  }
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: dom.window.document,
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: dom.window,
  });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.isTauri = true;
  globalThis.Event = dom.window.Event;
  globalThis.CustomEvent = dom.window.CustomEvent;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.localStorage.setItem("buzz-follow-system", "false");
  dom.window.isTauri = true;
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.ResizeObserver = class {
    observe() {}
    disconnect() {}
    unobserve() {}
  };
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.setPointerCapture = () => {};
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.animate = () => ({
    cancel() {},
    currentTime: 0,
    finished: Promise.resolve(),
    play() {},
    playbackRate: 1,
    reverse() {},
  });

  ({ act, cleanup, fireEvent, render, screen, waitFor } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider"));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ AddTeamToChannelDialog } = await import("./AddTeamToChannelDialog.tsx"));
});

after(() => dom.window.close());

function installTauriBridge() {
  createManagedAgentCalls = [];
  listManagedAgentsResult = [rawPinnedAgent()];
  let resolveList;
  let rejectList;
  listManagedAgentsDeferred = new Promise((resolve, reject) => {
    resolveList = resolve;
    rejectList = reject;
  });
  listManagedAgentsDeferred.resolve = resolveList;
  listManagedAgentsDeferred.reject = rejectList;

  const internals = {
    invoke(command, args) {
      switch (command) {
        case "list_managed_agents":
          return listManagedAgentsDeferred.then(() => listManagedAgentsResult);
        case "discover_acp_providers":
          return Promise.resolve([
            GOOSE_RUNTIME,
            CLAUDE_RUNTIME,
            BUZZ_AGENT_RUNTIME,
          ]);
        case "get_global_agent_config":
          return Promise.resolve({
            env_vars: {},
            provider: null,
            model: null,
            preferred_runtime: null,
          });
        case "get_identity":
          return Promise.resolve({
            pubkey: "11".repeat(32),
            display_name: "Test Owner",
          });
        case "get_channels":
          return Promise.resolve({
            hash: "h1",
            channels: [CHANNEL],
            last_messages: {},
          });
        case "get_channel_members":
          return Promise.resolve({ members: [] });
        case "create_managed_agent":
          createManagedAgentCalls.push(args);
          return Promise.resolve({
            agent: {
              ...rawPinnedAgent(),
              pubkey: "bb".repeat(32),
              team_id: "team-1",
              agent_command: args.input.agentCommand,
              agent_command_override: args.input.harnessOverride
                ? args.input.agentCommand
                : null,
              agent_args: args.input.agentArgs ?? [],
            },
            private_key_nsec: "nsec1test",
            profile_sync_error: null,
            spawn_error: null,
          });
        case "add_channel_members":
          return Promise.resolve({
            added: args.pubkeys,
            errors: [],
          });
        case "start_managed_agent":
          return Promise.resolve({
            ...rawPinnedAgent(),
            pubkey: args.pubkey,
            status: "running",
          });
        default:
          return Promise.resolve(null);
      }
    },
  };
  globalThis.__TAURI_INTERNALS__ = internals;
  dom.window.__TAURI_INTERNALS__ = internals;
}

beforeEach(() => {
  cleanup?.();
  // The full desktop suite shares one process. Re-bind jsdom + Tauri so a
  // prior file cannot leave Deploy stuck disabled (CI failed this waitFor
  // at the default 1s timeout after 4912 other tests).
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: dom.window.document,
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: dom.window,
  });
  globalThis.isTauri = true;
  dom.window.isTauri = true;
  dom.window.document.hasFocus = () => true;
  dom.window.localStorage.clear();
  dom.window.localStorage.setItem("buzz-follow-system", "false");
  dom.window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-1",
        name: "Test Community",
        relayUrl: "wss://relay.example",
        pubkey: "11".repeat(32),
        addedAt: "2026-01-01T00:00:00Z",
      },
    ]),
  );
  dom.window.localStorage.setItem("buzz-active-community-id", "community-1");
  installTauriBridge();
});

afterEach(() => {
  cleanup?.();
});

function renderDialog() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const view = render(
    createElement(
      ThemeProvider,
      null,
      createElement(
        CommunitiesProvider,
        null,
        createElement(
          QueryClientProvider,
          { client: queryClient },
          createElement(AddTeamToChannelDialog, {
            open: true,
            onOpenChange() {},
            onDeployed() {},
            team: TEAM,
            personas: [PERSONA],
          }),
        ),
      ),
    ),
  );
  return { queryClient, view };
}

function deployButton() {
  return screen.getByRole("button", { name: /Deploy 1 agent/i });
}

const SUITE_WAIT = { timeout: 5000 };

async function resolveManagedAgents() {
  await act(async () => {
    listManagedAgentsDeferred.resolve();
    await listManagedAgentsDeferred;
  });
}

async function rejectManagedAgents(error) {
  await act(async () => {
    listManagedAgentsDeferred.reject(error);
    await listManagedAgentsDeferred.catch(() => undefined);
  });
}

async function waitForNonManagedDeployGates(queryClient) {
  await waitFor(() => {
    const channelSelect = screen.getByLabelText(/^Channel$/);
    assert.equal(channelSelect.value, "chan-1");
    assert.equal(
      queryClient.getQueryState(["acp-runtimes"])?.status,
      "success",
    );
  }, SUITE_WAIT);
}

test("pending managed-agents query keeps Deploy disabled", async () => {
  const { queryClient } = renderDialog();

  await waitForNonManagedDeployGates(queryClient);

  assert.equal(deployButton().disabled, true);
  assert.equal(createManagedAgentCalls.length, 0);

  fireEvent.click(deployButton());
  await act(async () => {
    await Promise.resolve();
  });
  assert.equal(createManagedAgentCalls.length, 0);
});

test("managed-agents query error is surfaced and creates nothing", async () => {
  const { queryClient } = renderDialog();

  await waitForNonManagedDeployGates(queryClient);
  await rejectManagedAgents(new Error("Managed agent lookup failed."));

  await waitFor(() => {
    assert.ok(screen.getByText("Managed agent lookup failed."));
  }, SUITE_WAIT);
  assert.equal(deployButton().disabled, true);
  fireEvent.click(deployButton());
  await act(async () => {
    await Promise.resolve();
  });
  assert.equal(createManagedAgentCalls.length, 0);
});

test("Deploy preserves a full-path pin through create_managed_agent", async () => {
  listManagedAgentsResult = [
    rawPinnedAgent({ commandOverride: "/opt/homebrew/bin/goose-cmd" }),
  ];
  renderDialog();

  await waitFor(() => {
    assert.equal(deployButton().disabled, true);
  }, SUITE_WAIT);

  await resolveManagedAgents();

  await waitFor(() => {
    const channelSelect = screen.getByLabelText(/^Channel$/);
    assert.equal(channelSelect.value, "chan-1");
    assert.ok(screen.getByText("Goose"));
    assert.equal(deployButton().disabled, false);
  }, SUITE_WAIT);

  fireEvent.click(deployButton());

  await waitFor(() => {
    assert.equal(createManagedAgentCalls.length, 1);
  }, SUITE_WAIT);

  const input = createManagedAgentCalls[0].input;
  assert.equal(input.agentCommand, "/opt/homebrew/bin/goose-cmd");
  assert.equal(input.harnessOverride, true);
  assert.deepEqual(input.agentArgs, PINNED_ARGS);
  assert.equal(input.personaId, "p-1");
  assert.equal(input.teamId, "team-1");
  assert.equal(
    input.mcpCommand,
    "goose-mcp",
    "create must use catalog MCP, not the source agent's ignored mcp_command",
  );
});

test("Deploy preserves a supported alias through create_managed_agent", async () => {
  listManagedAgentsResult = [
    rawPinnedAgent({
      commandOverride: "/opt/bin/claude-code-acp",
      args: ["--permission-mode", "plan"],
    }),
  ];
  renderDialog();

  await resolveManagedAgents();

  await waitFor(() => {
    assert.ok(screen.getByText("Claude"));
    assert.equal(deployButton().disabled, false);
  }, SUITE_WAIT);

  fireEvent.click(deployButton());
  await waitFor(() => {
    assert.equal(createManagedAgentCalls.length, 1);
  }, SUITE_WAIT);

  const input = createManagedAgentCalls[0].input;
  assert.equal(input.agentCommand, "/opt/bin/claude-code-acp");
  assert.equal(input.harnessOverride, true);
  assert.deepEqual(input.agentArgs, ["--permission-mode", "plan"]);
});

test("conflicting personal runtimes require setup and create nothing", async () => {
  listManagedAgentsResult = [
    rawPinnedAgent({
      commandOverride: "goose-cmd",
      args: ["--profile", "goose-work"],
    }),
    {
      ...rawPinnedAgent({
        commandOverride: "codex-acp",
        args: ["--sandbox", "workspace-write"],
      }),
      pubkey: "cc".repeat(32),
      name: "Dolomite 2",
    },
  ];
  renderDialog();

  await resolveManagedAgents();

  await waitFor(() => {
    assert.ok(screen.getAllByText(/different runtime settings/i).length > 0);
  }, SUITE_WAIT);
  assert.equal(deployButton().disabled, true);
  fireEvent.click(deployButton());
  await act(async () => {
    await Promise.resolve();
  });
  assert.equal(createManagedAgentCalls.length, 0);
});

test("unresolved pin leaves Deploy disabled and creates nothing", async () => {
  listManagedAgentsResult = [rawPinnedAgent({ commandOverride: UNKNOWN_PIN })];
  renderDialog();

  await resolveManagedAgents();

  await waitFor(() => {
    assert.ok(screen.getAllByText(/Setup required/i).length > 0);
  }, SUITE_WAIT);

  assert.equal(deployButton().disabled, true);
  fireEvent.click(deployButton());
  await act(async () => {
    await Promise.resolve();
  });
  assert.equal(createManagedAgentCalls.length, 0);
});
