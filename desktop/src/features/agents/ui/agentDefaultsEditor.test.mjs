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
 *   - Operate the effort control: click the real Popover trigger, select "off"
 *     from the option list. Assert zero writes after selection.
 *   - Exactly one `set_global_agent_config` write fires on "Save defaults" click.
 *   - The save stub captures the submitted payload; asserts raw
 *     GOOSE_THINKING_EFFORT: "off" is present.
 *   - After save the stub stores its canonical response (from the actual
 *     submitted payload). A fresh mount hydrated from that stored response
 *     shows data-value="off" and text "Off".
 *
 * DefaultConfigStep (onboarding surface):
 *   - Same contract through the onboarding parent tree and the "Next" button.
 *   - Draft starts with isDirty=false. The form is dirtied by operating the
 *     real effort control (click trigger → select "off"), which calls
 *     onConfigChange → updateDraft → sets isDirtyRef=true.
 *   - Zero writes on mount and after effort selection.
 *   - Exactly one write fires on "Next" click (commit() is a no-op when
 *     !isDirty, so real-control dirtying is load-bearing here).
 *   - Same payload capture + stored canonical + fresh remount contract.
 *
 * Mutation proofs:
 *   - Removing isHarnessNativeEffort branch in AgentConfigFields → effort
 *     custom trigger shows inherit placeholder instead of "Off" on mount and
 *     after remount → mount and remount assertions RED.
 *   - Removing the Save-gate (firing set_global_agent_config outside of a
 *     Save/Next click) → write-count-before-save assertion fails → RED.
 *   - Dropping GOOSE_THINKING_EFFORT from the submitted payload → payload
 *     assertion fails → RED.
 *   - In the onboarding test: removing the effort-select dirtying steps (so
 *     isDirty stays false) → commit() is a no-op → write-count assertion after
 *     Next fails (0 instead of 1) → RED.
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
// saveCallCount: total set_global_agent_config calls.
// capturedSavePayload: exact config submitted in the most recent Save/Next.
// storedCanonicalResponse: canonical response the stub computed from the Save
//   payload; returned by get_global_agent_config on the fresh remount.
let saveCallCount = 0;
let capturedSavePayload = null;
let storedCanonicalResponse = null;

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
      // Capture the submitted config and compute the canonical response by
      // echoing the payload (the server's canonical form is what was saved).
      capturedSavePayload = payload?.config ?? null;
      storedCanonicalResponse = capturedSavePayload ?? DEFAULT_CONFIG;
      return Promise.resolve({
        config: storedCanonicalResponse,
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
  capturedSavePayload = null;
  storedCanonicalResponse = null;
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

/**
 * Select an effort value through the real Popover-based custom select.
 * Clicks the trigger button to open the popover, then clicks the option button.
 * The AgentDropdownSelect is a controlled Popover + listbox, not a native
 * <select>; options are rendered as buttons with data-testid=
 * `${testId}-option-${value}`.
 */
async function selectEffortOption(testId, value) {
  const trigger = screen.getByTestId(testId);
  await act(async () => {
    fireEvent.click(trigger);
  });
  await settle();
  const option = screen.getByTestId(`${testId}-option-${value}`);
  await act(async () => {
    fireEvent.click(option);
  });
  await settle();
}

// ── Tests ──────────────────────────────────────────────────────────────────────

test("AgentDefaultsEditor: effort write→save→reread contract through the real Settings parent", async () => {
  // Production Settings journey through the real AgentDefaultsEditor parent:
  //   1. Mount with Goose effort "low" and a valid credential in env_vars.
  //   2. Assert zero IPC writes and trigger shows "Low" (Save-gated contract).
  //   3. Select "off" via the real Popover trigger + option click. Assert still
  //      zero IPC writes (effort selection is Save-gated, not direct-write).
  //   4. Dirty the form via the Advanced env-vars editor (regular HTML input,
  //      not Radix): open Advanced, click Add, type a key name.
  //   5. Click the real "Save defaults" button — fires exactly one IPC write.
  //   6. Assert the submitted payload contains raw GOOSE_THINKING_EFFORT: "off".
  //   7. Unmount and remount a FRESH AgentDefaultsEditor whose
  //      get_global_agent_config stub returns the stored canonical response
  //      (the actual server response from step 5, not a hand-written fixture).
  //      Assert zero writes on second mount and trigger shows "Off".
  //
  // Mutation proofs:
  //   - Remove the isHarnessNativeEffort branch in AgentConfigFields → the
  //     trigger shows "Select" instead of "Low"/"Off" at steps 2 and 7 → RED.
  //   - Fire set_global_agent_config outside a Save click → write-count-before-
  //     save assertion fails → RED.
  //   - Drop GOOSE_THINKING_EFFORT from the submitted payload → step 6
  //     payload assertion fails → RED.

  // ANTHROPIC_API_KEY satisfies credentialsValid so configIsValid=true and
  // the Save button is enabled once the form is dirtied.
  const initialConfig = {
    env_vars: {
      GOOSE_THINKING_EFFORT: "low",
      ANTHROPIC_API_KEY: "sk-test",
    },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(initialConfig),
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

  // Step 2: zero writes on mount, trigger shows "Low".
  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire on mount (effort is Save-gated)",
  );
  const triggerMount = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerMount,
    "effort trigger must be present after AgentDefaultsEditor loads",
  );
  assert.equal(
    triggerMount.getAttribute("data-value"),
    "low",
    'trigger data-value must be "low" on mount',
  );
  assert.ok(
    triggerMount.textContent?.includes("Low"),
    `trigger must show "Low" on mount; got: "${triggerMount.textContent}"`,
  );

  // Step 3: select "off" via the real effort Popover control.
  // AgentDropdownSelect renders options as <button data-testid="${testId}-option-${value}">.
  await selectEffortOption("global-agent-thinking-effort-select", "off");

  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire after effort selection (Save-gated, not direct-write)",
  );
  const triggerAfterEffort = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.equal(
    triggerAfterEffort?.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after effort selection',
  );
  assert.ok(
    triggerAfterEffort?.textContent?.includes("Off"),
    `trigger must show "Off" after effort selection; got: "${triggerAfterEffort?.textContent}"`,
  );

  // Step 4: open the Advanced section and add a new env-var row to ensure
  // the form is dirty for Save (credentials are present so configIsValid=true).
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

  // Step 6: assert the submitted payload contains raw GOOSE_THINKING_EFFORT: "off".
  assert.ok(
    capturedSavePayload !== null,
    "set_global_agent_config must have captured a payload",
  );
  assert.equal(
    capturedSavePayload?.env_vars?.GOOSE_THINKING_EFFORT,
    "off",
    'submitted payload must contain raw GOOSE_THINKING_EFFORT: "off"',
  );

  // Step 7: unmount and remount a FRESH parent hydrated from the stored
  // canonical response — the actual server response from step 5, not a
  // hand-written fixture. Zero additional writes; trigger shows "Off".
  unmount();
  cleanup();

  assert.ok(
    storedCanonicalResponse !== null,
    "stub must have stored a canonical response from the Save payload",
  );
  const canonicalForRemount = storedCanonicalResponse;
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(canonicalForRemount),
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

  const triggerRemount = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerRemount,
    "effort trigger must be present after fresh remount",
  );
  assert.equal(
    triggerRemount.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after fresh remount from stored canonical response',
  );
  assert.ok(
    triggerRemount.textContent?.includes("Off"),
    `trigger must show "Off" after fresh remount; got: "${triggerRemount.textContent}"`,
  );
});

test("DefaultConfigStep: effort write→save→reread contract through the real onboarding parent", async () => {
  // Production onboarding journey through the real DefaultConfigStep parent:
  //   1. Mount with Goose effort "low", valid credentials (ANTHROPIC_API_KEY),
  //      and isDirty=false in the draft. The credential makes configIsValid=true.
  //   2. Assert zero IPC writes and trigger shows "Low".
  //   3. Select "off" via the real effort Popover control. This calls
  //      onConfigChange → updateDraft → sets isDirtyRef=true. Assert still zero
  //      writes (effort selection is Save-gated).
  //   4. Click the real "Next" button — fires exactly one IPC write via
  //      persistenceState.commit() (which is a no-op when !isDirty, so the
  //      real-control dirtying in step 3 is load-bearing here).
  //   5. Assert the submitted payload contains raw GOOSE_THINKING_EFFORT: "off".
  //   6. Unmount and remount a FRESH DefaultConfigStep with draft=null, whose
  //      get_global_agent_config stub returns the stored canonical response.
  //      Assert zero writes on second mount and trigger shows "Off".
  //
  // Mutation proofs:
  //   - Remove the isHarnessNativeEffort branch in AgentConfigFields → trigger
  //     shows "Select" instead of "Low"/"Off" → mount and remount assertions RED.
  //   - Remove the effort-select steps (so isDirty stays false) → commit() is a
  //     no-op → write-count assertion after Next fails (0 instead of 1) → RED.
  //   - Drop GOOSE_THINKING_EFFORT from the submitted payload → step 5
  //     payload assertion fails → RED.

  const initialConfig = {
    env_vars: {
      GOOSE_THINKING_EFFORT: "low",
      ANTHROPIC_API_KEY: "sk-test",
    },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(initialConfig),
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

  // isDirty=false: commit() would be a no-op without real-control dirtying.
  // The effort selection in step 3 sets isDirtyRef=true via onConfigChange →
  // updateDraft, making the write load-bearing.
  const initialDraft = {
    config: initialConfig,
    isCustomModelEditing: false,
    isCustomProvider: false,
    isDirty: false,
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

  // Step 2: zero writes on mount, trigger shows "Low".
  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire on DefaultConfigStep mount",
  );
  const triggerMount = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerMount,
    "effort trigger must be present in DefaultConfigStep after Goose loads",
  );
  assert.equal(
    triggerMount.getAttribute("data-value"),
    "low",
    'DefaultConfigStep effort trigger data-value must be "low" on mount',
  );
  assert.ok(
    triggerMount.textContent?.includes("Low"),
    `DefaultConfigStep trigger must show "Low" on mount; got: "${triggerMount.textContent}"`,
  );

  // Step 3: select "off" via the real effort Popover control.
  // This calls onConfigChange → updateDraft → isDirtyRef=true, making the
  // subsequent Next click a real write rather than a no-op.
  await selectEffortOption("global-agent-thinking-effort-select", "off");

  assert.equal(
    saveCallCount,
    0,
    "zero IPC writes must fire after effort selection (Save-gated, not direct-write)",
  );
  const triggerAfterEffort = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.equal(
    triggerAfterEffort?.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after effort selection',
  );
  assert.ok(
    triggerAfterEffort?.textContent?.includes("Off"),
    `trigger must show "Off" after effort selection; got: "${triggerAfterEffort?.textContent}"`,
  );

  // Step 4: click the real "Next" button.
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
    "exactly one set_global_agent_config write must fire on Next (isDirty=true after effort selection)",
  );
  assert.ok(
    completeCalled.value,
    "actions.complete() must have been called after Next",
  );

  // Step 5: assert the submitted payload contains raw GOOSE_THINKING_EFFORT: "off".
  assert.ok(
    capturedSavePayload !== null,
    "set_global_agent_config must have captured a payload",
  );
  assert.equal(
    capturedSavePayload?.env_vars?.GOOSE_THINKING_EFFORT,
    "off",
    'submitted payload must contain raw GOOSE_THINKING_EFFORT: "off"',
  );

  // Step 6: unmount and remount a FRESH DefaultConfigStep with draft=null,
  // hydrated from the stored canonical response. Zero additional writes; "Off".
  unmount();
  cleanup();

  assert.ok(
    storedCanonicalResponse !== null,
    "stub must have stored a canonical response from the Save payload",
  );
  const canonicalForRemount = storedCanonicalResponse;
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(canonicalForRemount),
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

  const triggerRemount = screen.queryByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.ok(
    triggerRemount,
    "effort trigger must be present after fresh DefaultConfigStep remount",
  );
  assert.equal(
    triggerRemount.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" after fresh remount from stored canonical response',
  );
  assert.ok(
    triggerRemount.textContent?.includes("Off"),
    `DefaultConfigStep trigger must show "Off" after fresh remount; got: "${triggerRemount.textContent}"`,
  );
});
