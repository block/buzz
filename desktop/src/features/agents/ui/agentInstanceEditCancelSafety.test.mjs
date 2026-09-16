/**
 * Cancel-safety + Save-gated effort acceptance pins (production seam).
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
 *
 * The effort-write tests (Carl r8 P1 / P3) pin the SUBMIT wiring the pure
 * `resolveEffortSubmission` unit tests cannot reach: they mount the dialog with
 * an effort-capable config surface, drive the REAL effort dropdown, and assert
 * the `update_managed_agent` IPC boundary carries `effortLevel` in its payload.
 * Selection alone and Cancel dispatch no effort write (a selection-time write
 * would fail these); a pin→inherit Save suppresses effortLevel (dropping the
 * inherit-transition guard fails it); and an ordinary effort Save proves
 * `effortLevel` is in the locked update payload so an access-change-triggered
 * restart snapshots the new effort atomically (P3: no separate setter race).
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
  // No-op: the auto-restart setter is a standalone IPC that existing tests
  // never trigger (agent state matches dialog default), but mock it so a
  // test that flips autoRestartOnConfigChange doesn't hit "unmocked command".
  set("set_managed_agent_auto_restart", (args) => {
    ipcCalls.push({ cmd: "set_managed_agent_auto_restart", args });
    return Promise.resolve();
  });
}

// An effort-capable local config surface: the picker renders only when the
// backend is local AND the running session has advertised a `thought_level`
// configId, so these must be present for the effort dropdown to mount.
function effortConfigSurface() {
  return {
    ...configSurface(),
    effortConfigId: "thought_level",
    effortOptions: [
      { value: "low", displayName: "Low" },
      { value: "high", displayName: "High" },
    ],
  };
}

// Installs the effort-capable surface plus a CONTROLLABLE update boundary: the
// tests can prove effort is included in the locked update payload — never on
// selection, never before the update settles. `failUpdate` makes
// `update_managed_agent` reject immediately; `deferUpdate` holds it pending
// until the returned `resolveUpdate()` is called.
// Note: `persist_agent_effort_level` has been removed (PR #4625 pass 3);
// effort is embedded in the locked update payload — no mock needed.
function installEffortIpc({ deferUpdate = false, failUpdate = false } = {}) {
  installIpc();
  const set = (cmd, handler) => ipcHandlers.set(cmd, handler);
  set("get_agent_config_surface", () => Promise.resolve(effortConfigSurface()));

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
  return {
    resolveUpdate: () => resolveUpdate(),
  };
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

// ── Effort write is Save-gated: real dialog wiring, controlled deferred IPC ────

// Opens the effort dropdown (Radix DropdownMenu trigger) and selects the option
// whose visible label matches `label`. Mirrors a real user pick — the seam the
// pure resolveEffortSubmission unit tests never touch.
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

function effortCalls() {
  // Effort is persisted inside the locked update_managed_agent payload
  // (input.effortLevel) — not a separate IPC call. This helper returns
  // update_managed_agent calls that carry an effortLevel in their args.input.
  return ipcCalls.filter(
    (c) =>
      c.cmd === "update_managed_agent" &&
      c.args.input?.effortLevel !== undefined,
  );
}

test("effort selection alone dispatches no effort in update_managed_agent", async () => {
  installEffortIpc();
  await act(async () => {
    renderDialog(() => {});
  });

  await selectEffort("High");

  assert.equal(
    effortCalls().length,
    0,
    "picking an effort value must not write until Save — a selection-time IPC is the r8 race",
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

test("effort Save with a rejected update results in no persisted effort", async () => {
  // With P3, effort is embedded in the locked update payload. A failed update
  // means both the update AND the effort write fail atomically — nothing committed.
  // Assert exactly one update was attempted (no double-dispatch) and no second
  // IPC was fired on failure.
  installEffortIpc({ failUpdate: true });
  await act(async () => {
    renderDialog(() => {});
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  });

  // The locked update was dispatched (with effortLevel in the payload)...
  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "Save must attempt the locked update exactly once",
  );
  // ...but the update rejected, so no persistence occurred and no second
  // IPC was dispatched (no separate effortLevel write to recover from).
  assert.equal(
    ipcCalls.filter((c) => c.cmd !== "update_managed_agent").length,
    0,
    "a failed update must not trigger any secondary IPC calls",
  );
});

test("effort Save includes effortLevel inside update_managed_agent payload", async () => {
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });
  await act(async () => {
    renderDialog(() => {});
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  });

  // The locked update is still pending: no update_managed_agent has resolved yet.
  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "the locked update is dispatched",
  );
  // The update call carries effortLevel in the payload — not a separate IPC.
  const updateArgs = ipcCalls.find(
    (c) => c.cmd === "update_managed_agent",
  )?.args;
  assert.equal(
    updateArgs?.input?.effortLevel,
    "high",
    "effortLevel must be in the locked update payload (P3: restart sees new effort)",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  // Only one update call total — effort was NOT a second standalone IPC.
  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "effort persisted in one locked update; no separate call",
  );
  const effort = effortCalls();
  assert.equal(
    effort.length,
    1,
    "the update that carries effortLevel was dispatched exactly once",
  );
  assert.equal(
    effort[0].args.input.effortLevel,
    "high",
    "the persisted value is the picked effort level",
  );
});

test("pin→inherit Save with a picked effort does not write effortLevel", async () => {
  installEffortIpc();
  await act(async () => {
    renderDialog(() => {});
  });

  await selectEffort("High");
  await expandAdvancedAndToggleInherit();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
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
    "the inherit-transition guard must suppress the effort write so it cannot restore the just-cleared pin — dropping the guard fails this",
  );
});

// ── Composite isSaving gate covers the FULL Save transaction (Carl r9 P1) ────

test("Cancel and Save are disabled while the locked update is in flight", async () => {
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });
  let openChange;
  await act(async () => {
    renderDialog((next) => {
      openChange = next;
    });
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  });

  // While update_managed_agent is still pending, both buttons must be gated.
  const cancelBtn = screen.getByRole("button", { name: "Cancel" });
  const saveBtn = screen.getByRole("button", { name: "Saving..." });
  assert.ok(
    cancelBtn.disabled,
    "Cancel must be disabled while the locked update is pending — keying off updateMutation.isPending alone would leave it open to a window-close race",
  );
  assert.ok(
    saveBtn.disabled,
    "Save must remain disabled (Saving... label) while the locked update is pending",
  );
  assert.equal(openChange, undefined, "dialog must not have closed yet");

  // Resolve and confirm the dialog eventually closes normally.
  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  assert.equal(
    openChange,
    false,
    "dialog closes after the full Save sequence completes",
  );
});

test("Cancel and Save are disabled while the locked update with effort is in flight", async () => {
  // With P3, effort is embedded inside the locked update_managed_agent payload.
  // The isSaving gate must hold Cancel and Save disabled for the ENTIRE locked
  // update duration — there is no separate effort-setter phase after the update.
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });

  let openChange;
  await act(async () => {
    renderDialog((next) => {
      openChange = next;
    });
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  // The locked update is still pending — both buttons must be gated.
  const cancelBtn = screen.getByRole("button", { name: "Cancel" });
  const saveBtn = screen.getByRole("button", { name: "Saving..." });
  assert.ok(
    cancelBtn.disabled,
    "Cancel must be disabled while the locked update (which carries effort) is pending",
  );
  assert.ok(
    saveBtn.disabled,
    "Save must remain disabled (Saving...) while the locked update is pending",
  );
  assert.equal(openChange, undefined, "dialog must not have closed yet");

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  assert.equal(
    openChange,
    false,
    "dialog closes after the locked update with effort resolves",
  );
});

test("duplicate Save is impossible while the locked update with effort is in flight", async () => {
  // With P3, effort is embedded inside the locked update_managed_agent payload,
  // so the only window where a duplicate Save could fire is DURING the locked
  // update itself. The isSaving gate (set at the top of handleSubmit) must
  // remain true for the entire update duration, preventing re-entry.
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });

  await act(async () => {
    renderDialog(() => {});
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    // Let React process the state transition (isSaving=true → Save button gated).
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  // The locked update is still pending. Try to click Save again — it must
  // be disabled, preventing a duplicate dispatch.
  const saveBtn = screen.getByRole("button", { name: "Saving..." });
  await act(async () => {
    fireEvent.click(saveBtn);
  });

  assert.equal(
    ipcCalls.filter((c) => c.cmd === "update_managed_agent").length,
    1,
    "exactly one update_managed_agent must have been dispatched — the isSaving gate prevents a duplicate while the first is pending",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
});

test("access and effort changed together both appear in the locked update_managed_agent call", async () => {
  // P3 regression: when both respondTo (access) and effort change, the
  // update_managed_agent call that stops/restarts the running pairs must carry
  // BOTH fields. Previously effortLevel was written after the update via a
  // separate persistAgentEffortLevel call; the restart therefore snapshotted
  // and launched the old effort. With P3, effort is embedded in the locked
  // update so the restart sees the new value atomically.
  installEffortIpc();

  await act(async () => {
    renderDialog(() => {});
  });

  // Change access mode from initial "mentions" to "owner-only".
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

  // Also pick an effort level.
  await selectEffort("High");

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  const updates = ipcCalls.filter((c) => c.cmd === "update_managed_agent");
  assert.equal(updates.length, 1, "exactly one update_managed_agent must fire");

  // Access change must be in the locked update (restart sees new access).
  assert.equal(
    updates[0].args.input.respondTo,
    "owner-only",
    "respondTo must be included in the locked update",
  );

  // Effort must also be in the same locked update — not a separate IPC call.
  // This is the P3 invariant: the restart that update_managed_agent triggers
  // for the access change must snapshot and launch the NEW effort value.
  assert.equal(
    updates[0].args.input.effortLevel,
    "high",
    "effortLevel must be in the SAME locked update as the access change (P3: restart sees new effort atomically)",
  );
});

test("runtime switch clears touched effort — no effortLevel dispatched after switching runtimes", async () => {
  // Repro for Carl r10: effort selected for the running runtime → runtime
  // switched → Save → stale effort value committed to the new runtime's store,
  // which rejects vocab it never advertised.
  //
  // The fixture uses a non-null pinned originalEffortLevel ("low") so the
  // test is sensitive to the load-bearing guard: `effortTouched.current = false`
  // in handleRuntimeDropdownChange. Without it, resolveEffortSubmission sees
  // effortLevel=null ≠ originalEffortLevel="low" and emits an unintended clear
  // write. The secondary guard (setEffortLevel(null)) alone does not suppress.
  installEffortIpc();
  const set = (cmd, handler) => ipcHandlers.set(cmd, handler);
  // Override the config surface with a non-null pinned effort so the original
  // value is "low" — the critical case for the effortTouched guard.
  set("get_agent_config_surface", () =>
    Promise.resolve({
      ...effortConfigSurface(),
      normalized: {
        ...effortConfigSurface().normalized,
        thinkingEffort: {
          value: "low",
          origin: "agentRecord",
          writeVia: "standalone",
          overriddenValue: null,
          overriddenOrigin: null,
          isRequired: false,
        },
      },
    }),
  );
  set("update_managed_agent", (args) => {
    ipcCalls.push({ cmd: "update_managed_agent", args });
    return Promise.resolve({ agent: rawAgent(), profile_sync_error: null });
  });
  await act(async () => {
    renderDialog(() => {});
  });

  // Select an effort level while the effort picker is visible (goose session).
  await selectEffort("High");

  // Switch the runtime to a different entry. The config surface is only valid
  // for the running session; switching must clear effortTouched so Save does
  // not write a stale vocab value to the new runtime.
  const runtimeTrigger =
    dom.window.document.getElementById("edit-agent-runtime");
  assert.ok(runtimeTrigger, "runtime dropdown trigger must be present");
  await act(async () => {
    fireEvent.pointerDown(
      runtimeTrigger,
      new dom.window.MouseEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    fireEvent.click(runtimeTrigger);
  });
  const claudeItem = [
    ...dom.window.document.querySelectorAll('[role="menuitemradio"]'),
  ].find((node) => node.textContent?.trim() === "claude");
  assert.ok(claudeItem, "claude runtime option must appear in the dropdown");
  await act(async () => {
    fireEvent.click(claudeItem);
  });

  // The picker must disappear once runtimeTouched is set — its config surface
  // is only valid for the running session, not the prospective runtime.
  assert.equal(
    dom.window.document.getElementById("edit-agent-effort"),
    null,
    "effort picker must be hidden after a runtime switch — its options are unknown for the prospective runtime",
  );

  // Save — the runtime changed, so effortTouched must have been cleared.
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 10));
  });

  assert.equal(
    effortCalls().length,
    0,
    "no effortLevel must appear in update_managed_agent after a runtime switch — the config surface vocab belongs to the running session, not the prospective one",
  );
});

// ── Dismissal paths are blocked while isSaving (Escape, close-X, checkboxes) ────

// Sets up a deferred locked update (with effort in the payload) and clicks Save,
// then suspends with the update still pending. Returns the resolver so the caller
// can let the dialog settle. Pass onOpenChange to observe whether the dialog closed.
// With P3, effort is embedded in the locked update — there is no separate
// effort-setter phase. The "setter is in flight" window IS the update window.
async function startSaveWithDeferredUpdate(onOpenChange = () => {}) {
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });
  await act(async () => {
    renderDialog(onOpenChange);
  });
  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    // Let React process the isSaving=true transition before checks.
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
  return { resolveUpdate };
}

test("Escape dismissal is blocked while the locked update with effort is in flight", async () => {
  let openChange;
  const { resolveUpdate } = await startSaveWithDeferredUpdate((next) => {
    openChange = next;
  });

  // Locked update is still pending — simulate Escape on the document.
  await act(async () => {
    fireEvent.keyDown(dom.window.document, {
      key: "Escape",
      code: "Escape",
      bubbles: true,
    });
  });

  assert.equal(
    openChange,
    undefined,
    "Escape must be suppressed while a Save is in flight — the in-flight update must complete before the dialog may close",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
  assert.equal(
    openChange,
    false,
    "dialog closes after the locked update resolves",
  );
});

test("close-X dismissal is blocked while the locked update with effort is in flight", async () => {
  let openChange;
  const { resolveUpdate } = await startSaveWithDeferredUpdate((next) => {
    openChange = next;
  });

  // Locked update still pending — click the dialog's close-X button.
  const closeX = screen.getByRole("button", { name: /^close$/i });
  assert.ok(closeX, "close-X button must be present in the rendered dialog");
  await act(async () => {
    fireEvent.click(closeX);
  });

  assert.equal(
    openChange,
    undefined,
    "close-X must be suppressed while a Save is in flight — the in-flight update must complete before the dialog may close",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
  assert.equal(
    openChange,
    false,
    "dialog closes after the locked update resolves",
  );
});

test("auto-restart and inherit checkboxes are disabled while the locked update with effort is in flight", async () => {
  const { resolveUpdate } = installEffortIpc({ deferUpdate: true });
  await act(async () => {
    renderDialog(() => {});
  });

  // Expand Advanced to reveal the checkboxes before Save.
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /Advanced/ }));
  });

  await selectEffort("High");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await new Promise((resolve) => setTimeout(resolve, 5));
  });

  // Locked update is still pending. Both checkboxes must be disabled so a
  // post-snapshot state mutation cannot silently be discarded on close.
  const autoRestart = dom.window.document.getElementById(
    "edit-agent-auto-restart",
  );
  const inheritHarness = dom.window.document.getElementById(
    "edit-agent-inherit-harness",
  );
  assert.ok(autoRestart, "auto-restart checkbox must be present in Advanced");
  assert.ok(
    autoRestart.disabled,
    "auto-restart checkbox must be disabled while a Save is in flight — a post-snapshot toggle would be silently discarded on close",
  );
  assert.ok(
    inheritHarness,
    "inherit-harness checkbox must be present in Advanced",
  );
  assert.ok(
    inheritHarness.disabled,
    "inherit-harness checkbox must be disabled while a Save is in flight",
  );

  await act(async () => {
    resolveUpdate();
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
});
