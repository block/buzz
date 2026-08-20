/**
 * Mounted consumer regressions for HarnessCatalogDialog forced-probe states.
 *
 * P2: isColdError (isError && data===undefined) drives the error branch;
 * isRefreshing (isFetching && !isLoading) drives the refresh-indicator branch.
 * Both elements carry data-testid selectors that distinguish them from the
 * generic "No runtimes match" fallback.
 *
 * Mutation proof: revert only the HarnessCatalogDialog.tsx hunk and both
 * tests go RED — cold rejection shows "No runtimes match" instead of the
 * error element; cached + pending shows no refresh indicator.
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
// Copy all DOM-level globals from JSDOM that Radix Dialog's focus/dismiss
// machinery references without a window. prefix (HTMLInputElement, NodeFilter,
// getComputedStyle, etc.). Doing this in bulk avoids whack-a-mole per missing
// global as new Radix internals are encountered.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      key.startsWith("CSS") ||
      [
        "Node",
        "NodeFilter",
        "NodeList",
        "NamedNodeMap",
        "Event",
        "CustomEvent",
        "MouseEvent",
        "KeyboardEvent",
        "FocusEvent",
        "InputEvent",
        "PointerEvent",
        "TouchEvent",
        "WheelEvent",
        "EventTarget",
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
        "getComputedStyle",
        "IntersectionObserver",
        "ResizeObserver",
      ].includes(key))
  ) {
    const val = dom.window[key];
    if (val !== undefined) globalThis[key] = val;
  }
}
// getComputedStyle must be bound to dom.window or it throws "Illegal invocation".
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);

// Radix DismissableLayer and FocusScope dispatch plain objects via dispatchEvent
// for layer-coordination events. JSDOM's strict Event type validation throws on
// these; silently drop non-Event objects so the Dialog renders its content
// without throwing from effects. This does not affect real Event delivery.
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── Tauri IPC stub ────────────────────────────────────────────────────────────

let discoverHandler = () => Promise.resolve([]);

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    if (command === "discover_acp_providers") return discoverHandler(args);
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────

let React,
  act,
  createRoot,
  QueryClient,
  QueryClientProvider,
  HarnessCatalogDialog,
  acpRuntimesQueryKey,
  ThemeProvider,
  TooltipProvider;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ HarnessCatalogDialog } = await import("./HarnessCatalogDialog.tsx"));
  ({ acpRuntimesQueryKey } = await import(
    "@/features/agents/acpRuntimesQuery.ts"
  ));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider.tsx"));
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

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("HarnessCatalogDialog forced-probe rendering — P2 regression (mounted consumer)", () => {
  it("harness-catalog-load-error is rendered on cold forced probe rejection", async () => {
    const queryClient = makeQueryClient();
    // No cache seeded — isColdError = isError && data === undefined.
    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("cold load failure"))
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
            ThemeProvider,
            null,
            React.createElement(
              TooltipProvider,
              null,
              React.createElement(HarnessCatalogDialog, {
                open: true,
                onOpenChange: () => {},
              }),
            ),
          ),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    // Dialog renders via Radix portal into document.body.
    const errorEl = document.body.querySelector(
      '[data-testid="harness-catalog-load-error"]',
    );
    assert.ok(
      errorEl,
      "harness-catalog-load-error must be rendered on cold forced probe rejection",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("harness-catalog-refreshing is rendered while forced probe is pending over cached entries", async () => {
    const queryClient = makeQueryClient();
    // Seed cache with a visible runtime so filtered.length > 0 and the
    // isRefreshing branch renders (it is inside the non-empty entries block).
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
            ThemeProvider,
            null,
            React.createElement(
              TooltipProvider,
              null,
              React.createElement(HarnessCatalogDialog, {
                open: true,
                onOpenChange: () => {},
              }),
            ),
          ),
        ),
      );
    });
    // Allow mount-time forceRefresh to dispatch (but not resolve).
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    const refreshingEl = document.body.querySelector(
      '[data-testid="harness-catalog-refreshing"]',
    );
    assert.ok(
      refreshingEl,
      "harness-catalog-refreshing must be rendered while forced probe is pending over cached entries",
    );

    // Resolve inside act so React Query drains before unmount.
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
});
