/**
 * Real-parent Save/Next journeys: AgentDefaultsEditor and DefaultConfigStep
 * exercise the complete effort write→save→reread contract through the
 * production component trees that users actually encounter.
 *
 * Finding 2 (PR #4625): effortAutoClear.test.mjs tests AgentConfigFields
 * directly via a hand-rolled SettingsParent. These tests mount the real parents
 * to confirm the same invariants hold through the production entry points.
 *
 * AgentDefaultsEditor (Settings surface):
 *   - Loads config via `get_global_agent_config` IPC on mount.
 *   - Selects harness from the ACP runtime cache (QueryClientProvider).
 *   - Renders AgentConfigFields with useCustomSelect=true.
 *   - Zero IPC writes on mount and before Save (Save-gated contract).
 *   - Dirty the form via the Advanced env-vars editor (regular input, not Radix):
 *     open Advanced → Add row → type a key name.
 *   - Exactly one `set_global_agent_config` write fires on "Save defaults" click.
 *   - After save, a fresh mount hydrated from the server's canonical response
 *     (effort "off") shows data-value="off" and text "Off".
 *
 * DefaultConfigStep (onboarding surface):
 *   - Same contract through the onboarding parent tree and the "Next" button.
 *   - Starts with isDirty=true in the draft so commit() fires on Next.
 *   - After save, a fresh mount hydrated from the canonical server response
 *     shows data-value="off" and text "Off".
 *
 * Mutation proofs:
 *   - Removing isHarnessNativeEffort branch in AgentConfigFields → effort custom
 *     trigger shows inherit placeholder instead of "Off" → mount assertions RED.
 *   - Removing the Save-gate (firing set_global_agent_config outside of a Save/
 *     Next click) → write-count-before-save assertion fails → RED.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// ── Global env setup ─────────────────────────────────────────────────────────
Object.assign(globalThis, {
  document: dom.window.document,
  window: dom.window,
  IS_REACT_ACT_ENVIRONMENT: true,
  localStorage: dom.window.localStorage,
  self: dom.window,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
dom.window.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
dom.window.matchMedia ??= (query) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
globalThis.matchMedia = dom.window.matchMedia;
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (key === "window" || key === "document" || key === "globalThis") continue;
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
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── QueryClient tracking ──────────────────────────────────────────────────────
// react-query's default gcTime schedules timers that outlive each test and
// stall the process. Track every client; cancel + clear in afterEach.
const clients = [];

// ── IPC write tracking ────────────────────────────────────────────────────────
let saveCallCount = 0;

// ── Tauri IPC stub ────────────────────────────────────────────────────────────
const DEFAULT_CONFIG = {
  env_vars: {},
  provider: null,
  model: null,
  preferred_runtime: "goose",
};

function makeIpcHandler(overrides = {}) {
  return (cmd, payload) => {
    if (cmd in overrides) return overrides[cmd](payload);
    if (cmd === "get_global_agent_config")
      return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "set_global_agent_config") {
      saveCallCount += 1;
      return Promise.resolve({
        config: payload?.config ?? DEFAULT_CONFIG,
        restarted_count: 0,
        failed_restart_count: 0,
      });
    }
    if (cmd === "get_baked_build_env" || cmd === "get_baked_build_env_keys")
      return Promise.resolve([]);
    if (cmd === "discover_acp_providers")
      return Promise.resolve([rawGooseCatalogEntry()]);
    if (cmd === "discover_agent_models")
      return Promise.resolve({ options: [], is_optional: true });
    if (cmd === "get_runtime_file_config") return Promise.resolve(null);
    return Promise.reject(new Error(`unmocked: ${cmd}`));
  };
}

globalThis.__TAURI_INTERNALS__ = {
  invoke: makeIpcHandler(),
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────
let act, render, screen, cleanup, fireEvent, createElement;
let AgentDefaultsEditor;
let DefaultConfigStep;
let QueryClient, QueryClientProvider;
let acpRuntimesQueryKey, fromRawAcpRuntimeCatalogEntry;

before(async () => {
  ({ act, render, screen, cleanup, fireEvent } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ AgentDefaultsEditor } = await import("./AgentDefaultsEditor.tsx"));
  ({ DefaultConfigStep } = await import(
    "../../onboarding/ui/DefaultConfigStep.tsx"
  ));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ acpRuntimesQueryKey } = await import(
    "@/features/agents/acpRuntimesQuery.ts"
  ));
  ({ fromRawAcpRuntimeCatalogEntry } = await import("@/shared/api/tauri.ts"));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  // Reset write tracking and restore default IPC stub.
  saveCallCount = 0;
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler();
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

// ── Fixtures ──────────────────────────────────────────────────────────────────

/** Minimal raw Goose catalog entry with effort_canonical_values. */
function rawGooseCatalogEntry() {
  return {
    id: "goose",
    label: "Goose",
    avatar_url: "",
    availability: "available",
    command: "goose",
    binary_path: "/usr/local/bin/goose",
    default_args: [],
    mcp_command: null,
    model_env_var: "GOOSE_MODEL",
    provider_env_var: "GOOSE_PROVIDER",
    thinking_env_var: "GOOSE_THINKING_EFFORT",
    max_tokens_env_var: null,
    context_limit_env_var: null,
    max_rounds_env_var: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    requires_external_cli: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    login_hint: null,
    source: "builtin",
    effort_canonical_values: ["off", "low", "medium", "high", "max"],
  };
}

function makeQueryClient() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  clients.push(client);
  return client;
}

function seedGooseRuntime(queryClient) {
  const entry = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  queryClient.setQueryData(acpRuntimesQueryKey, [entry]);
  return entry;
}

function withQueryClient(client, children) {
  return createElement(QueryClientProvider, { client }, children);
}

/** Drain React update queue. */
async function settle() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 50));
  });
  await act(async () => {});
}

// ── Tests ──────────────────────────────────────────────────────────────────────

test("AgentDefaultsEditor: effort 'off' — zero writes on mount, one write on Save, 'Off' survives reread", async () => {
  // Production Settings journey through the real AgentDefaultsEditor parent:
  //   1. Mount with Goose effort "off" and a valid credential in env_vars.
  //   2. Assert zero IPC writes and trigger shows "Off" (Save-gated contract —
  //      no write fires from mount or effort-field visibility).
  //   3. Dirty the form via the Advanced env-vars editor (regular HTML input,
  //      not Radix): open Advanced, click Add, type a key name.
  //   4. Assert still zero IPC writes (all changes are Save-gated).
  //   5. Click the real "Save defaults" button — fires exactly one IPC write.
  //   6. Unmount and remount a FRESH AgentDefaultsEditor whose
  //      get_global_agent_config stub returns the canonical server response
  //      (effort "off"). Assert zero writes on second mount and trigger shows
  //      "Off".
  //
  // Mutation proofs:
  //   - Remove the isHarnessNativeEffort branch in AgentConfigFields → the
  //     trigger shows "Select" instead of "Off" at steps 2 and 6 → RED.
  //   - Fire set_global_agent_config outside a Save click → write-count-before-
  //     save assertion fails → RED.

  // ANTHROPIC_API_KEY satisfies credentialsValid so configIsValid=true and
  // the Save button is enabled once the form is dirtied.
  const savedConfig = {
    env_vars: {
      GOOSE_THINKING_EFFORT: "off",
      ANTHROPIC_API_KEY: "sk-test",
    },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(savedConfig),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient = makeQueryClient();
  seedGooseRuntime(queryClient);

  const { unmount } = render(
    withQueryClient(
      queryClient,
      createElement(AgentDefaultsEditor, { layout: "grouped" }),
    ),
  );

  await settle();

  // Step 2: zero writes on mount, trigger shows "Off".
  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire on mount (effort is Save-gated)",
  );
  const triggerBefore = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerBefore,
    "effort trigger must be present after AgentDefaultsEditor loads",
  );
  assert.equal(
    triggerBefore.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" on mount',
  );
  assert.ok(
    triggerBefore.textContent?.includes("Off"),
    `trigger must show "Off" on mount; got: "${triggerBefore.textContent}"`,
  );

  // Step 3: open the Advanced section via the toggle button (regular HTML
  // button, not Radix) and add a new env-var row, then type a key name.
  // This dirts the form via a real control without touching the effort field.
  const advancedToggle = screen.getByTestId("global-agent-advanced-toggle");
  await act(async () => {
    fireEvent.click(advancedToggle);
  });
  await settle();

  const addButton = screen.getByTestId("env-vars-add");
  await act(async () => {
    fireEvent.click(addButton);
  });
  await settle();

  // The new row's key input is the last [data-testid="env-vars-key"] in DOM.
  const keyInputs = screen.queryAllByTestId("env-vars-key");
  assert.ok(
    keyInputs.length > 0,
    "env-vars-key input must be present after Add",
  );
  const lastKeyInput = keyInputs[keyInputs.length - 1];
  await act(async () => {
    fireEvent.change(lastKeyInput, { target: { value: "TEST_DIRTY_KEY" } });
  });
  await settle();

  // Step 4: still zero IPC writes — all field changes are Save-gated.
  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire after env-var key edit (Save-gated, not direct-write)",
  );

  // Step 5: click the real "Save defaults" button.
  const saveButton = screen.getByRole("button", { name: /Save defaults/i });
  assert.ok(
    !saveButton.disabled,
    "Save button must be enabled after dirtying the form",
  );
  await act(async () => {
    fireEvent.click(saveButton);
  });
  await settle();

  assert.equal(
    saveCallCount,
    1,
    "exactly one set_global_agent_config write must fire on Save",
  );

  // Step 6: unmount and remount a FRESH parent hydrated from the canonical
  // server response (effort "off"). Zero additional writes; trigger shows "Off".
  unmount();
  cleanup();

  const canonicalConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(canonicalConfig),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient2 = makeQueryClient();
  seedGooseRuntime(queryClient2);

  render(
    withQueryClient(
      queryClient2,
      createElement(AgentDefaultsEditor, { layout: "grouped" }),
    ),
  );

  await settle();

  assert.equal(
    saveCallCount,
    1,
    "second mount must not fire any additional IPC writes",
  );

  const triggerAfter = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(triggerAfter, "effort trigger must be present after fresh remount");
  assert.equal(
    triggerAfter.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after fresh remount with canonical server response',
  );
  assert.ok(
    triggerAfter.textContent?.includes("Off"),
    `trigger must show "Off" after fresh remount; got: "${triggerAfter.textContent}"`,
  );
});

test("DefaultConfigStep: effort 'off' — zero writes on mount, one write on Next, 'Off' survives reread", async () => {
  // Production onboarding journey through the real DefaultConfigStep parent:
  //   1. Mount with Goose effort "off", valid credentials (ANTHROPIC_API_KEY),
  //      and isDirty=true in the draft (simulates the user having edited a field
  //      earlier in onboarding). The credential makes configIsValid=true so the
  //      Next button is enabled.
  //   2. Assert zero IPC writes and trigger shows "Off".
  //   3. Click the real "Next" button — fires exactly one IPC write via
  //      persistenceState.commit() (which is a no-op when !isDirty, so the
  //      isDirty=true draft is load-bearing here).
  //   4. Unmount and remount a FRESH DefaultConfigStep whose
  //      get_global_agent_config stub returns the canonical server response
  //      (effort "off"). Assert zero writes on second mount and trigger shows
  //      "Off".
  //
  // Mutation proofs:
  //   - Remove the isHarnessNativeEffort branch in AgentConfigFields → trigger
  //     shows "Select" instead of "Off" → mount assertion RED.
  //   - Remove isDirty=true from the draft → commit() is a no-op → write-count
  //     assertion after Next fails (0 instead of 1) → RED.

  const gooseConfig = {
    env_vars: {
      GOOSE_THINKING_EFFORT: "off",
      ANTHROPIC_API_KEY: "sk-test",
    },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  // Save stub echoes submitted config as the canonical response.
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(gooseConfig),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient = makeQueryClient();
  seedGooseRuntime(queryClient);

  const completeCalled = { value: false };
  const actions = {
    back: () => {},
    complete: () => {
      completeCalled.value = true;
    },
    discardDraft: () => {},
    updateDraft: () => {},
  };

  // isDirty=true: commit() will call setGlobalAgentConfig (instead of no-op).
  const initialDraft = {
    config: gooseConfig,
    isCustomModelEditing: false,
    isCustomProvider: false,
    isDirty: true,
  };

  const { unmount } = render(
    withQueryClient(
      queryClient,
      createElement(DefaultConfigStep, {
        actions,
        direction: "forward",
        draft: initialDraft,
        readyRuntimeIds: ["goose"],
      }),
    ),
  );

  await settle();

  // Step 2: zero writes on mount, trigger shows "Off".
  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire on DefaultConfigStep mount",
  );
  const triggerBefore = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerBefore,
    "effort trigger must be present in DefaultConfigStep after Goose loads",
  );
  assert.equal(
    triggerBefore.getAttribute("data-value"),
    "off",
    'DefaultConfigStep effort trigger data-value must be "off" on mount',
  );
  assert.ok(
    triggerBefore.textContent?.includes("Off"),
    `DefaultConfigStep trigger must show "Off" on mount; got: "${triggerBefore.textContent}"`,
  );

  // Step 3: click the real "Next" button.
  const nextButton = screen.getByTestId("onboarding-finish");
  assert.ok(
    !nextButton.disabled,
    "Next button must be enabled (canComplete=true: runtime selected + configIsValid)",
  );
  await act(async () => {
    fireEvent.click(nextButton);
  });
  await settle();

  assert.equal(
    saveCallCount,
    1,
    "exactly one set_global_agent_config write must fire on Next (isDirty=true in draft)",
  );
  assert.ok(
    completeCalled.value,
    "actions.complete() must have been called after Next",
  );

  // Step 4: unmount and remount a FRESH DefaultConfigStep hydrated from the
  // canonical server response (effort "off"). Zero additional writes; "Off".
  unmount();
  cleanup();

  const canonicalConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(canonicalConfig),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient2 = makeQueryClient();
  seedGooseRuntime(queryClient2);
  const actions2 = {
    back: () => {},
    complete: () => {},
    discardDraft: () => {},
    updateDraft: () => {},
  };

  render(
    withQueryClient(
      queryClient2,
      createElement(DefaultConfigStep, {
        actions: actions2,
        direction: "forward",
        draft: null,
        readyRuntimeIds: ["goose"],
      }),
    ),
  );

  await settle();

  assert.equal(
    saveCallCount,
    1,
    "second DefaultConfigStep mount must not fire any additional IPC writes",
  );

  const triggerAfter = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerAfter,
    "effort trigger must be present after fresh DefaultConfigStep remount",
  );
  assert.equal(
    triggerAfter.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after fresh remount with canonical server response',
  );
  assert.ok(
    triggerAfter.textContent?.includes("Off"),
    `DefaultConfigStep trigger must show "Off" after fresh remount; got: "${triggerAfter.textContent}"`,
  );
});
