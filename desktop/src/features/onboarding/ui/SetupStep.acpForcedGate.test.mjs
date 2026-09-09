/**
 * Mounted consumer regressions for SetupStep cached-ready revalidation.
 * A warm forced probe still runs on entry, but cached readiness remains
 * visually stable unless that probe fails.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, describe, it } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
dom.window.ResizeObserver = globalThis.ResizeObserver;
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

// ── Tauri IPC stub ────────────────────────────────────────────────────────────

let discoverHandler = () => Promise.resolve([]);

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    if (command === "discover_acp_providers") return discoverHandler(args);
    // All other commands (e.g. plugin:event|listen from useInstallOutputLine)
    // reject; useInstallOutputLine catches gracefully ("event system unavailable").
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports (must run after globalThis is configured) ────────────────

let React,
  act,
  createRoot,
  QueryClient,
  QueryClientProvider,
  SetupStep,
  acpRuntimesQueryKey,
  TooltipProvider;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ SetupStep } = await import("./SetupStep.tsx"));
  ({ acpRuntimesQueryKey } = await import(
    "@/features/agents/acpRuntimesQuery.ts"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
});

afterEach(() => {
  discoverHandler = () => Promise.resolve([]);
});

after(() => dom.window.close());

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Camelcase AcpRuntimeCatalogEntry as stored in acpRuntimesQueryKey cache. */
function catalogEntry(id, authStatusValue) {
  return {
    id,
    label: id,
    avatarUrl: "",
    availability: "available",
    command: id,
    binaryPath: `/usr/bin/${id}`,
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
    authStatus: { status: authStatusValue },
    loginHint: null,
    source: "builtin",
    definitionEnv: {},
  };
}

/** Raw snake_case backend entry as `discoverAcpRuntimes` receives it before
 * `fromRawAcpRuntimeCatalogEntry`. Use for values a forced probe resolves at
 * the IPC boundary (vs. `catalogEntry` for values seeded directly into cache). */
function rawReadyEntry(id) {
  return {
    id,
    label: id,
    avatar_url: "",
    availability: "available",
    command: id,
    binary_path: `/usr/bin/${id}`,
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    requires_external_cli: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "logged_in" },
    source: "builtin",
  };
}

function makeQueryClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const NOOP = () => {};
const ACTIONS = { back: NOOP, next: NOOP, navigateToAgentSettings: NOOP };

/** Mount SetupStep under the query client + tooltip provider it requires. */
function renderSetupStep() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  return { container, root };
}

function setupStepTree(queryClient) {
  return React.createElement(
    QueryClientProvider,
    { client: queryClient },
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(SetupStep, {
        actions: ACTIONS,
        direction: "forward",
        initialMethod: "subscription",
        onReadyRuntimeIdsChange: NOOP,
      }),
    ),
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("SetupStep cached-ready revalidation", () => {
  it("keeps a cached ready harness available while a warm forced probe is pending", async () => {
    const queryClient = makeQueryClient();
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    const pending = deferred();
    discoverHandler = (args) =>
      args?.force === true ? pending.promise : Promise.resolve([]);

    const { container, root } = renderSetupStep();
    await act(async () => {
      root.render(setupStepTree(queryClient));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    const readyCard = container.querySelector(
      '[data-testid="onboarding-runtime-codex"]',
    );
    assert.ok(readyCard, "the cached harness remains visible during recheck");
    assert.equal(readyCard.getAttribute("data-ready"), "true");
    assert.equal(
      container.querySelector('[data-testid="onboarding-runtime-ready-codex"]'),
      null,
      "the installed section does not repeat readiness with a tag",
    );
    assert.equal(
      container.querySelector(
        '[data-testid="onboarding-runtime-rechecking-codex"]',
      ),
      null,
      "a warm recheck does not flash a redundant Checking state",
    );

    // Success preserves the stable ready card without adding a status tag.
    await act(async () => {
      pending.resolve([rawReadyEntry("codex")]);
      await new Promise((r) => setTimeout(r, 50));
    });
    assert.equal(
      container
        .querySelector('[data-testid="onboarding-runtime-codex"]')
        ?.getAttribute("data-ready"),
      "true",
      "the harness remains ready once the warm recheck succeeds",
    );
    assert.equal(
      container.querySelector(
        '[data-testid="onboarding-runtime-rechecking-codex"]',
      ),
      null,
      "no Checking indicator appears on success",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("replaces cached Ready with a recheck affordance after a warm forced probe rejects", async () => {
    const queryClient = makeQueryClient();
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("warm recheck failed"))
        : Promise.resolve([]);

    const { container, root } = renderSetupStep();
    await act(async () => {
      root.render(setupStepTree(queryClient));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.ok(
      container.querySelector(
        '[data-testid="onboarding-runtime-recheck-codex"]',
      ),
      "a warm rejection over a cached-ready runtime must offer a recheck, not claim READY",
    );
    assert.equal(
      container.querySelector('[data-testid="onboarding-runtime-ready-codex"]'),
      null,
      "cached READY must not be presented as current after the recheck rejects",
    );
    assert.ok(
      container.querySelector('[data-testid="onboarding-setup-error"]'),
      "the warm rejection error stays visible alongside the retained card",
    );
    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });
});
