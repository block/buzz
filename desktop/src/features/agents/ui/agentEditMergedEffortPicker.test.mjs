/**
 * EffortPickerField mount test for the merged edit dialog.
 *
 * #4557 mounted EffortPickerField into the (now-deleted) AgentInstanceEditDialog
 * beside the Model block, gated on a local backend AND a discovered effort
 * configId from the agent config surface. This PR folds the instance edit
 * surface into AgentEditMergedDialog; this test proves the ported field still
 * mounts under the same gating (id `edit-agent-effort`) and stays hidden when
 * the config surface has no discovered effort configId. It is mutation-
 * sensitive: remove the mount from the instance section and both the
 * visible-case assertion fails.
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

const GOOSE_RUNTIME = {
  id: "goose",
  label: "Goose",
  avatarUrl: "",
  availability: "not_installed",
  command: "goose-cmd",
  binaryPath: "goose-cmd",
  defaultArgs: [],
  mcpCommand: null,
  modelEnvVar: null,
  providerEnvVar: null,
  thinkingEnvVar: null,
  maxTokensEnvVar: null,
  contextLimitEnvVar: null,
  maxRoundsEnvVar: null,
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  requiresExternalCli: false,
  underlyingCliPath: null,
  nodeRequired: false,
  authStatus: { status: "not_applicable" },
  loginHint: null,
  source: "builtin",
  maxParallelism: 4,
};

function INSTANCE(overrides = {}) {
  return {
    pubkey: "pk-effort",
    name: "Solo",
    personaId: null,
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
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

function surface(overrides = {}) {
  return {
    runtimeId: "goose",
    runtimeLabel: "Goose",
    isPreSpawn: false,
    normalized: { thinkingEffort: null },
    advanced: [],
    extensions: [],
    sources: {},
    ...overrides,
  };
}

async function makeSeededClient(configSurface) {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], [GOOSE_RUNTIME]);
  qc.setQueryData(["personas"], []);
  qc.setQueryData(["managed-agents"], [INSTANCE()]);
  qc.setQueryData(["teams"], []);
  qc.setQueryData(["baked-build-env-keys"], []);
  qc.setQueryData(["agent-access-owner-only"], false);
  qc.setQueryData(["globalAgentConfig"], {
    env_vars: {},
    provider: null,
    model: null,
    preferred_runtime: null,
  });
  qc.setQueryData(["runtime-file-config", "goose"], {
    provider: null,
    model: null,
    satisfiedEnvKeys: [],
  });
  qc.setQueryData(["agent-config-surface", "pk-effort"], configSurface);
  liveClients.push(qc);
  return qc;
}

function installInvoke() {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => {
      if (cmd === "list_personas") return [];
      if (cmd === "list_managed_agents") return [INSTANCE()];
      if (cmd === "get_runtime_file_config") {
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      }
      if (cmd === "get_agent_config_surface") return surface();
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
}

async function renderMerged(qc) {
  const { createElement, act } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  const { AgentEditMergedDialog } = await import("./AgentEditMergedDialog.tsx");

  let view;
  await act(async () => {
    view = render(
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(
          ThemeProvider,
          { defaultTheme: "buzz" },
          createElement(AgentEditMergedDialog, {
            open: true,
            onOpenChange: () => {},
            ctx: { kind: "instance-only", instance: INSTANCE() },
          }),
        ),
      ),
    );
  });
  const present = !!dom.window.document.getElementById("edit-agent-effort");
  await act(async () => {
    view.unmount();
  });
  return present;
}

test("effort picker mounts in the merged dialog when the surface discovers an effort configId", async () => {
  const qc = await makeSeededClient(
    surface({
      effortConfigId: "thought_level",
      effortOptions: [
        { value: "low", displayName: "Low" },
        { value: "high", displayName: "High" },
      ],
    }),
  );
  installInvoke();
  const present = await renderMerged(qc);
  assert.equal(
    present,
    true,
    "the ported EffortPickerField must mount (id edit-agent-effort) for a local instance with a discovered effort configId",
  );
});

test("effort picker stays hidden when the surface advertises no effort configId", async () => {
  const qc = await makeSeededClient(surface());
  installInvoke();
  const present = await renderMerged(qc);
  assert.equal(
    present,
    false,
    "without a discovered effort configId the picker must not render",
  );
});
