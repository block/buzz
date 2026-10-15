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
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { QueryClientProvider } from "@tanstack/react-query";

import {
  acpRuntimesQueryKey,
  applyBootWarmGate,
  getBootWarmSnapshot,
  refreshAcpRuntimes,
  startBootWarm,
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

// Runs FIRST so the process-global boot-warm gate is observed from `idle`.
// Covers Carl's ask: the cheap/forced race (a cold cheap catalog must read as
// loading, not authoritative, while the first forced pass is in flight) and the
// failure state (a failed forced pass must surface a retryable error carrying
// the real reason, not a silent empty catalog), plus recovery on retry.
describe("boot-warm gate drives cheap consumers through the initial pass", () => {
  it("applyBootWarmGate: a non-empty cold catalog is not authoritative while pending or failed", () => {
    // The real cold cheap response is NEVER empty: discovery always emits the
    // known runtimes as not_installed/cli_missing rows plus presets. Model that
    // wire shape so the gate is exercised against the payload it exists to
    // gate, not a `[]` that never occurs in production.
    const coldCatalog = {
      data: [
        rawEntry("codex", "unknown"),
        rawEntry("goose", "unknown"),
        rawEntry("claude-code", "unknown"),
      ],
      error: null,
      isLoading: false,
      isPending: false,
      isFetching: false,
      isError: false,
    };
    // A consumer maps `isLoading -> "loading"`, `isError -> "error"`, else
    // `"ready"`. "Ready" is what blesses the cold rows as authoritative — the
    // exact P2 defect. Assert neither pending nor failed reads as ready.
    const readsAsReady = (q) => !q.isLoading && !q.isError;

    const pending = applyBootWarmGate(coldCatalog, {
      status: "pending",
      error: null,
    });
    assert.equal(pending.isLoading, true);
    assert.equal(pending.isPending, true);
    assert.equal(
      readsAsReady(pending),
      false,
      "pending must not read as ready",
    );
    // The catalog rows are preserved so a consumer reading `data ?? []` keeps
    // them; only the lifecycle flags are overlaid.
    assert.equal(pending.data.length, 3);

    const reason = new Error("PATH probe timed out");
    const failed = applyBootWarmGate(coldCatalog, {
      status: "failed",
      error: reason,
    });
    assert.equal(failed.isError, true);
    assert.equal(failed.error, reason);
    assert.equal(readsAsReady(failed), false, "failed must not read as ready");
    assert.equal(failed.data.length, 3);

    // idle/settled pass through untouched: onboarding renders before the warm
    // starts (idle) and the warmed hot path (settled) must both read as ready.
    for (const status of ["idle", "settled"]) {
      const passed = applyBootWarmGate(coldCatalog, { status, error: null });
      assert.equal(passed.isLoading, false);
      assert.equal(passed.isError, false);
      assert.equal(readsAsReady(passed), true, `${status} must read as ready`);
    }
  });

  it("applyBootWarmGate: a warmed non-empty catalog reads as ready once settled", () => {
    const warm = {
      data: [rawEntry("codex", "logged_in")],
      error: null,
      isLoading: false,
      isPending: false,
      isFetching: false,
      isError: false,
    };
    const settled = applyBootWarmGate(warm, { status: "settled", error: null });
    assert.equal(settled.isLoading, false);
    assert.equal(settled.isError, false);
    assert.equal(settled.data.length, 1);
  });

  it("applyBootWarmGate: failed reads as a retryable error with the real reason", () => {
    const cold = {
      data: [],
      error: null,
      isLoading: true,
      isPending: true,
      isFetching: true,
      isError: false,
    };
    const reason = new Error("PATH probe timed out");
    const failed = applyBootWarmGate(cold, { status: "failed", error: reason });
    assert.equal(failed.isError, true);
    assert.equal(failed.error, reason);
    assert.equal(failed.isLoading, false, "a failed warm is not still loading");
  });

  it("startBootWarm: failure marks the gate failed, a retry settles it", async () => {
    assert.equal(
      getBootWarmSnapshot().status,
      "idle",
      "gate must start idle before any warm",
    );

    const queryClient = makeQueryClient();
    queryClient.mount();

    // 1. First forced pass fails: the gate goes `failed` and captures the
    //    reason, so cold cheap surfaces can show a retryable error.
    let failForced = true;
    discoverHandler = (args) =>
      args?.force === true && failForced
        ? Promise.reject(new Error("discovery boom"))
        : Promise.resolve([]);
    await startBootWarm(queryClient);
    assert.equal(getBootWarmSnapshot().status, "failed");
    assert.equal(getBootWarmSnapshot().error?.message, "discovery boom");

    // 2. A retry that succeeds settles the gate and clears the error, so cheap
    //    consumers stop overlaying and render the warmed catalog.
    failForced = false;
    discoverHandler = () => Promise.resolve([rawEntry("codex", "logged_in")]);
    await startBootWarm(queryClient);
    assert.equal(getBootWarmSnapshot().status, "settled");
    assert.equal(getBootWarmSnapshot().error, null);

    // 3. Once settled, further boot warms are no-ops (fixes the per-remount
    //    re-fire): no additional forced probe fires.
    const before = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === true,
    ).length;
    await startBootWarm(queryClient);
    const after = calls.filter(
      (c) => c.command === "discover_acp_providers" && c.args?.force === true,
    ).length;
    assert.equal(after, before, "a settled gate must not re-fire the probe");

    queryClient.unmount();
  });
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

  it("an in-flight cheap query cannot clobber the forced result after refresh", async () => {
    // Carl's settle-order finding: a cheap query in flight on the shared key
    // must not land its (older) result after the forced catalog is written.
    // `refreshAcpRuntimes` cancels the shared-key query before settling; this
    // proves the cancel is load-bearing by holding a real cheap observer
    // fetching, running the forced refresh, then resolving the cheap request
    // late — its result must not overwrite the forced catalog, and the gate
    // must settle on the forced state. (Removing the `cancelQueries` call makes
    // the late cheap result win and fails this test.)
    const queryClient = makeQueryClient();
    queryClient.mount();

    // Seed a pre-existing cold catalog, then start a mounted cheap observer that
    // refetches and is held pending — the real in-flight shape.
    queryClient.setQueryData(acpRuntimesQueryKey, [
      rawEntry("codex", "unknown"),
    ]);
    const cheap = deferred();
    discoverHandler = (args) => {
      if (args?.force === false) return cheap.promise;
      return Promise.resolve([rawEntry("codex", "logged_in")]);
    };
    const observer = new QueryObserver(queryClient, {
      queryKey: acpRuntimesQueryKey,
      queryFn: () => discoverAcpRuntimes(),
      staleTime: 0,
    });
    const unsubscribe = observer.subscribe(() => {});
    await new Promise((r) => setImmediate(r));

    // Forced refresh completes and settles while the cheap observer is fetching.
    await refreshAcpRuntimes(queryClient);

    // The cheap request resolves afterward; its result must be dropped.
    cheap.resolve([rawEntry("codex", "unknown")]);
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setImmediate(r));

    assert.equal(
      queryClient.getQueryData(acpRuntimesQueryKey)?.[0]?.authStatus.status,
      "logged_in",
      "shared cache must remain the forced result after a late cheap resolution",
    );
    assert.equal(
      getBootWarmSnapshot().status,
      "settled",
      "the gate must settle on the forced catalog, not the stale cheap state",
    );

    unsubscribe();
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
