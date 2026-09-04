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
  useAcpRuntimesQueryForced,
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
  ({ useAcpRuntimesQueryForced, acpRuntimesQueryKey } = await import(
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

/** Raw snake_case backend entry as `discoverAcpRuntimes` receives before
 * `fromRawAcpRuntimeCatalogEntry`. Use for values returned from discoverHandler
 * (the IPC boundary), vs. `catalogEntry` for values seeded directly into cache. */
function rawEntry(id) {
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

/**
 * Mirrors the real surface: HarnessesSettingsPanel owns the single
 * force-on-mount (`useAcpRuntimesQueryForced()` with `forceOnMount` defaulted
 * to true) and renders the always-mounted dialog, which consumes shared state
 * with `forceOnMount: false`. Tests render this so the dialog reflects a real
 * owner-driven probe rather than firing its own.
 */
function HarnessSurface({ open }) {
  useAcpRuntimesQueryForced();
  return React.createElement(HarnessCatalogDialog, {
    open,
    onOpenChange: () => {},
  });
}

function renderSurface() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  return { container, root };
}

/**
 * Drive the search input the way a user would: set its value and dispatch a
 * React-observed input event, so `filterCatalogEntries` runs against a live
 * query. A query that matches no cached entry filters the list to zero.
 */
async function typeSearch(query) {
  const input = document.body.querySelector(
    '[data-testid="harness-catalog-search"]',
  );
  assert.ok(input, "search input must be present");
  const setter = Object.getOwnPropertyDescriptor(
    dom.window.HTMLInputElement.prototype,
    "value",
  ).set;
  await act(async () => {
    setter.call(input, query);
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
  });
}

function surfaceTree(open) {
  return React.createElement(
    QueryClientProvider,
    { client: queryClient },
    React.createElement(
      ThemeProvider,
      null,
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(HarnessSurface, { open }),
      ),
    ),
  );
}

let queryClient;

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("HarnessCatalogDialog forced-probe rendering — P2 regression (mounted consumer)", () => {
  it("harness-catalog-load-error is rendered on cold forced probe rejection", async () => {
    queryClient = makeQueryClient();
    // No cache seeded — isColdError = isError && data === undefined.
    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("cold load failure"))
        : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
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
    queryClient = makeQueryClient();
    // Seed cache with a visible runtime so filtered.length > 0 and the
    // isRefreshing branch renders (it is inside the non-empty entries block).
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    const pending = deferred();
    discoverHandler = (args) =>
      args?.force === true ? pending.promise : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    // Allow the owner's mount-time forceRefresh to dispatch (but not resolve).
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

  it("harness-catalog-refresh-error is rendered when a forced refresh rejects over cached entries", async () => {
    queryClient = makeQueryClient();
    // Cached entries present, so a rejected forced refresh is a WARM error
    // (isError && data !== undefined): stale entries must stay listed and the
    // failure must be visible inside the dialog.
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("warm refresh failure"))
        : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    const refreshErrorEl = document.body.querySelector(
      '[data-testid="harness-catalog-refresh-error"]',
    );
    assert.ok(
      refreshErrorEl,
      "harness-catalog-refresh-error must be rendered when a forced refresh rejects over cached entries",
    );
    // Cached entries remain listed alongside the failure indication.
    const cachedRow = document.body.querySelector(
      '[data-testid="harness-catalog-list-item-codex"]',
    );
    assert.ok(
      cachedRow,
      "cached entries must stay listed when a warm refresh fails",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("reopening the dialog after the owner's probe settles triggers zero additional forced probes", async () => {
    queryClient = makeQueryClient();
    let forcedProbeCount = 0;
    discoverHandler = (args) => {
      if (args?.force === true) forcedProbeCount += 1;
      // Probe COUNT is the assertion here, not rendered entries. Return an
      // empty catalog so no detail pane renders (handler results flow through
      // fromRawAcpRuntimeCatalogEntry, which expects the raw backend shape).
      return Promise.resolve([]);
    };

    const { container, root } = renderSurface();

    // Mount the surface (owner fires its single force-on-mount) and let it
    // settle.
    await act(async () => {
      root.render(surfaceTree(false));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    assert.equal(
      forcedProbeCount,
      1,
      "owner's force-on-mount must run exactly one forced probe",
    );

    // Open, close, reopen the dialog. With forceOnMount:false the dialog must
    // not fire its own probe on any of these transitions.
    for (const open of [true, false, true]) {
      await act(async () => {
        root.render(surfaceTree(open));
      });
      await act(async () => {
        await new Promise((r) => setTimeout(r, 20));
      });
    }
    assert.equal(
      forcedProbeCount,
      1,
      "opening/closing/reopening the dialog must trigger zero additional forced probes",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("harness-catalog-refresh-error is rendered when a forced refresh rejects over a cached EMPTY catalog", async () => {
    queryClient = makeQueryClient();
    // Cached successful empty catalog (data !== undefined but zero entries), so
    // filtered.length === 0. A rejected forced refresh is still a WARM error;
    // the status row must render independently of whether the filter has rows,
    // not be swallowed by the "No runtimes match" branch.
    queryClient.setQueryData(acpRuntimesQueryKey, []);

    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("warm empty refresh failure"))
        : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.ok(
      document.body.querySelector(
        '[data-testid="harness-catalog-refresh-error"]',
      ),
      "warm-error status must render over a cached empty catalog, not be hidden by 'No runtimes match'",
    );

    const emptyCatalogText = document.body.querySelector(
      '[data-testid="harness-catalog-list"]',
    )?.textContent;
    assert.ok(
      emptyCatalogText?.includes("No runtimes found."),
      "an empty catalog with no active search must show the truthful empty-catalog copy",
    );
    assert.ok(
      !emptyCatalogText?.includes("No runtimes match."),
      "the search-specific copy must not appear when no search is active",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("harness-catalog-refresh-error stays visible when an active search filters cached rows to zero", async () => {
    queryClient = makeQueryClient();
    // A cached row exists, but a search query matches nothing, so
    // filtered.length === 0 while data !== undefined. The warm-error status
    // must still show; only the entry list is filtered away.
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("warm filtered refresh failure"))
        : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    await typeSearch("zzz-no-such-runtime");

    assert.ok(
      document.body.querySelector(
        '[data-testid="harness-catalog-refresh-error"]',
      ),
      "warm-error status must stay visible while a search filters every cached row away",
    );
    assert.ok(
      document.body
        .querySelector('[data-testid="harness-catalog-list"]')
        ?.textContent?.includes("No runtimes match"),
      "the filtered-empty message and the status row coexist",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("harness-catalog-refreshing stays visible when an active search filters cached rows to zero", async () => {
    queryClient = makeQueryClient();
    queryClient.setQueryData(acpRuntimesQueryKey, [
      catalogEntry("codex", "logged_in"),
    ]);

    const pending = deferred();
    discoverHandler = (args) =>
      args?.force === true ? pending.promise : Promise.resolve([]);

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    await typeSearch("zzz-no-such-runtime");

    assert.ok(
      document.body.querySelector('[data-testid="harness-catalog-refreshing"]'),
      "refreshing spinner must stay visible while a search filters every cached row away",
    );

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

  it("cold-error Retry issues exactly one forced probe and renders entries on success", async () => {
    queryClient = makeQueryClient();
    // No cache seeded → cold error on the first forced probe. The owner's
    // mount-time probe rejects; the dialog's cold-error Retry must run a fresh
    // forced probe (not rely on close/reopen, which fires nothing) and, on
    // success, render the catalog.
    let forcedProbeCount = 0;
    discoverHandler = (args) => {
      if (args?.force !== true) return Promise.resolve([]);
      forcedProbeCount += 1;
      return forcedProbeCount === 1
        ? Promise.reject(new Error("cold load failure"))
        : Promise.resolve([rawEntry("codex")]);
    };

    const { container, root } = renderSurface();

    await act(async () => {
      root.render(surfaceTree(true));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    const retry = document.body.querySelector(
      '[data-testid="harness-catalog-load-retry"]',
    );
    assert.ok(retry, "cold-error state must render a Retry control");
    assert.equal(
      forcedProbeCount,
      1,
      "only the owner's mount probe has run before Retry",
    );

    await act(async () => {
      retry.dispatchEvent(
        new dom.window.MouseEvent("click", { bubbles: true }),
      );
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.equal(
      forcedProbeCount,
      2,
      "Retry must issue exactly one additional forced probe",
    );
    assert.ok(
      document.body.querySelector(
        '[data-testid="harness-catalog-list-item-codex"]',
      ),
      "entries must render after the Retry probe succeeds",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });
});
