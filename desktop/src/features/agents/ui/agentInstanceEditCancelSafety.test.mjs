/**
 * Cancel-safety acceptance pin (production seam).
 *
 * The pin→inherit effort/runtime clear is derived entirely inside the backend's
 * locked save, keyed off the `agentCommand: ""` sentinel that
 * `resolveAgentCommandUpdate` produces at SUBMIT. Cancel-safety is therefore a
 * UI-wiring invariant: toggling the inherit checkbox mutates only local dialog
 * state, and the real Cancel button must route to `onOpenChange`, never to the
 * submit path — so no `update_managed_agent` (and thus no column/env clear) is
 * ever dispatched when the user backs out.
 *
 * Why a full production render rather than a hand-written miniature: the seam
 * being pinned is the DIALOG FOOTER's wiring (Cancel → handleOpenChange, Save →
 * handleSubmit → update_managed_agent). A miniature that re-implements a fake
 * Cancel/Save cannot catch a regression that rewires the real Cancel button to
 * handleSubmit. This test mounts the actual `AgentInstanceEditDialog`, expands
 * Advanced, toggles the inherit checkbox, clicks the REAL Cancel button, and
 * asserts the mocked `update_managed_agent` IPC boundary recorded zero calls —
 * so rewiring Cancel to handleSubmit() makes it fail. The companion test clicks
 * the REAL Save button and asserts the same boundary receives exactly one call
 * carrying the `agentCommand: ""` inherit sentinel.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Track every QueryClient so afterEach can cancel pending queries + clear the
// cache — react-query's default gcTime otherwise schedules timers that outlive
// the test and stall the shared `pnpm test` process.
const clients = [];

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let QueryClient;
let QueryClientProvider;
let ThemeProvider;
let AgentInstanceEditDialog;

// Records every Tauri command invocation the mounted dialog issues; unmocked
// commands reject so a new IPC dependency surfaces as a loud failure.
const ipcCalls = [];
const ipcHandlers = new Map();

const AGENT_PK = "d".repeat(64);

// A goose-pinned instance linked to a claude persona — the pin→inherit
// transition. `agentCommandOverride` non-null means it opens PINNED (inherit
// checkbox unchecked); toggling inherit ON produces the `agentCommand: ""`
// clear sentinel at submit. Claude persona keeps the prospective runtime
// credential-free so the Save sanity case is enabled.
function rawAgent(overrides = {}) {
  return {
    pubkey: AGENT_PK,
    name: "pinned-instance",
    persona_id: "p1",
    runtime: "goose",
    relay_url: "wss://relay.example",
    acp_command: "acp",
    agent_command: "goose",
    agent_command_override: "goose",
    agent_args: [],
    mcp_command: "mcp",
    turn_timeout_seconds: 300,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: null,
    avatar_url: null,
    model: null,
    provider: null,
    persona_out_of_date: false,
    persona_orphaned: false,
    needs_restart: false,
    env_vars: {},
    status: "running",
    pid: 1234,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    last_error_code: null,
    log_path: "/tmp/agent.log",
    start_on_app_launch: false,
    auto_restart_on_config_change: true,
    backend: { type: "local" },
    backend_agent_id: null,
    respond_to: "mentions",
    respond_to_allowlist: [],
    ...overrides,
  };
}

function rawPersona(overrides = {}) {
  return {
    id: "p1",
    display_name: "Scribe",
    avatar_url: null,
    system_prompt: "be helpful",
    runtime: "claude",
    model: null,
    provider: null,
    name_pool: [],
    is_builtin: false,
    is_active: true,
    shared: false,
    source_team: null,
    env_vars: {},
    respond_to: null,
    respond_to_allowlist: [],
    parallelism: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function rawRuntime(id, overrides = {}) {
  return {
    id,
    label: id,
    avatar_url: "",
    availability: "available",
    command: id,
    binary_path: `/usr/local/bin/${id}`,
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "logged_in" },
    source: "builtin",
    ...overrides,
  };
}

function configSurface() {
  return {
    runtimeId: "goose",
    runtimeLabel: "goose",
    isPreSpawn: false,
    normalized: {
      model: null,
      provider: null,
      mode: null,
      thinkingEffort: null,
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    extensions: [],
    sources: {
      acpNative: "notApplicable",
      acpConfigOptions: "notApplicable",
      envVars: "available",
      configFile: "notApplicable",
      configFilePath: null,
      mcpConfigFilePath: null,
    },
  };
}

function installIpc() {
  const set = (cmd, handler) => ipcHandlers.set(cmd, handler);
  set("discover_acp_providers", () =>
    Promise.resolve([rawRuntime("claude"), rawRuntime("goose")]),
  );
  set("list_personas", () => Promise.resolve([rawPersona()]));
  set("get_agent_config_surface", () => Promise.resolve(configSurface()));
  set("get_global_agent_config", () =>
    Promise.resolve({
      env_vars: {},
      provider: null,
      model: null,
      preferred_runtime: null,
    }),
  );
  set("get_baked_build_env", () => Promise.resolve([]));
  set("get_baked_build_env_keys", () => Promise.resolve([]));
  set("get_runtime_file_config", () => Promise.resolve(null));
  set("agent_access_owner_only", () => Promise.resolve(false));
  set("discover_agent_models", () =>
    Promise.resolve({
      agentName: "goose",
      agentVersion: "1.0",
      models: [],
      agentDefaultModel: null,
      selectedModel: null,
      supportsSwitching: false,
    }),
  );
  // The persistence boundary under test. Returns a valid response so the
  // mutation's onSuccess/onSettled cache updates don't throw.
  set("update_managed_agent", (args) => {
    ipcCalls.push({ cmd: "update_managed_agent", args });
    return Promise.resolve({ agent: rawAgent(), profile_sync_error: null });
  });
}

function renderDialog(onOpenChange) {
  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0 },
      queries: { gcTime: 0, retry: false },
    },
  });
  clients.push(client);
  return render(
    createElement(
      ThemeProvider,
      { defaultTheme: "buzz" },
      createElement(
        QueryClientProvider,
        { client },
        createElement(AgentInstanceEditDialog, {
          agent: { ...toCamelAgent(rawAgent()) },
          open: true,
          onOpenChange,
          onUpdated: () => {},
        }),
      ),
    ),
  );
}

// The dialog takes a camelCase ManagedAgent prop (the caller maps it via
// fromRawManagedAgent). Only the fields the dialog reads are needed.
function toCamelAgent(raw) {
  return {
    pubkey: raw.pubkey,
    name: raw.name,
    personaId: raw.persona_id,
    runtime: raw.runtime,
    relayUrl: raw.relay_url,
    acpCommand: raw.acp_command,
    agentCommand: raw.agent_command,
    agentCommandOverride: raw.agent_command_override,
    agentArgs: raw.agent_args,
    mcpCommand: raw.mcp_command,
    turnTimeoutSeconds: raw.turn_timeout_seconds,
    idleTimeoutSeconds: raw.idle_timeout_seconds,
    maxTurnDurationSeconds: raw.max_turn_duration_seconds,
    parallelism: raw.parallelism,
    systemPrompt: raw.system_prompt,
    avatarUrl: raw.avatar_url,
    model: raw.model,
    modelSource: null,
    provider: raw.provider,
    personaOutOfDate: raw.persona_out_of_date,
    personaOrphaned: raw.persona_orphaned,
    needsRestart: raw.needs_restart,
    restartDiff: [],
    envVars: raw.env_vars,
    status: raw.status,
    pid: raw.pid,
    createdAt: raw.created_at,
    updatedAt: raw.updated_at,
    lastStartedAt: raw.last_started_at,
    lastStoppedAt: raw.last_stopped_at,
    lastExitCode: raw.last_exit_code,
    lastError: raw.last_error,
    lastErrorCode: raw.last_error_code,
    logPath: raw.log_path,
    startOnAppLaunch: raw.start_on_app_launch,
    autoRestartOnConfigChange: raw.auto_restart_on_config_change,
    backend: raw.backend,
    backendAgentId: raw.backend_agent_id,
    respondTo: raw.respond_to,
    respondToAllowlist: raw.respond_to_allowlist,
  };
}

async function expandAdvancedAndToggleInherit() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /Advanced/ }));
  });
  const checkbox = dom.window.document.getElementById(
    "edit-agent-inherit-harness",
  );
  assert.ok(
    checkbox,
    "inherit checkbox must render for a persona-linked agent inside Advanced",
  );
  assert.equal(
    checkbox.checked,
    false,
    "a harness-pinned agent must open with inherit unchecked",
  );
  await act(async () => {
    fireEvent.click(checkbox);
  });
  assert.equal(checkbox.checked, true, "inherit toggle must flip to checked");
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  // Radix + testing-library reach for a broad set of DOM constructors and
  // globals off the realm's `globalThis`. Node ships its own incompatible
  // `Event`/`CustomEvent` globals, so JSDOM nodes reject events built from
  // them ("parameter 1 is not of type 'Event'"). Force every DOM constructor
  // and *Event/*Element/Node* binding to JSDOM's, overriding Node's built-ins,
  // so the mounted dialog resolves them all against one realm.
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key === "window" || key === "document" || key === "globalThis")
      continue;
    const value = dom.window[key];
    if (
      typeof value === "function" &&
      /^(HTML|SVG)|Element$|Event$|EventTarget$|^Node|^Document|Observer$/.test(
        key,
      )
    ) {
      globalThis[key] = value;
    }
  }
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });
  // Radix Dialog probes pointer-capture and scrolls focus into view on mount.
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const handler = ipcHandlers.get(cmd);
      if (handler) return handler(args);
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider"));
  ({ AgentInstanceEditDialog } = await import("./AgentInstanceEditDialog.tsx"));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  ipcHandlers.clear();
  ipcCalls.length = 0;
});

after(() => dom.window.close());

test("inherit toggle then Cancel dispatches no update_managed_agent", async () => {
  installIpc();
  let openChange;
  await act(async () => {
    renderDialog((next) => {
      openChange = next;
    });
  });

  await expandAdvancedAndToggleInherit();

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  });

  assert.equal(
    openChange,
    false,
    "Cancel must route through onOpenChange(false)",
  );
  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    0,
    "Cancel after toggling inherit must not dispatch update_managed_agent — rewiring Cancel to handleSubmit() breaks this",
  );
});

test("inherit toggle then Save dispatches the agentCommand:'' inherit sentinel", async () => {
  installIpc();
  await act(async () => {
    renderDialog(() => {});
  });

  await expandAdvancedAndToggleInherit();

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  });

  const updates = ipcCalls.filter((c) => c.cmd === "update_managed_agent");
  assert.equal(updates.length, 1, "Save must dispatch exactly one update");
  assert.equal(
    updates[0].args.input.agentCommand,
    "",
    "Save on the pin→inherit transition must carry the empty-command sentinel the backend clears the column on",
  );
});
