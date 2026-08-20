/**
 * Mounted consumer regressions for the SetupStep forced-probe readiness gate.
 *
 * P1: isChecking = isFetching (not isLoading) ensures the Next button stays
 * disabled while the forced probe is in flight or has rejected, even when
 * cached data exists. With the old isLoading mapping, isLoading is false when
 * data is present, so the button was incorrectly enabled.
 *
 * Mutation proof: revert only the SetupStep.tsx hunk and both tests go RED
 * (button enabled in states it must block).
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

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("SetupStep Next button readiness gate — P1 regression (mounted consumer)", () => {
  it("onboarding-setup-next is disabled while forced probe is pending over cached data", async () => {
    const queryClient = makeQueryClient();
    // Pre-seed cache with a ready runtime. getReadyOnboardingRuntimes
    // will return it, so readyRuntimeIds.length > 0 — proving the button
    // is blocked by isChecking, not by an empty ready set.
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    const pending = deferred();
    discoverHandler = (args) =>
      args?.force === true ? pending.promise : Promise.resolve([]);

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(SetupStep, {
              actions: ACTIONS,
              direction: "forward",
              onReadyRuntimeIdsChange: NOOP,
            }),
          ),
        ),
      );
    });
    // Let the mount-time forceRefresh dispatch (but not resolve).
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    const button = container.querySelector(
      '[data-testid="onboarding-setup-next"]',
    );
    assert.ok(button, "onboarding-setup-next button must be present");
    assert.ok(
      button.disabled,
      "Next button must be disabled while forced probe is in flight over cached data",
    );

    // Resolve the pending probe inside act so React Query drains its state
    // update before unmount — prevents "Promise resolution still pending"
    // from the dangling deferred.
    await act(async () => {
      pending.resolve([]);
      await new Promise((r) => setTimeout(r, 0));
    });
    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("onboarding-setup-next is disabled after forced probe rejects over cached data", async () => {
    const queryClient = makeQueryClient();
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("forced probe rejected"))
        : Promise.resolve([]);

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(SetupStep, {
              actions: ACTIONS,
              direction: "forward",
              onReadyRuntimeIdsChange: NOOP,
            }),
          ),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    const button = container.querySelector(
      '[data-testid="onboarding-setup-next"]',
    );
    assert.ok(button, "onboarding-setup-next button must be present");
    assert.ok(
      button.disabled,
      "Next button must be disabled after forced probe rejects, even with cached data",
    );

    const errorEl = container.querySelector(
      '[data-testid="onboarding-setup-error"]',
    );
    assert.ok(
      errorEl,
      "the forced rejection error must be rendered after the probe rejects",
    );
    assert.match(
      errorEl.textContent ?? "",
      /forced probe rejected/,
      "rendered error must surface the forced rejection message",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });
});
