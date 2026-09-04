/**
 * Instance-name validity gate regressions (Carl round-5 P1-2, Thufir pass-3 IMPORTANT 3).
 *
 * Thufir pass-2 mutation-proved that the `instanceName.trim().length > 0` guard
 * in AgentEditMergedDialog.tsx has no coverage: removing it left the full
 * mounted/model/coordinator suite GREEN. These two tests mount the real dialog
 * with an instance context and verify that Save is disabled after clearing the
 * visible instance name — for both pooled and unpooled definitions.
 *
 * Acceptance mutation: replace `instanceName.trim().length > 0` with `true` in
 * AgentEditMergedDialog.tsx canSubmit. Each test must turn RED under that mutation.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** QueryClients created per test, cleared in afterEach so no gc timer lingers. */
const liveClients = [];
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

  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, _args) => {
      if (cmd === "list_personas") return [];
      if (cmd === "list_managed_agents") return [];
      if (cmd === "get_runtime_file_config")
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      if (cmd === "discover_agent_models")
        return {
          agentName: "buzz-agent",
          agentVersion: "1.0.0",
          models: [],
          agentDefaultModel: null,
          selectedModel: null,
          supportsSwitching: false,
        };
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
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

const BUZZ_AGENT_RUNTIME = {
  id: "buzz-agent",
  displayName: "Buzz Agent",
  commandTemplate: "buzz-agent-cmd",
  isBuiltIn: true,
  isAvailable: true,
  unavailableReason: null,
  supportsMultipleInstances: true,
};

function DEFINITION(overrides = {}) {
  return {
    id: "def-instance-gate",
    displayName: "Gatekeeper",
    avatarUrl: null,
    systemPrompt: "Keep the gate.",
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

function INSTANCE(overrides = {}) {
  return {
    pubkey: "pk-gate",
    name: "Gatekeeper",
    personaId: "def-instance-gate",
    runtime: "buzz-agent",
    teamId: null,
    relayUrl: "wss://relay.test",
    acpCommand: "",
    agentCommand: "buzz-agent-cmd",
    agentCommandOverride: null,
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 0,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: "Keep the gate.",
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

async function makeSeededClient({ definition } = {}) {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], [BUZZ_AGENT_RUNTIME]);
  qc.setQueryData(["personas"], definition ? [definition] : []);
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

async function fireChange(el, value) {
  const { fireEvent } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  await reactAct(async () => {
    fireEvent.change(el, { target: { value } });
    await new Promise((r) => setTimeout(r, 10));
  });
}

// ── Blank-name gate: unpooled instance ───────────────────────────────────────

test("test_blank_unpooled_instance_name_disables_save", async () => {
  // An unpooled instance-with-definition: clearing the visible instance name
  // must disable the Save button. This pins the `instanceName.trim().length > 0`
  // gate in canSubmit. If the gate is removed (Thufir's prescribed mutation),
  // Save remains enabled after the clear and this test turns RED.
  const definition = DEFINITION(); // no namePool
  const qc = await makeSeededClient({ definition });
  await renderDialog(qc, {
    kind: "instance-with-definition",
    definition,
    instance: INSTANCE(),
  });
  await settle();

  const { screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");

  // Initially enabled (name is seeded non-blank).
  assert.equal(
    save.disabled,
    false,
    "Save must be enabled when the instance name is non-blank at seed time",
  );

  // Clear the visible instance name.
  const nameInput = screen.getByLabelText(
    /agent name \(this deployed instance\)/i,
  );
  await fireChange(nameInput, "");

  assert.equal(
    save.disabled,
    true,
    "Save must be disabled when the instance name is cleared — Carl P1-2: blank name must not no-op",
  );
});

// ── Blank-name gate: pooled instance ─────────────────────────────────────────

test("test_blank_pooled_instance_name_disables_save", async () => {
  // A pooled instance-with-definition: the definition has a namePool but the
  // VISIBLE instance name field must still be gated by canSubmit when blank.
  // Before the pass-2 fix, the pool exemption allowed a blank pooled name to
  // slip through — this test pins both the pool-exemption removal and the gate
  // itself. If the gate is removed (Thufir's mutation), Save stays enabled and
  // this test turns RED.
  const definition = DEFINITION({ namePool: ["Alpha", "Beta", "Gamma"] });
  const qc = await makeSeededClient({ definition });
  await renderDialog(qc, {
    kind: "instance-with-definition",
    definition,
    instance: INSTANCE({ name: "Alpha" }),
  });
  await settle();

  const { screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");

  // Initially enabled (seeded name is "Alpha" — non-blank).
  assert.equal(
    save.disabled,
    false,
    "Save must be enabled when the pooled instance name is seeded non-blank",
  );

  // Clear the visible instance name.
  const nameInput = screen.getByLabelText(
    /agent name \(this deployed instance\)/i,
  );
  await fireChange(nameInput, "");

  assert.equal(
    save.disabled,
    true,
    "Save must be disabled when the pooled instance name is cleared — pool exemption must not bypass the gate",
  );
});
