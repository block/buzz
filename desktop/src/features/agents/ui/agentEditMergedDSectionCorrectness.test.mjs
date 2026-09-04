/**
 * D-section correctness regressions (Thufir pass-2 findings F1–F3).
 *
 * The parity/write-seam tests cover the four RESTORED controls. These three
 * lock the three behavioral gaps Thufir's control-by-control trace exposed —
 * each mounts the REAL dialog and asserts the observable a broken fix would
 * change:
 *
 *   F1 — linked D provider/model machinery follows the DEFINITION runtime, not
 *        the instance harness pin. Observable: the discover_agent_models IPC
 *        agentCommand. A definition on buzz-agent linked to a goose-pinned
 *        instance must discover models against buzz-agent.
 *   F2 — a cleared D tuning value falls back to the global/build default, never
 *        to the definition's own just-deleted value. Observable: the numeric
 *        input's "Inherit (…)" placeholder after a clear.
 *   F3 — a missing required D credential gates Save and marks collapsed
 *        Advanced. Observables: the Save button disabled state and the
 *        "Required" badge.
 *
 * Deleting any one of the three fixes turns its test red (verified by mutation
 * in the fix-round report).
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Records every Tauri command the dialog invokes. */
let invokeCalls = [];
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
  invokeCalls = [];
});

after(() => {
  globalThis.setTimeout = nativeSetTimeout;
  globalThis.setInterval = nativeSetInterval;
  globalThis.clearTimeout = nativeClearTimeout;
  globalThis.clearInterval = nativeClearInterval;
  dom.window.close();
});

/**
 * Two available runtimes with DISTINCT discovery commands so the F1 observable
 * (which command discovery runs against) unambiguously identifies the layer.
 */
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

const GOOSE_RUNTIME = {
  ...BUZZ_AGENT_RUNTIME,
  id: "goose",
  label: "Goose",
  command: "goose-cmd",
  binaryPath: "goose-cmd",
  modelEnvVar: null,
  providerEnvVar: null,
  thinkingEnvVar: null,
  maxTokensEnvVar: null,
  contextLimitEnvVar: null,
  maxRoundsEnvVar: null,
  maxParallelism: 4,
};

function DEFINITION(overrides = {}) {
  return {
    id: "def-correctness",
    displayName: "Brain",
    avatarUrl: null,
    systemPrompt: "You are helpful.",
    runtime: "buzz-agent",
    model: "claude-sonnet-4-5",
    provider: "anthropic",
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    sourceTeam: null,
    catalogSource: null,
    envVars: { ANTHROPIC_API_KEY: "sk-seed" },
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

/**
 * A ManagedAgent whose harness is pinned to goose (agentCommandOverride set,
 * so inheritHarness is false and prospectiveRuntimeId resolves to goose). Its
 * personaId links it to the definition, making the context linked.
 */
function INSTANCE(overrides = {}) {
  return {
    pubkey: "pk-correctness",
    name: "Brain",
    personaId: "def-correctness",
    runtime: "goose",
    teamId: null,
    relayUrl: "wss://relay.test",
    acpCommand: "",
    agentCommand: "goose-cmd",
    agentCommandOverride: "goose-cmd",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 0,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "You are helpful.",
    avatarUrl: null,
    model: null,
    modelSource: "definition",
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

/**
 * Seed a QueryClient with every catalog the dialog reads on open. `globalEnv`
 * seeds the global agent config env (used by F2 to prove the D inherited layer
 * is the global default, not the definition's own env).
 */
async function makeSeededClient({ runtimes, definition, globalEnv = {} } = {}) {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], runtimes ?? [BUZZ_AGENT_RUNTIME]);
  qc.setQueryData(["personas"], definition ? [definition] : []);
  qc.setQueryData(["managed-agents"], []);
  qc.setQueryData(["teams"], []);
  qc.setQueryData(["baked-build-env-keys"], []);
  qc.setQueryData(["agent-access-owner-only"], false);
  qc.setQueryData(["globalAgentConfig"], {
    env_vars: globalEnv,
    provider: null,
    model: null,
    preferred_runtime: null,
  });
  qc.setQueryData(["runtime-file-config", "buzz-agent"], {
    provider: null,
    model: null,
    satisfiedEnvKeys: [],
  });
  qc.setQueryData(["runtime-file-config", "goose"], {
    provider: null,
    model: null,
    satisfiedEnvKeys: [],
  });
  liveClients.push(qc);
  return qc;
}

/** Records every discover_agent_models agentCommand the dialog requests. */
function installInvoke({ definitionRaw, onDiscover } = {}) {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      invokeCalls.push({ cmd, args });
      if (cmd === "list_personas") return definitionRaw ? [definitionRaw] : [];
      if (cmd === "list_managed_agents") return [];
      if (cmd === "get_runtime_file_config") {
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      }
      if (cmd === "discover_agent_models") {
        onDiscover?.(args?.input);
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

async function renderDialog(qc, ctx) {
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
          ctx,
        }),
      ),
    ),
  );
}

async function settle(ms = 60) {
  const { act } = await import("react");
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

async function openAdvanced() {
  const { fireEvent, screen } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  const advanced = screen.getByTestId("persona-advanced-toggle");
  await reactAct(async () => {
    fireEvent.click(advanced);
    await new Promise((r) => setTimeout(r, 30));
  });
}

async function fireChange(el, value) {
  const { fireEvent } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  await reactAct(async () => {
    fireEvent.change(el, { target: { value } });
    await new Promise((r) => setTimeout(r, 10));
  });
}

// ── F1: linked D provider/model machinery follows the definition runtime ─────

test("test_linked_dsection_model_discovery_follows_definition_runtime_not_instance_pin", async () => {
  // Definition runs buzz-agent; the instance is harness-pinned to goose. The
  // D-section provider/model picker is the definition's, so model discovery
  // must run against the DEFINITION runtime command (buzz-agent-cmd), never the
  // instance pin (goose-cmd). Before F1, the discovery selectedRuntime switched
  // on `showInst` and would have discovered against goose.
  const definition = DEFINITION();
  const discoverCommands = [];
  const qc = await makeSeededClient({
    runtimes: [BUZZ_AGENT_RUNTIME, GOOSE_RUNTIME],
    definition,
  });
  installInvoke({
    onDiscover: (input) => discoverCommands.push(input?.agentCommand),
  });
  await renderDialog(qc, {
    kind: "instance-with-definition",
    definition,
    instance: INSTANCE(),
  });
  // anthropic requires an explicit model → discovery is debounced 250ms.
  await settle(400);

  assert.ok(
    discoverCommands.length > 0,
    "model discovery must run for the buzz-agent definition runtime",
  );
  assert.ok(
    discoverCommands.every((c) => c === "buzz-agent-cmd"),
    `discovery must target the definition runtime command; saw ${JSON.stringify(discoverCommands)}`,
  );
  assert.equal(
    discoverCommands.includes("goose-cmd"),
    false,
    "discovery must never run against the instance harness pin (goose)",
  );
});

// ── F2: a cleared D tuning value inherits the global default, not its own ────

test("test_linked_dsection_cleared_tuning_inherits_global_default_not_definition_value", async () => {
  // The definition sets BUZZ_AGENT_MAX_OUTPUT_TOKENS=4096; the global default is
  // 2048. When the user clears the definition's value, the numeric input's
  // inherited placeholder must show the GLOBAL fallback (2048) — not the
  // definition's just-deleted 4096. Before F2, the D inherited layer reused the
  // persona-inclusive overlay and echoed the definition's own value back.
  const definition = DEFINITION({
    envVars: {
      ANTHROPIC_API_KEY: "sk-seed",
      BUZZ_AGENT_MAX_OUTPUT_TOKENS: "4096",
    },
  });
  const qc = await makeSeededClient({
    runtimes: [BUZZ_AGENT_RUNTIME, GOOSE_RUNTIME],
    definition,
    globalEnv: { BUZZ_AGENT_MAX_OUTPUT_TOKENS: "2048" },
  });
  installInvoke();
  await renderDialog(qc, {
    kind: "instance-with-definition",
    definition,
    instance: INSTANCE(),
  });
  await settle();
  await openAdvanced();

  const { screen } = await import("@testing-library/react");
  const maxTokens = screen.getByTestId("numeric-max-output-tokens-input");
  assert.equal(
    maxTokens.value,
    "4096",
    "the definition's stored value shows in the input before clearing",
  );

  // Clear the value — onTuningEnvVarChange deletes the key, reverting to inherit.
  await fireChange(maxTokens, "");

  assert.equal(
    maxTokens.value,
    "",
    "the input is empty after the clear (value reverts to inherited)",
  );
  assert.equal(
    maxTokens.getAttribute("placeholder"),
    "Inherit (2048)",
    "the inherited placeholder must be the GLOBAL default, not the definition's just-deleted 4096",
  );
});

// ── F3: a missing required D credential gates Save + marks Advanced ──────────

test("test_definition_only_missing_required_secret_disables_save", async () => {
  // A buzz-agent/anthropic definition with no ANTHROPIC_API_KEY has an unmet
  // required credential. Save must be disabled until it is satisfied — main's
  // localModeSatisfied gate. Before F3, requiredEnvKeyMissing was dropped from
  // the D bundle and Save stayed enabled in definition-only context.
  const definition = DEFINITION({ envVars: {} });
  const qc = await makeSeededClient({ definition });
  installInvoke();
  await renderDialog(qc, { kind: "definition-only", definition });
  await settle();

  const { screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");
  assert.equal(
    save.disabled,
    true,
    "Save must be blocked while the required ANTHROPIC_API_KEY is unset",
  );

  // Typing the key satisfies the gate and re-enables Save.
  const apiKey = screen.getByTestId("persona-provider-api-key");
  await fireChange(apiKey, "sk-live-xyz");
  assert.equal(
    save.disabled,
    false,
    "Save re-enables once the required credential is provided",
  );
});

test("test_definition_only_missing_nonsecret_required_key_marks_collapsed_advanced", async () => {
  // Databricks requires DATABRICKS_HOST (a non-secret required key rendered as a
  // generic Advanced row, not an API-key field). When it is missing, the
  // collapsed Advanced disclosure must carry the "Required" badge so the user
  // sees configuration is incomplete without expanding. Before F3 the badge was
  // absent from the merged D-section.
  const definition = DEFINITION({
    provider: "databricks",
    model: null,
    envVars: {},
  });
  const qc = await makeSeededClient({ definition });
  installInvoke();
  await renderDialog(qc, { kind: "definition-only", definition });
  await settle();

  const { screen } = await import("@testing-library/react");
  // Advanced is collapsed on open; the badge must be visible without expanding.
  assert.ok(
    screen.getByTestId("persona-advanced-required-badge"),
    "collapsed Advanced must show the Required badge for a missing non-secret required key",
  );
});

test("test_team_managed_definition_does_not_block_save_on_missing_credential", async () => {
  // A team-managed (read-only) definition's credentials are not the user's to
  // change, so a missing required key must NOT block Save — F3's defReadOnly
  // carve-out. Mounted definition-only with a sourceTeam and no key.
  const definition = DEFINITION({ sourceTeam: "team-acme", envVars: {} });
  const qc = await makeSeededClient({ definition });
  installInvoke();
  await renderDialog(qc, { kind: "definition-only", definition });
  await settle();

  const { screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");
  assert.equal(
    save.disabled,
    false,
    "a team-managed definition must not block Save on a credential the user cannot edit",
  );
});
