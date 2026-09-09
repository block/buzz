/**
 * D-section write-seam tests.
 *
 * The parity mount tests (agentEditMergedDSectionParity.test.mjs) assert the
 * restored controls RENDER and read the definition layer; the pure-logic tests
 * (agentFormModel.test.mjs) assert a changed `envVars` object routes to
 * personaInput. Neither exercises the seam between the two: the restored
 * control's own `onChange` → the D-env write → the coordinator's update_persona
 * payload. A disconnected callback (onEnvVarChange / onValueChange → no-op)
 * passes every one of those tests while silently dropping the edit.
 *
 * These mount the REAL dialog in a definition-only context, interact with each
 * restored control (numeric tuning input, buzz-agent effort select, provider
 * API-key input), click Save, and assert the typed value reaches
 * update_persona.envVars — and that no instance write (update_managed_agent)
 * fires, since definition-only context has no instance overlay. Deleting any
 * one of the three production callbacks turns its test red.
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

const BUZZ_AGENT_RUNTIME = {
  id: "buzz-agent",
  label: "Buzz Agent",
  avatarUrl: "",
  availability: "not_installed",
  command: "buzz-agent",
  binaryPath: "buzz-agent",
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

const DEFINITION = {
  id: "def-seam",
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
  envVars: {},
  respondTo: null,
  respondToAllowlist: [],
  parallelism: null,
  createdAt: "2025-01-01T00:00:00Z",
  updatedAt: "2025-01-01T00:00:00Z",
};

function DEFINITION_RAW() {
  return {
    id: DEFINITION.id,
    display_name: DEFINITION.displayName,
    avatar_url: null,
    system_prompt: DEFINITION.systemPrompt,
    runtime: "buzz-agent",
    model: "claude-sonnet-4-5",
    provider: "anthropic",
    name_pool: [],
    is_builtin: false,
    is_active: true,
    shared: false,
    source_team: null,
    env_vars: {},
    respond_to: null,
    respond_to_allowlist: [],
    parallelism: null,
    created_at: DEFINITION.createdAt,
    updated_at: DEFINITION.updatedAt,
  };
}

/** Seed a QueryClient with every catalog the dialog reads on open. */
async function makeSeededClient() {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], [BUZZ_AGENT_RUNTIME]);
  qc.setQueryData(["personas"], [DEFINITION]);
  qc.setQueryData(["managed-agents"], []);
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
 * Install a Tauri invoke stub that records the update_persona payload. The
 * persisted persona becomes the settlement re-fetch result so the coordinator
 * observes the write and closes the dialog.
 */
function installInvoke(onUpdatePersona) {
  let currentRaw = DEFINITION_RAW();
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      invokeCalls.push({ cmd, args });
      if (cmd === "list_personas") return [currentRaw];
      if (cmd === "list_managed_agents") return [];
      if (cmd === "get_runtime_file_config") {
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      }
      if (cmd === "update_persona") {
        currentRaw = onUpdatePersona(args?.input) ?? currentRaw;
        return currentRaw;
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

  const closed = { value: false };
  const result = render(
    createElement(
      QueryClientProvider,
      { client: qc },
      createElement(
        ThemeProvider,
        { defaultTheme: "buzz" },
        createElement(AgentEditMergedDialog, {
          open: true,
          onOpenChange: (open) => {
            if (!open) closed.value = true;
          },
          ctx,
        }),
      ),
    ),
  );
  return { ...result, closed };
}

async function settle(ms = 40) {
  const { act } = await import("react");
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

async function openAdvanced() {
  const { fireEvent, screen } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  const advanced = screen.getByRole("button", { name: /Advanced/ });
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

async function clickSave() {
  const { fireEvent, screen } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  const save = screen.getByTestId("edit-agent-dialog-submit");
  await reactAct(async () => {
    fireEvent.click(save);
    await new Promise((r) => setTimeout(r, 60));
  });
}

const definitionCtx = { kind: "definition-only", definition: DEFINITION };

// ── Numeric tuning knob write reaches personaInput.envVars ───────────────────

test("test_numeric_tuning_edit_reaches_update_persona_env_and_not_instance", async () => {
  const qc = await makeSeededClient();
  let captured = null;
  installInvoke((input) => {
    captured = input;
    return { ...DEFINITION_RAW(), env_vars: input.envVars ?? {} };
  });
  // Seed the required provider secret so the D credential gate does not block
  // Save — the subject under test is the numeric-knob write, not readiness.
  await renderDialog(qc, {
    kind: "definition-only",
    definition: { ...DEFINITION, envVars: { ANTHROPIC_API_KEY: "sk-live" } },
  });
  await settle();
  await openAdvanced();

  const { screen } = await import("@testing-library/react");
  const maxTokens = screen.getByTestId("numeric-max-output-tokens-input");
  await fireChange(maxTokens, "8192");
  await clickSave();

  assert.ok(
    captured,
    "a numeric tuning edit must reach update_persona (personaInput emitted)",
  );
  assert.equal(
    captured.envVars?.BUZZ_AGENT_MAX_OUTPUT_TOKENS,
    "8192",
    "the numeric value must land in update_persona.envVars (definition layer)",
  );
  assert.equal(
    invokeCalls.some((c) => c.cmd === "update_managed_agent"),
    false,
    "definition-only context has no instance overlay — no update_managed_agent",
  );
});

// ── buzz-agent effort select write reaches personaInput.envVars ──────────────

test("test_effort_knob_edit_reaches_update_persona_env_and_not_instance", async () => {
  const qc = await makeSeededClient();
  let captured = null;
  installInvoke((input) => {
    captured = input;
    return { ...DEFINITION_RAW(), env_vars: input.envVars ?? {} };
  });
  await renderDialog(qc, {
    kind: "definition-only",
    definition: { ...DEFINITION, envVars: { ANTHROPIC_API_KEY: "sk-live" } },
  });
  await settle();
  await openAdvanced();

  const { screen } = await import("@testing-library/react");
  const effort = screen.getByTestId("ba-thinking-effort-select");
  // "high" is a valid effort for claude-sonnet-4-5 (anthropic adaptive table).
  await fireChange(effort, "high");
  await clickSave();

  assert.ok(
    captured,
    "an effort edit must reach update_persona (personaInput emitted)",
  );
  assert.equal(
    captured.envVars?.BUZZ_AGENT_THINKING_EFFORT,
    "high",
    "the effort value must land in update_persona.envVars (definition layer)",
  );
  assert.equal(
    invokeCalls.some((c) => c.cmd === "update_managed_agent"),
    false,
    "definition-only context has no instance overlay — no update_managed_agent",
  );
});

// ── provider API-key write reaches personaInput.envVars ──────────────────────

test("test_api_key_edit_reaches_update_persona_env_and_not_instance", async () => {
  const qc = await makeSeededClient();
  let captured = null;
  installInvoke((input) => {
    captured = input;
    return { ...DEFINITION_RAW(), env_vars: input.envVars ?? {} };
  });
  await renderDialog(qc, definitionCtx);
  await settle();

  // The API-key field is a top-level D control (not under Advanced).
  const { screen } = await import("@testing-library/react");
  const apiKey = screen.getByTestId("persona-provider-api-key");
  await fireChange(apiKey, "sk-live-xyz");
  await clickSave();

  assert.ok(
    captured,
    "an API-key edit must reach update_persona (personaInput emitted)",
  );
  assert.equal(
    captured.envVars?.ANTHROPIC_API_KEY,
    "sk-live-xyz",
    "the API key must land in update_persona.envVars (definition layer)",
  );
  assert.equal(
    invokeCalls.some((c) => c.cmd === "update_managed_agent"),
    false,
    "definition-only context has no instance overlay — no update_managed_agent",
  );
});
