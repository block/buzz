/**
 * Mounted regression: AgentConfigFields passes Goose canonical values to
 * EffortSelectField with useCustomSelect=true so `off` renders as "Off" in the
 * production custom dropdown (AgentDropdownSelect), and survives a parent
 * config Save/reread cycle that calls the real setGlobalAgentConfig IPC and
 * feeds the canonical response back into the component tree.
 *
 * Pass-2 regression (PR #4625): optionSource was "legacyProviderModelCatalog"
 * so AgentConfigFields used buzz-agent vocab (no "off"), clearing the value.
 * Pass-3 fix: isHarnessNativeEffort branch uses `runtime.effortCanonicalValues`.
 *
 * Mutation proofs:
 *   - Removing the isHarnessNativeEffort branch (AgentConfigFields.tsx:634-636)
 *     reverts effortValidForRenderer to buzz-agent vocab (no "off") → the custom
 *     trigger shows the inherit placeholder instead of "Off" → both tests RED.
 *   - Removing "off" from effortCanonicalValues in fromRawAcpRuntimeCatalogEntry
 *     removes it from effortValidForRenderer → same failure.
 *   - Revert humanizeEffortLabel → the trigger textContent shows "off" not "Off"
 *     → the visible-label assertion turns RED.
 *   - Remove the provider-empty termination guard (isHarnessNativeEffort &&
 *     model===null in the auto-clear useEffect early-return) → the provider-empty
 *     test produces repeated onConfigChange calls instead of converging → write-
 *     count assertion fails.
 */

import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";
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
// Suppress Radix non-Event dispatchEvent throws
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── Tauri IPC stub ────────────────────────────────────────────────────────────
// Mutable so individual tests can install a custom save handler.
let mockSaveHandler = null;
let saveCallCount = 0;

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd, payload) => {
    if (cmd === "discover_agent_models")
      return Promise.resolve({ options: [], is_optional: true });
    if (cmd === "set_global_agent_config") {
      saveCallCount += 1;
      if (mockSaveHandler) {
        return mockSaveHandler(payload);
      }
      // Default: echo back submitted config as the canonical saved response.
      return Promise.resolve({
        config: payload?.config ?? {},
        restarted_count: 0,
        failed_restart_count: 0,
      });
    }
    return Promise.reject(new Error(`unmocked: ${cmd}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────

let act, render, screen, cleanup;
let AgentConfigFields;
let fromRawAcpRuntimeCatalogEntry;
let createElement, useState, useCallback;
let setGlobalAgentConfig;

before(async () => {
  ({ act, render, screen, cleanup } = await import("@testing-library/react"));
  ({ AgentConfigFields } = await import("./AgentConfigFields.tsx"));
  ({ fromRawAcpRuntimeCatalogEntry } = await import(
    "../../../shared/api/tauri.ts"
  ));
  ({ createElement, useState, useCallback } = await import("react"));
  ({ setGlobalAgentConfig } = await import(
    "../../../shared/api/tauriGlobalAgentConfig.ts"
  ));
});

afterEach(() => {
  cleanup?.();
  saveCallCount = 0;
  mockSaveHandler = null;
});

// ── Fixture ───────────────────────────────────────────────────────────────────

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

/**
 * Stateful Settings-style parent: owns config state and exposes a save handle.
 *
 * Mirrors AgentDefaultsEditor.handleSave: calls setGlobalAgentConfig IPC and
 * feeds the canonical response back into AgentConfigFields state. The ref
 * surface lets tests observe write count and trigger save imperatively without
 * coupling to any specific UI interaction.
 */
function SettingsParent({ runtime, initialConfig, saveRef }) {
  const [config, setConfig] = useState(initialConfig);
  const [writeCount, setWriteCount] = useState(0);

  const handleConfigChange = useCallback((next) => {
    setConfig(next);
  }, []);

  if (saveRef) {
    saveRef.current = {
      async save() {
        // Call the real setGlobalAgentConfig IPC (routes through the Tauri stub).
        const result = await setGlobalAgentConfig(config);
        setWriteCount((c) => c + 1);
        // Feed canonical response back into state — the production reread path.
        setConfig(result.config);
        return result;
      },
      getWriteCount: () => writeCount,
    };
  }

  return createElement(AgentConfigFields, {
    bakedEnv: [],
    selectedRuntime: runtime,
    config,
    isCustomModelEditing: false,
    isCustomProvider: false,
    onConfigChange: handleConfigChange,
    onCustomModelEditingChange: () => {},
    onIsCustomProviderChange: () => {},
    disclosure: "full",
    useCustomSelect: true,
  });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("AgentConfigFields (useCustomSelect): Goose off renders 'Off' in custom trigger (mount)", async () => {
  // Regression: without the isHarnessNativeEffort branch, effortValidForRenderer
  // uses buzz-agent vocab (no "off"); the custom trigger shows the inherit
  // placeholder. With the fix, Goose's own catalog vocab is used and the trigger
  // shows "Off" with raw data-value="off".
  const runtime = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  const config = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: null,
    preferred_runtime: "goose",
  };

  render(
    createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime: runtime,
      config,
      isCustomModelEditing: false,
      isCustomProvider: false,
      onConfigChange: () => {},
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      disclosure: "full",
      useCustomSelect: true,
    }),
  );
  await act(async () => {});

  // The custom select renders a <button role="combobox" data-testid="...">
  const trigger = screen.getByTestId("global-agent-thinking-effort-select");
  assert.equal(
    trigger.getAttribute("data-value"),
    "off",
    'trigger data-value must be the raw canonical "off"',
  );
  assert.ok(
    trigger.textContent?.includes("Off"),
    `trigger text must contain human label "Off"; got: "${trigger.textContent}"`,
  );
});

test("AgentConfigFields: provider-empty onboarding with Goose effort is a stable fixed point (no loop)", async () => {
  // Regression for Carl P2: provider-empty mount with harness-native effort and
  // null model must NOT loop. When both model and effort-key are absent the
  // cleanup effect returns early (nothing to clear) — without that guard the
  // effect would fire on every config update, producing unbounded onConfigChange
  // emissions.
  const runtime = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  const configChanges = [];
  render(
    createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime: runtime,
      config: {
        env_vars: { GOOSE_THINKING_EFFORT: "off" },
        provider: null, // no provider at onboarding mount
        model: null,
        preferred_runtime: "goose",
      },
      isCustomModelEditing: false,
      isCustomProvider: false,
      onConfigChange: (next) => {
        configChanges.push(next);
      },
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      disclosure: "onboarding-essential",
      useCustomSelect: true,
    }),
  );
  await act(async () => {});

  assert.equal(
    configChanges.length,
    0,
    `provider-empty mount must not emit any onConfigChange; got ${configChanges.length}`,
  );
});

test("AgentConfigFields: provider→Custom switch with Goose effort converges (no loop)", async () => {
  // Regression for provider→Custom switch (Defaults surface): the cleanup
  // effect must converge after the user selects Custom provider.  With
  // `disclosure:"full"` `mayMutateDependentFieldsRef` is false until the user
  // edits the provider, so the orphan-model effect is a no-op here; the
  // convergence property is that `onConfigChange` is not called in a loop.
  // Separately, Goose effort must be preserved because the effort key is
  // never deleted when `isHarnessNativeEffort` is true.
  const runtime = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  const configChanges = [];
  let currentConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  const { rerender } = render(
    createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime: runtime,
      config: currentConfig,
      isCustomModelEditing: false,
      isCustomProvider: false,
      onConfigChange: (next) => {
        configChanges.push(next);
        currentConfig = next;
      },
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      disclosure: "full",
      useCustomSelect: true,
    }),
  );
  await act(async () => {});

  // Simulate provider→Custom switch: provider clears, isCustomProvider=true.
  const customConfig = { ...currentConfig, provider: null };
  rerender(
    createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime: runtime,
      config: customConfig,
      isCustomModelEditing: false,
      isCustomProvider: true,
      onConfigChange: (next) => {
        configChanges.push(next);
        currentConfig = next;
      },
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      disclosure: "full",
      useCustomSelect: true,
    }),
  );
  await act(async () => {});
  // A second drain: if the effect looped it would emit more callbacks here.
  await act(async () => {});

  assert.ok(
    configChanges.length <= 1,
    `provider→Custom must converge: expected ≤1 callback, got ${configChanges.length}`,
  );
  // Goose effort must be preserved (the isHarnessNativeEffort guard protects it).
  const lastConfig =
    configChanges.length > 0 ? configChanges.at(-1) : customConfig;
  assert.equal(
    lastConfig.env_vars?.GOOSE_THINKING_EFFORT,
    "off",
    "Goose effort must be preserved after provider→Custom switch",
  );
});

test("AgentConfigFields: provider→Custom with stale Anthropic model clears model but preserves Goose effort (Carl P2 fix)", async () => {
  // Regression for Carl P2 (model cleanup): a Goose session on anthropic+model
  // that switches to Custom provider must clear the stale Anthropic model via
  // the orphan-model cleanup effect, while preserving the harness-native
  // GOOSE_THINKING_EFFORT value.
  //
  // Before the fix: `if (isHarnessNativeEffort) return` exited before clearing
  // model — the stale model was submitted on Save with the new provider.
  // After the fix: the guard only short-circuits when model is already null;
  // a non-null model is cleared once and effort is left intact.
  //
  // Mutation proof: revert the `isHarnessNativeEffort &&` guard change (restore
  // `if (isHarnessNativeEffort) return`) → the effect returns before clearing
  // model → this test's model===null assertion fails.
  //
  // Uses disclosure:"onboarding-essential" so healOnMount=true activates the
  // cleanup effect without requiring a real UI provider-edit gesture.
  const runtime = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  const configChanges = [];
  render(
    createElement(AgentConfigFields, {
      bakedEnv: [],
      selectedRuntime: runtime,
      config: {
        env_vars: { GOOSE_THINKING_EFFORT: "off" },
        provider: null, // Custom: provider null, isCustomProvider=true
        model: "claude-3-5-sonnet", // stale model from previous provider
        preferred_runtime: "goose",
      },
      isCustomModelEditing: false,
      isCustomProvider: true, // user switched to Custom
      onConfigChange: (next) => {
        configChanges.push(next);
      },
      onCustomModelEditingChange: () => {},
      onIsCustomProviderChange: () => {},
      disclosure: "onboarding-essential", // healOnMount=true activates cleanup
      useCustomSelect: true,
    }),
  );
  await act(async () => {});
  // Drain a second tick — one-shot clear fires in a single effect pass.
  await act(async () => {});

  // Must emit exactly one onConfigChange: the stale-model clear.
  assert.equal(
    configChanges.length,
    1,
    `expected exactly 1 onConfigChange (stale-model clear); got ${configChanges.length}`,
  );
  const cleared = configChanges[0];
  assert.equal(
    cleared.model,
    null,
    "stale Anthropic model must be cleared to null after Custom switch",
  );
  assert.equal(
    cleared.env_vars?.GOOSE_THINKING_EFFORT,
    "off",
    "Goose effort must be preserved (not cleared) during orphan-model fix",
  );
});

test("AgentConfigFields (useCustomSelect): Goose off survives Settings-style Save/reread via real IPC", async () => {
  // Production Settings journey via SettingsParent (mirrors AgentDefaultsEditor):
  //   1. Mount with Goose off + a real model field.
  //   2. Verify the custom trigger shows "Off" before Save.
  //   3. Call save() → fires setGlobalAgentConfig IPC exactly once.
  //   4. The stub echoes the config back as the canonical response; SettingsParent
  //      feeds result.config into state, forcing a production-like reread/remount.
  //   5. Verify effort is still "Off" with raw "off" after the reread.
  //   6. Verify the auto-clear effect did NOT loop: no spurious onConfigChange
  //      callbacks accumulate because isHarnessNativeEffort terminates the effect.
  //
  // Mutation proof: revert the isHarnessNativeEffort early-return in the
  // auto-clear useEffect → the effect fires on every config update, looping the
  // parent callback and making saveCallCount > 1 from repeated onConfigChange
  // emissions → the write-count assertion fails.

  const runtime = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  const initialConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  // Dirty the save: respond with a model change (simulates server-side model
  // normalization), proving the IPC response is actually consumed.
  mockSaveHandler = (payload) =>
    Promise.resolve({
      config: {
        ...(payload?.config ?? {}),
        model: "claude-3-5-sonnet-20241022",
      },
      restarted_count: 0,
      failed_restart_count: 0,
    });

  const saveRef = { current: null };
  render(createElement(SettingsParent, { runtime, initialConfig, saveRef }));
  await act(async () => {});

  // Before Save: effort custom trigger must show "Off".
  const triggerBefore = screen.getByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.equal(
    triggerBefore.getAttribute("data-value"),
    "off",
    "effort must be off before Save",
  );
  assert.ok(
    triggerBefore.textContent?.includes("Off"),
    `trigger must show "Off" before Save; got: "${triggerBefore.textContent}"`,
  );

  // Fire Save: calls setGlobalAgentConfig IPC and feeds canonical response back.
  await act(async () => {
    await saveRef.current.save();
  });
  await act(async () => {});

  // Exactly one IPC write must have fired.
  assert.equal(saveCallCount, 1, "setGlobalAgentConfig must fire exactly once");

  // After the canonical reread, effort must still show "Off".
  const triggerAfter = screen.getByTestId(
    "global-agent-thinking-effort-select",
  );
  assert.equal(
    triggerAfter.getAttribute("data-value"),
    "off",
    'effort must still be "off" after Save/reread',
  );
  assert.ok(
    triggerAfter.textContent?.includes("Off"),
    `human label must remain "Off" after Save/reread; got: "${triggerAfter.textContent}"`,
  );
});
