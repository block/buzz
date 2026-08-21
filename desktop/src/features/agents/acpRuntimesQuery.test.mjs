/**
 * Regression tests for the cheap/forced ACP runtime discovery split.
 *
 * Two IMPORTANT correctness contracts from the review of the split:
 *
 *  (1) refreshAcpRuntimes() must never coalesce onto an in-flight *cheap*
 *      request. React Query's fetchQuery deduplicates on the shared query key,
 *      so a cheap fetch already running would otherwise satisfy the forced
 *      refresh with cached data and the forced { force: true } probe would
 *      never run. The fix runs the forced probe on a separate key, writes its
 *      result into the shared cache, then cancels the in-flight cheap query.
 *      This test holds a cheap request pending, fires refreshAcpRuntimes(),
 *      resolves the cheap request, and asserts a distinct { force: true } native
 *      call happened and the shared cache holds the forced result.
 *
 *  (2) useAcpRuntimesQueryForced({ forceOnMount: false }) must consume shared
 *      state without mounting its own force effect. Onboarding mounts the hook
 *      once as the surface owner (forceOnMount default true) and once per row
 *      (forceOnMount false); entering the surface must cause exactly one forced
 *      native call before any user action.
 *
 * The Tauri IPC bridge is stubbed at globalThis.__TAURI_INTERNALS__.invoke so
 * discoverAcpRuntimes() calls are intercepted by command name and the { force }
 * payload is observed directly (same pattern as
 * useLoadArchivedObserverEvents.test.mjs).
 */

import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

// ── Minimal DOM shim (subset used by other mounted-hook tests) ────────────────

function installDOMShim() {
  if (globalThis.document) return;

  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      this._listeners[type] ??= [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      this._listeners[type] = (this._listeners[type] ?? []).filter(
        (f) => f !== fn,
      );
    }
    dispatchEvent(e) {
      for (const fn of this._listeners[e.type] ?? []) fn(e);
      return true;
    }
  }

  class MinimalNode extends MinimalEventTarget {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.nodeType = 1;
      this.parentNode = null;
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    get nextSibling() {
      return null;
    }
    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((c) => c !== child);
      this.childNodes = this.childNodes.filter((c) => c !== child);
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.children.indexOf(refNode);
      if (i < 0) return this.appendChild(newNode);
      this.children.splice(i, 0, newNode);
      this.childNodes.splice(i, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    contains(node) {
      if (!node) return false;
      return this === node || this.children.some((c) => c?.contains?.(node));
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      const n = new MinimalNode("#text");
      n.nodeValue = value;
      n.nodeType = 3;
      return n;
    }
    createComment(value) {
      const n = new MinimalNode("#comment");
      n.nodeValue = value;
      n.nodeType = 8;
      return n;
    }
    get body() {
      if (!this._body) this._body = this.createElement("body");
      return this._body;
    }
    get activeElement() {
      return null;
    }
    contains(node) {
      return node != null;
    }
  }

  globalThis.document = new MinimalDocument();
  globalThis.HTMLElement = MinimalNode;
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  if (!Object.getOwnPropertyDescriptor(globalThis, "navigator")?.value) {
    Object.defineProperty(globalThis, "navigator", {
      value: { userAgent: "node" },
      configurable: true,
    });
  }
  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
}

installDOMShim();

// ── Tauri IPC interceptor ─────────────────────────────────────────────────────

/** @type {Array<{ command: string, args: unknown }>} */
const calls = [];
/** @type {(args: unknown) => Promise<unknown>} */
let discoverHandler = () => Promise.resolve([]);

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    calls.push({ command, args });
    if (command === "discover_acp_providers") return discoverHandler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback: () => Math.random(),
};

// ── Production imports (after shim + IPC stub) ────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient } from "@tanstack/react-query";
import { QueryClientProvider } from "@tanstack/react-query";

import {
  acpRuntimesQueryKey,
  refreshAcpRuntimes,
  useAcpRuntimesQueryForced,
} from "./acpRuntimesQuery.ts";
import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery.ts";

// ── Wire-shape helper ─────────────────────────────────────────────────────────

/** A raw discover_acp_providers row (snake_case wire shape). */
function rawEntry(id, authStatusValue) {
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
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: authStatusValue },
    source: "builtin",
  };
}

function makeQueryClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

/** A promise plus its resolver, for holding a request pending. */
function deferred() {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

afterEach(() => {
  calls.length = 0;
  discoverHandler = () => Promise.resolve([]);
});

describe("refreshAcpRuntimes cannot dedup onto an in-flight cheap request", () => {
  it("runs a distinct force:true probe and writes it into the shared cache", async () => {
    const queryClient = makeQueryClient();
    queryClient.mount();

    // 1. A cheap request (force:false) is in flight and held pending.
    const cheap = deferred();
    discoverHandler = (args) => {
      if (args?.force === false) return cheap.promise;
      // 2. The forced request resolves immediately with distinct data.
      return Promise.resolve([rawEntry("codex", "logged_in")]);
    };

    // Start the cheap fetch through the real cheap query path and leave pending.
    const cheapFetch = queryClient.fetchQuery({
      queryKey: acpRuntimesQueryKey,
      queryFn: () => discoverAcpRuntimes(),
      staleTime: 30 * 60_000,
    });
    await new Promise((r) => setImmediate(r));

    // 3. Forced refresh fires while the cheap fetch is still pending.
    const forced = await refreshAcpRuntimes(queryClient);

    // 4. Resolve the cheap request afterward; it must not be what the caller got.
    cheap.resolve([rawEntry("codex", "unknown")]);
    await cheapFetch.catch(() => {});

    const forceCalls = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === true,
    );
    assert.equal(
      forceCalls.length,
      1,
      "exactly one forced native probe must have run",
    );
    assert.equal(forced[0]?.authStatus.status, "logged_in");
    assert.equal(
      queryClient.getQueryData(acpRuntimesQueryKey)?.[0]?.authStatus.status,
      "logged_in",
      "shared cache must hold the forced result, not the later cheap one",
    );

    queryClient.unmount();
  });
});

describe("useAcpRuntimesQueryForced surfaces forced-probe failures", () => {
  it("projects a mount-time forced rejection into error with no unhandled rejection", async () => {
    const unhandled = [];
    const onUnhandled = (err) => unhandled.push(err);
    process.on("unhandledRejection", onUnhandled);

    const queryClient = makeQueryClient();
    discoverHandler = (args) =>
      args?.force === true
        ? Promise.reject(new Error("forced probe failed"))
        : Promise.resolve([]);

    let latest = null;
    function Consumer() {
      latest = useAcpRuntimesQueryForced();
      return null;
    }

    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Consumer),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.equal(
      latest?.error instanceof Error && latest.error.message,
      "forced probe failed",
      "mount-time forced rejection must surface as the hook's error",
    );
    assert.equal(
      latest?.isError,
      true,
      "isError must reflect the forced failure",
    );

    // Drain the microtask queue so any stray rejection would have fired.
    await new Promise((r) => setTimeout(r, 10));
    process.off("unhandledRejection", onUnhandled);
    assert.deepEqual(
      unhandled,
      [],
      "no unhandled rejection may escape the fire-and-forget mount force",
    );

    await act(async () => {
      root.unmount();
    });
  });

  it("surfaces an explicit-refresh rejection and clears it on the next success", async () => {
    const unhandled = [];
    const onUnhandled = (err) => unhandled.push(err);
    process.on("unhandledRejection", onUnhandled);

    const queryClient = makeQueryClient();
    let failForced = true;
    discoverHandler = (args) => {
      if (args?.force !== true) return Promise.resolve([]);
      return failForced
        ? Promise.reject(new Error("refresh failed"))
        : Promise.resolve([rawEntry("codex", "logged_in")]);
    };

    let latest = null;
    function Consumer() {
      // forceOnMount:false so the only forced probe is the explicit refresh.
      latest = useAcpRuntimesQueryForced({ forceOnMount: false });
      return null;
    }

    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Consumer),
        ),
      );
    });

    // Explicit refresh (button/polling shape): void-called, must not reject.
    await act(async () => {
      void latest.forceRefresh();
      await new Promise((r) => setTimeout(r, 50));
    });
    assert.equal(
      latest?.error instanceof Error && latest.error.message,
      "refresh failed",
      "explicit-refresh rejection must surface as the hook's error",
    );

    // A subsequent successful refresh clears the error and delivers data.
    failForced = false;
    await act(async () => {
      void latest.forceRefresh();
      await new Promise((r) => setTimeout(r, 50));
    });
    assert.equal(
      latest?.error,
      null,
      "a later successful refresh clears the error",
    );
    assert.equal(
      queryClient.getQueryData(acpRuntimesQueryKey)?.[0]?.authStatus.status,
      "logged_in",
      "successful refresh writes the fresh catalog into the shared cache",
    );

    await new Promise((r) => setTimeout(r, 10));
    process.off("unhandledRejection", onUnhandled);
    assert.deepEqual(
      unhandled,
      [],
      "no unhandled rejection may escape a void forceRefresh() call",
    );

    await act(async () => {
      root.unmount();
    });
  });
});

describe("useAcpRuntimesQueryForced force-on-mount ownership", () => {
  it("a later-mounted row does not fire a second forced probe", async () => {
    const queryClient = makeQueryClient();
    discoverHandler = () => Promise.resolve([rawEntry("codex", "logged_in")]);

    // Onboarding's real sequence: the surface owner mounts and forces discovery;
    // once its result renders, per-runtime rows mount. A row that shared the
    // owner's default force-on-mount would fire a *second*, sequential forced
    // probe (forced-key dedup cannot collapse it — the owner's fetch is already
    // idle). Rows pass forceOnMount:false to consume shared state only.
    function Owner() {
      useAcpRuntimesQueryForced();
      return null;
    }
    function Row() {
      useAcpRuntimesQueryForced({ forceOnMount: false });
      return null;
    }

    const container = document.createElement("div");
    const root = createRoot(container);

    // 1. Owner mounts and forces once; let the probe settle.
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Owner),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    const afterOwner = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === true,
    ).length;
    assert.equal(afterOwner, 1, "owner mount must force exactly once");

    // 2. Rows mount after the owner's result settled; they must not re-probe.
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Owner),
          React.createElement(Row),
          React.createElement(Row),
          React.createElement(Row),
        ),
      );
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    const forceCalls = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === true,
    );
    assert.equal(
      forceCalls.length,
      1,
      "later-mounted rows must not trigger a second forced probe",
    );
    const cheapCalls = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === false,
    );
    assert.equal(
      cheapCalls.length,
      0,
      "the forced hook must never fire a cheap fetch (enabled: false observer)",
    );

    await act(async () => {
      root.unmount();
    });
  });
});
