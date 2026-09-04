/**
 * Deep-link focus coordination test (scenario-8 regression).
 *
 * When the merged edit dialog opens from a config-nudge deep link with an
 * `initialFocus` target, the requested field must own initial focus. The dialog
 * mounts Radix's `Dialog`, whose default open-autofocus lands on the first
 * focusable control (the avatar/name inputs). In the browser that autofocus
 * runs AFTER the dialog's own rAF-scheduled targeted focus and steals focus
 * back, so the deep link lands on the wrong field. The fix passes
 * `onOpenAutoFocus={e => e.preventDefault()}` to the dialog content whenever an
 * `initialFocus` is supplied, so Radix never moves focus and the targeted
 * effect owns it.
 *
 * jsdom orders rAF-vs-focus-scope the opposite way from the browser (the
 * component's rAF focus runs last and wins regardless of the fix), so an
 * end-state activeElement assertion cannot see the seam. This harness ISOLATES
 * Radix's default autofocus: it neutralizes `focus()` on the deep-link target
 * elements so the ONLY thing that can move focus off <body> is Radix's default
 * open-autofocus. With the fix, that autofocus is prevented and focus stays on
 * <body>; remove the `onOpenAutoFocus` prevention and Radix moves focus onto
 * the first control — turning the first test red. The second test is the
 * control: with no deep-link target the prevention is inert and Radix's default
 * autofocus must still fire, guarding against an unconditional suppression.
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
  // Real rAF (macrotask-backed): the instance runtime hook schedules work via
  // requestAnimationFrame during mount, so a no-op rAF crashes the render. A
  // setTimeout-backed rAF lets the mount complete AND lets the dialog's own
  // targeted-focus effect run — which is why we neutralize focus() on the
  // targets below rather than the rAF itself.
  dom.window.requestAnimationFrame = (cb) =>
    globalThis.setTimeout(() => cb(Date.now()), 0);
  dom.window.cancelAnimationFrame = (id) => globalThis.clearTimeout(id);
  globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
  globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;
  // Neutralize focus() on the deep-link targets so the dialog's own targeted
  // focus cannot move focus. The ONLY remaining focus mover is Radix's default
  // open-autofocus — exactly the behavior onOpenAutoFocus controls.
  const realFocus = dom.window.HTMLElement.prototype.focus;
  dom.window.HTMLElement.prototype.focus = function focus(...args) {
    if (
      this.id === "edit-agent-model" ||
      this.id === "edit-agent-llm-provider"
    ) {
      return;
    }
    return realFocus.apply(this, args);
  };
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

/** An unlinked (instance-only) ManagedAgent — Gurney's failing tuple. */
function INSTANCE(overrides = {}) {
  return {
    pubkey: "pk-deeplink",
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
    backend: "local",
    backendAgentId: null,
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

async function makeSeededClient() {
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
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
}

/**
 * Render the dialog and flush mount effects inside a single act() — this runs
 * Radix's open-autofocus (a mount effect) and reads activeElement immediately
 * after, BEFORE advancing the macrotask queue. Two reasons this must be
 * synchronous, not settle()d: (1) it captures Radix's autofocus decision in
 * isolation, before the (harness-neutralized) targeted-focus rAF; (2) when the
 * fix is removed, Radix's focus scope re-traps focus on every subsequent
 * macrotask tick, so a settle() spins forever — a synchronous read observes the
 * initial decision and lets teardown complete cleanly.
 */
async function renderAndReadActiveElement(qc, initialFocus) {
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
            initialFocus,
          }),
        ),
      ),
    );
  });
  const active = dom.window.document.activeElement;
  // Tear the dialog down before returning. When the fix is removed, Radix's
  // focus scope re-traps focus on every macrotask tick while the dialog is
  // mounted, which would spin the event loop and stall the process. Unmounting
  // here removes those listeners so the assertion below fails fast and cleanly.
  await act(async () => {
    view.unmount();
  });
  return active;
}

test("test_deep_link_suppresses_default_open_autofocus", async () => {
  const qc = await makeSeededClient();
  installInvoke();
  const active = await renderAndReadActiveElement(qc, {
    type: "normalized_field",
    field: "model",
  });

  // The dialog's own targeted focus is neutralized in this harness, so the only
  // thing that can move focus off <body> is Radix's default open-autofocus. The
  // fix prevents it when a deep-link target is requested, so focus stays on
  // <body>. Remove the onOpenAutoFocus prevention and Radix moves focus onto
  // the first focusable control, failing this assertion.
  assert.equal(
    active,
    dom.window.document.body,
    "with a deep-link target, default open-autofocus must be suppressed so it can't steal focus from the targeted field",
  );
});

test("test_default_open_autofocus_active_without_deep_link_target", async () => {
  // Control: with no initialFocus, the prevention is inert and Radix's default
  // open-autofocus must still fire — moving focus off <body> onto the first
  // control. Guards against an unconditional suppression that would regress the
  // ordinary open-and-type flow.
  const qc = await makeSeededClient();
  installInvoke();
  const active = await renderAndReadActiveElement(qc, undefined);

  assert.notEqual(
    active,
    dom.window.document.body,
    "without a deep-link target, default open-autofocus must move focus off <body>",
  );
});
