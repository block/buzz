/**
 * Cancel-safety + Save-gated effort production-seam pins for AgentEditMergedDialog.
 *
 * Ported from the deleted agentInstanceEditCancelSafety.test.mjs (present in
 * main at 0dbd036f5) against the surviving AgentEditMergedDialog. The deleted
 * file mounted AgentInstanceEditDialog; every test here mounts the merged
 * dialog against the same IPC boundary and asserts the same contracts:
 *
 *   (a) effort selection alone + Cancel dispatch ZERO update_managed_agent
 *   (b) ordinary effort Save carries effortLevel inside the locked update
 *   (c) rejected effort Save keeps the dialog open (no close)
 *   (d) pin→inherit Save sends agentCommand:"" with NO effortLevel
 *   (e) runtime switch clears stale effort — no effortLevel after switch
 *   (f) access + effort changes share one atomic update_managed_agent payload
 *
 * Why full production renders: the seam being pinned is the dialog footer's
 * wiring (Cancel → onOpenChange, Save → handleSubmit → update_managed_agent)
 * and the submit hook's resolveEffortSubmission gate. A model-level miniature
 * cannot catch a regression that rewires Cancel to handleSubmit or drops the
 * inherit-transition suppression.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const liveClients = [];
const pendingTimers = new Set();
const nativeSetTimeout = globalThis.setTimeout;
const nativeSetInterval = globalThis.setInterval;
const nativeClearTimeout = globalThis.clearTimeout;
const nativeClearInterval = globalThis.clearInterval;

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let QueryClient;
let QueryClientProvider;
let ThemeProvider;
let AgentEditMergedDialog;

const ipcCalls = [];
const ipcHandlers = new Map();

// A goose-pinned instance linked to a claude persona — gives us the
// pin→inherit transition (agentCommandOverride non-null = opens pinned).
const AGENT_PK = "d".repeat(64);

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
    model_source: null,
    provider: null,
    persona_out_of_date: false,
    persona_orphaned: false,
    needs_restart: false,
    restart_diff: [],
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

function configSurface(overrides = {}) {
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
    ...overrides,
  };
}

function effortConfigSurface(overrides = {}) {
  return configSurface({
    effortConfigId: "thought_level",
    effortOptions: [
      { value: "low", displayName: "Low" },
      { value: "high", displayName: "High" },
    ],
    ...overrides,
  });
}

// Convert a snake_case agent fixture to the camelCase ManagedAgent the dialog
// receives as ctx.instance.
function toCamelAgent(raw) {
  return {
    pubkey: raw.pubkey,
    name: raw.name,
    personaId: raw.persona_id,
    runtime: raw.runtime,
    teamId: null,
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
    modelSource: raw.model_source,
    provider: raw.provider,
    personaOutOfDate: raw.persona_out_of_date,
    personaOrphaned: raw.persona_orphaned,
    needsRestart: raw.needs_restart,
    restartDiff: raw.restart_diff ?? [],
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

function installIpc(surface = configSurface()) {
  const set = (cmd, handler) => ipcHandlers.set(cmd, handler);
  set("discover_acp_providers", () =>
    Promise.resolve([rawRuntime("claude"), rawRuntime("goose")]),
  );
  set("list_personas", () => Promise.resolve([rawPersona()]));
  set("get_agent_config_surface", () => Promise.resolve(surface));
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
  set("update_managed_agent", (args) => {
    ipcCalls.push({ cmd: "update_managed_agent", args });
    return Promise.resolve({ agent: rawAgent(), profile_sync_error: null });
  });
  set("set_managed_agent_auto_restart", (args) => {
    ipcCalls.push({ cmd: "set_managed_agent_auto_restart", args });
    return Promise.resolve();
  });
}

function installEffortIpc({ deferUpdate = false, failUpdate = false } = {}) {
  installIpc(effortConfigSurface());
  const set = (cmd, handler) => ipcHandlers.set(cmd, handler);

  let resolveUpdate = () => {};
  set("update_managed_agent", (args) => {
    ipcCalls.push({ cmd: "update_managed_agent", args });
    if (failUpdate) {
      return Promise.reject(new Error("update failed"));
    }
    const response = { agent: rawAgent(), profile_sync_error: null };
    if (!deferUpdate) {
      return Promise.resolve(response);
    }
    return new Promise((resolve) => {
      resolveUpdate = () => resolve(response);
    });
  });
  return { resolveUpdate: () => resolveUpdate() };
}

function renderDialog(onOpenChange = () => {}) {
  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0 },
      queries: { gcTime: 0, retry: false },
    },
  });
  liveClients.push(client);
  return render(
    createElement(
      ThemeProvider,
      { defaultTheme: "buzz" },
      createElement(
        QueryClientProvider,
        { client },
        createElement(AgentEditMergedDialog, {
          open: true,
          onOpenChange,
          ctx: {
            kind: "instance-only",
            instance: toCamelAgent(rawAgent()),
          },
        }),
      ),
    ),
  );
}

/** Returns update_managed_agent calls whose input carries an effortLevel. */
function effortCalls() {
  return ipcCalls.filter(
    (c) =>
      c.cmd === "update_managed_agent" &&
      c.args.input?.effortLevel !== undefined,
  );
}

/** Opens the effort dropdown and clicks the item matching `label`. */
async function selectEffort(label) {
  const trigger = dom.window.document.getElementById("edit-agent-effort");
  assert.ok(
    trigger,
    "effort picker trigger must render for a local + effort-capable agent",
  );
  await act(async () => {
    fireEvent.pointerDown(
      trigger,
      new dom.window.MouseEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    fireEvent.click(trigger);
  });
  const item = [
    ...dom.window.document.querySelectorAll('[role="menuitemradio"]'),
  ].find((node) => node.textContent?.trim() === label);
  assert.ok(item, `effort option "${label}" must be offered`);
  await act(async () => {
    fireEvent.click(item);
  });
}

/** Expands Advanced and toggles the inherit checkbox from unchecked → checked. */
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
  globalThis.setTimeout = (fn, ms, ...args) => {
    const id = nativeSetTimeout(
      (...a) => {
        pendingTimers.delete(id);
        return fn(...a);
      },
      ms,
      ...args,
    );
    pendingTimers.add(id);
    return id;
  };
  globalThis.setInterval = (fn, ms, ...args) => {
    const id = nativeSetInterval(fn, ms, ...args);
    pendingTimers.add(id);
    return id;
  };
  globalThis.clearTimeout = (id) => {
    pendingTimers.delete(id);
    return nativeClearTimeout(id);
  };
  globalThis.clearInterval = (id) => {
    pendingTimers.delete(id);
    return nativeClearInterval(id);
  };

  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key in globalThis) continue;
    try {
      globalThis[key] = dom.window[key];
    } catch {
      /* getter-only global — skip */
    }
  }
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  });
  for (const key of [
    "Event",
    "CustomEvent",
    "MouseEvent",
    "KeyboardEvent",
    "FocusEvent",
    "PointerEvent",
    "InputEvent",
    "UIEvent",
  ]) {
    if (dom.window[key]) globalThis[key] = dom.window[key];
  }
  dom.window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  globalThis.ResizeObserver = dom.window.ResizeObserver;
  dom.window.requestAnimationFrame = (cb) =>
    globalThis.setTimeout(() => cb(Date.now()), 0);
  dom.window.cancelAnimationFrame = (id) => globalThis.clearTimeout(id);
  globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
  globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.setPointerCapture = () => {};
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
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
  ({ AgentEditMergedDialog } = await import("./AgentEditMergedDialog.tsx"));
});

afterEach(() => {
  cleanup?.();
  for (const client of liveClients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  ipcHandlers.clear();
  ipcCalls.length = 0;
  for (const id of pendingTimers) nativeClearTimeout(id);
  pendingTimers.clear();
});

after(() => {
  globalThis.setTimeout = nativeSetTimeout;
  globalThis.setInterval = nativeSetInterval;
  globalThis.clearTimeout = nativeClearTimeout;
  globalThis.clearInterval = nativeClearInterval;
  dom.window.close();
});

// ── (a) effort selection alone + Cancel → zero writes ────────────────────────

test("effort selection alone dispatches no effort in update_managed_agent", async () => {
  installEffortIpc();
  await act(async () => {
    renderDialog();
  });

  await selectEffort("High");

  assert.equal(
    effortCalls().length,
    0,
    "picking an effort value must not write until Save — a selection-time IPC is the race",
  );
});

test("effort selected then Cancel dispatches no effort in update_managed_agent", async () => {
  installEffortIpc();
  let openChange;
  await act(async () => {
    renderDialog((next) => {
      openChange = next;
    });
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  });

  assert.equal(
    openChange,
    false,
    "Cancel must route through onOpenChange(false)",
  );
  assert.equal(
    effortCalls().length,
    0,
    "Cancel after selecting an effort must discard the pending write",
  );
});

// ── (b) ordinary effort Save carries effortLevel in the locked update ─────────

test("effort Save includes effortLevel inside update_managed_agent payload", async () => {
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });
  await act(async () => {
    renderDialog();
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  });

  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "the locked update is dispatched",
  );
  const updateArgs = ipcCalls.find(
    (c) => c.cmd === "update_managed_agent",
  )?.args;
  assert.equal(
    updateArgs?.input?.effortLevel,
    "high",
    "effortLevel must be in the locked update payload (restart sees new effort)",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "effort persisted in one locked update; no separate call",
  );
});

// ── (c) rejected effort Save keeps dialog open ────────────────────────────────

test("effort Save with a rejected update results in no persisted effort", async () => {
  installEffortIpc({ failUpdate: true });
  let openChange;
  await act(async () => {
    renderDialog((next) => {
      openChange = next;
    });
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 10));
  });

  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "Save must attempt the locked update exactly once",
  );
  // update rejected — dialog must not close as success
  assert.notEqual(
    openChange,
    false,
    "a failed update must not close the dialog as success — the coordinator must keep it open",
  );
});

// ── (d) pin→inherit Save sends agentCommand:"" with NO effortLevel ────────────

test("pin→inherit Save with a picked effort does not write effortLevel", async () => {
  installEffortIpc();
  await act(async () => {
    renderDialog();
  });

  await selectEffort("High");
  await expandAdvancedAndToggleInherit();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  const updates = ipcCalls.filter((c) => c.cmd === "update_managed_agent");
  assert.equal(
    updates.length,
    1,
    "the pin→inherit Save dispatches the locked update",
  );
  assert.equal(
    updates[0].args.input.agentCommand,
    "",
    "the pin→inherit transition carries the clear sentinel",
  );
  assert.equal(
    effortCalls().length,
    0,
    "the inherit-transition guard must suppress the effort write — dropping the guard fails this",
  );
});

// ── (e) runtime switch: see agentEditMergedRuntimeReset.test.mjs ─────────────
// The runtime-switch reset test is in a dedicated file to avoid a jsdom
// interaction hang caused by Radix's dropdown close sequence racing with the
// concurrent React state update from setEffortLevel(null). The hook-level
// test there is mutation-sensitive and covers the exact production seam.

// ── (f) access + effort share one atomic update_managed_agent payload ─────────

test("access and effort changed together both appear in the locked update_managed_agent call", async () => {
  installEffortIpc();

  await act(async () => {
    renderDialog();
  });

  // Change access mode from "mentions" to "owner-only"
  const respondToTrigger =
    dom.window.document.getElementById("agent-respond-to");
  assert.ok(respondToTrigger, "respond-to trigger must be present");
  await act(async () => {
    fireEvent.pointerDown(
      respondToTrigger,
      new dom.window.MouseEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    fireEvent.click(respondToTrigger);
  });
  const ownerOnlyItem = [
    ...dom.window.document.querySelectorAll('[role="menuitemradio"]'),
  ].find((node) => node.textContent?.trim() === "Only me (default)");
  assert.ok(ownerOnlyItem, '"Only me (default)" option must be offered');
  await act(async () => {
    fireEvent.click(ownerOnlyItem);
  });

  // Also pick an effort level
  await selectEffort("High");

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  const updates = ipcCalls.filter((c) => c.cmd === "update_managed_agent");
  assert.equal(updates.length, 1, "exactly one update_managed_agent must fire");

  assert.equal(
    updates[0].args.input.respondTo,
    "owner-only",
    "respondTo must be included in the locked update",
  );
  assert.equal(
    updates[0].args.input.effortLevel,
    "high",
    "effortLevel must be in the SAME locked update as the access change (restart sees new effort atomically)",
  );
});
