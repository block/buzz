/**
 * Mounted deep-link focus ACCEPTANCE tests for the merged agent edit dialog.
 *
 * These sit at the boundary where scenario-8 actually failed: the whole
 * AgentEditMergedDialog mounted from a config-nudge deep link, driving the real
 * runtime-state / discovery / conditional-section pipeline — NOT an isolated
 * hook or seam. They differ from agentEditMergedDeepLinkFocus.test.mjs (which
 * neutralizes target focus to isolate Radix's autofocus decision) by leaving
 * target focus LIVE, so they assert the real `document.activeElement` end state
 * the deep link must reach.
 *
 * jsdom orders rAF after Radix's focus scope, so the dialog's own targeted
 * focus effect runs last and wins once its target is reachable — exactly the
 * behavior these tests observe. Each test's target only becomes reachable
 * through the fix under test: the env-key row requires the Advanced-expansion
 * effect to mount EnvVarsEditor; the model combobox requires the disabled-guard
 * to defer focus until discovery resolves and the control is focusable.
 *
 * Mutation evidence (verified in the round-5 report):
 *   • delete the env_key Advanced-expansion effect → env_key test RED;
 *   • delete the model disabled-guard on the normalized-field effect → model
 *     test RED (the one-shot latch fires against the disabled control and the
 *     no-op focus is never retried after discovery resolves).
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** QueryClients created per test, cleared in afterEach so no gc timer lingers. */
const liveClients = [];
/**
 * Radix's dismissable-layer / focus-scope schedule global timers that outlive
 * the render; jsdom's timers delegate to the global, so track every timer we
 * hand out and clear the survivors in teardown or the process hangs after pass.
 */
const pendingTimers = new Set();
const nativeSetTimeout = globalThis.setTimeout;
const nativeSetInterval = globalThis.setInterval;
const nativeClearTimeout = globalThis.clearTimeout;
const nativeClearInterval = globalThis.clearInterval;

before(() => {
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
  // Real rAF (macrotask-backed): the targeted-focus effects schedule the actual
  // focus() call in a rAF, so a no-op rAF would swallow it. Unlike the isolation
  // harness, focus() is NOT neutralized here — these tests observe real landing.
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
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const qc of liveClients.splice(0)) {
    qc.clear();
    qc.unmount();
  }
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

/** A buzz-agent runtime, available with a discovery command. */
const BUZZ_AGENT_RUNTIME = {
  id: "buzz-agent",
  label: "Buzz Agent",
  avatarUrl: "",
  availability: "available",
  command: "buzz-agent-cmd",
  binaryPath: "buzz-agent-cmd",
  defaultArgs: [],
  mcpCommand: null,
  modelEnvVar: "BUZZ_AGENT_MODEL",
  providerEnvVar: "BUZZ_AGENT_PROVIDER",
  thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
  maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
  contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
  maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  requiresExternalCli: false,
  underlyingCliPath: null,
  nodeRequired: false,
  authStatus: { status: "not_applicable" },
  loginHint: null,
  source: "builtin",
  maxParallelism: 2,
};

/**
 * An unlinked (instance-only) ManagedAgent pinned to the buzz-agent harness.
 * `provider` selects which credential/model machinery the dialog drives.
 */
function INSTANCE(overrides = {}) {
  return {
    pubkey: "pk-focus",
    name: "Solo",
    personaId: null,
    runtime: "buzz-agent",
    teamId: null,
    relayUrl: "wss://relay.test",
    acpCommand: "",
    agentCommand: "buzz-agent-cmd",
    agentCommandOverride: "buzz-agent-cmd",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 0,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "You are helpful.",
    avatarUrl: null,
    model: null,
    modelSource: "instance",
    provider: null,
    personaOutOfDate: false,
    personaOrphaned: false,
    needsRestart: false,
    restartDiff: [],
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    lastErrorCode: null,
    logPath: "",
    startOnAppLaunch: false,
    autoRestartOnConfigChange: false,
    backend: "local",
    backendAgentId: null,
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

async function makeSeededClient(instance) {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], [BUZZ_AGENT_RUNTIME]);
  qc.setQueryData(["personas"], []);
  qc.setQueryData(["managed-agents"], [instance]);
  qc.setQueryData(["teams"], []);
  qc.setQueryData(["baked-build-env-keys"], []);
  qc.setQueryData(["agent-access-owner-only"], false);
  qc.setQueryData(["globalAgentConfig"], {
    env_vars: {},
    provider: null,
    model: null,
    preferred_runtime: null,
  });
  qc.setQueryData(["runtime-file-config", "buzz-agent"], {
    provider: null,
    model: null,
    satisfiedEnvKeys: [],
  });
  liveClients.push(qc);
  return qc;
}

/**
 * Install the Tauri IPC seam. `onDiscover` optionally delays / customizes the
 * discover_agent_models response so a test can observe the loading window.
 */
function installInvoke(instance, { discoverDelayMs = 0 } = {}) {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => {
      if (cmd === "list_personas") return [];
      if (cmd === "list_managed_agents") return [instance];
      if (cmd === "get_runtime_file_config") {
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      }
      if (cmd === "discover_agent_models") {
        if (discoverDelayMs > 0) {
          await new Promise((r) => setTimeout(r, discoverDelayMs));
        }
        return {
          agentName: "buzz-agent",
          agentVersion: "1.0.0",
          models: [],
          agentDefaultModel: null,
          selectedModel: null,
          supportsSwitching: false,
        };
      }
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
}

async function renderDialog(qc, instance, initialFocus) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  const { AgentEditMergedDialog } = await import("./AgentEditMergedDialog.tsx");

  return render(
    createElement(
      QueryClientProvider,
      { client: qc },
      createElement(
        ThemeProvider,
        { defaultTheme: "buzz" },
        createElement(AgentEditMergedDialog, {
          open: true,
          onOpenChange: () => {},
          ctx: { kind: "instance-only", instance },
          initialFocus,
        }),
      ),
    ),
  );
}

async function settle(ms = 80) {
  const { act } = await import("react");
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

/**
 * Tear the dialog down inside act(). With a deep-link target supplied, Radix's
 * focus scope re-traps focus on every macrotask tick while the dialog stays
 * mounted, which spins the event loop and stalls teardown. Every test captures
 * the DOM facts it needs first, then unmounts here BEFORE asserting so the
 * process exits cleanly regardless of the assertion outcome.
 */
async function unmount(view) {
  const { act } = await import("react");
  await act(async () => {
    view.unmount();
  });
}

// ── env_key: deep link expands Advanced, mounts the row, and focuses it ──────

test("test_env_key_deep_link_expands_advanced_and_focuses_required_row", async () => {
  // A databricks_v2 instance requires the non-secret DATABRICKS_HOST key, which
  // renders as a generic required row inside the collapsed Advanced accordion.
  // Opening the dialog with an env_key deep link must (1) expand Advanced so
  // EnvVarsEditor mounts, (2) materialize the DATABRICKS_HOST required row, and
  // (3) land document.activeElement on that row's value input. Deleting the
  // hook's Advanced-expansion effect leaves Advanced collapsed, so the row
  // never mounts and focus never reaches it.
  const instance = INSTANCE({ provider: "databricks_v2" });
  const qc = await makeSeededClient(instance);
  installInvoke(instance);

  const view = await renderDialog(qc, instance, {
    type: "env_key",
    key: "DATABRICKS_HOST",
  });
  const { screen } = await import("@testing-library/react");

  // Let the Advanced-expansion effect run, EnvVarsEditor mount, the required
  // row materialize, and the row's focus rAF drain before capturing facts.
  await settle(120);
  await settle(20);

  const editor = screen.queryByTestId("env-vars-editor");
  const valueInput = screen.queryByLabelText("Value for DATABRICKS_HOST");
  const activeElement = dom.window.document.activeElement;
  await unmount(view);

  // (1) Advanced expanded: its editor mounted.
  assert.ok(editor, "Advanced must expand so EnvVarsEditor mounts");
  // (2) The DATABRICKS_HOST required row materialized.
  assert.ok(
    valueInput,
    "the required DATABRICKS_HOST row must materialize in the mounted editor",
  );
  // (3) Focus landed on that row's value input.
  assert.equal(
    activeElement,
    valueInput,
    "the env_key deep link must land focus on the DATABRICKS_HOST value input",
  );
});

// ── model: focus defers while disabled during discovery, lands after resolve ─

test("test_model_deep_link_defers_focus_until_discovery_resolves", async () => {
  // An anthropic instance triggers debounced model discovery on open. While
  // discovery is in flight the model combobox is `disabled`, and focusing a
  // disabled control is a silent no-op. The deep-link focus effect must NOT
  // latch against the disabled control: it must wait for modelDiscoveryLoading
  // to resolve, then focus the now-enabled combobox. Deleting the disabled
  // guard latches the one-shot on the disabled control (no-op focus) and never
  // retries after discovery resolves, so focus never reaches the model field.
  const instance = INSTANCE({ provider: "anthropic" });
  const qc = await makeSeededClient(instance);
  // Delay discovery past the 250ms credential debounce so the disabled window
  // is observable and the effect must survive it to focus post-resolution.
  installInvoke(instance, { discoverDelayMs: 60 });

  const view = await renderDialog(qc, instance, {
    type: "normalized_field",
    field: "model",
  });

  const modelControl = () =>
    dom.window.document.getElementById("edit-agent-model");

  // During discovery: the combobox is disabled and MUST NOT have been focused.
  await settle(40);
  const disabledDuringDiscovery = modelControl()?.hasAttribute("disabled");
  const activeDuringDiscovery = dom.window.document.activeElement;
  const disabledControl = modelControl();

  // Discovery resolves: the combobox becomes focusable and the deep-link
  // effect (which re-runs on modelDiscoveryLoading) claims it.
  await settle(400);
  await settle(20);
  const model = modelControl();
  const disabledAfterDiscovery = model?.hasAttribute("disabled");
  const activeAfterDiscovery = dom.window.document.activeElement;
  await unmount(view);

  assert.ok(
    disabledDuringDiscovery,
    "the model combobox is disabled while model discovery is in flight",
  );
  assert.notEqual(
    activeDuringDiscovery,
    disabledControl,
    "focus must not land on the model combobox while it is disabled",
  );
  assert.equal(
    disabledAfterDiscovery,
    false,
    "the model combobox is enabled once discovery resolves",
  );
  assert.equal(
    activeAfterDiscovery,
    model,
    "the model deep link must land focus on the combobox after discovery resolves",
  );
});
