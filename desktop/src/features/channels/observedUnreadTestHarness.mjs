/**
 * Shared test infrastructure for observed-unread tests.
 *
 * Exported from a non-test file so the `src/**\/*.test.mjs` glob never picks
 * this up as a test suite. Import into individual test files after installing
 * the DOM shim (installDOMShim must run before any React imports).
 */

// ── DOM shim ──────────────────────────────────────────────────────────────────
// Minimal DOM with full event-listener support so createRoot + pagehide flush
// both work in a plain Node.js process.

export function installDOMShim() {
  class ET {
    constructor() {
      this._ls = {};
    }
    addEventListener(t, fn) {
      this._ls[t] ??= [];
      this._ls[t].push(fn);
    }
    removeEventListener(t, fn) {
      this._ls[t] = (this._ls[t] ?? []).filter((f) => f !== fn);
    }
    dispatchEvent(e) {
      for (const fn of this._ls[e.type] ?? []) fn(e);
      return true;
    }
  }
  class Node extends ET {
    constructor(tag) {
      super();
      this.tagName = tag;
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
    get lastChild() {
      return this.children[this.children.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    get nodeValue() {
      return null;
    }
    appendChild(c) {
      this.children.push(c);
      this.childNodes.push(c);
      c.parentNode = this;
      return c;
    }
    removeChild(c) {
      this.children = this.children.filter((x) => x !== c);
      this.childNodes = this.childNodes.filter((x) => x !== c);
      return c;
    }
    insertBefore(n, r) {
      if (!r) return this.appendChild(n);
      const i = this.children.indexOf(r);
      if (i < 0) return this.appendChild(n);
      this.children.splice(i, 0, n);
      this.childNodes.splice(i, 0, n);
      n.parentNode = this;
      return n;
    }
    contains(n) {
      return this === n || this.children.some((c) => c?.contains?.(n));
    }
  }
  class Doc extends ET {
    constructor() {
      super();
      this.nodeType = 9;
    }
    createElement(t) {
      return new Node(t);
    }
    createTextNode(v) {
      const n = new Node("#text");
      n.nodeType = 3;
      n.nodeValue = v;
      return n;
    }
    createComment(v) {
      const n = new Node("#comment");
      n.nodeType = 8;
      n.nodeValue = v;
      return n;
    }
    get activeElement() {
      return null;
    }
    contains(n) {
      return n != null;
    }
  }

  const windowET = new ET();
  globalThis.document = new Doc();
  globalThis.HTMLIFrameElement = Node;
  globalThis.HTMLElement = Node;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  globalThis.addEventListener = windowET.addEventListener.bind(windowET);
  globalThis.removeEventListener = windowET.removeEventListener.bind(windowET);
  globalThis.dispatchEvent = windowET.dispatchEvent.bind(windowET);
  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
}

// ── localStorage helpers ──────────────────────────────────────────────────────

export function makeLocalStorage() {
  const store = new Map();
  return {
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
    clear: () => store.clear(),
    _store: store,
  };
}

/** Install a fresh isolated localStorage and return it. Configurable so each
 *  test can call this at the top to start with a clean slate. */
export function installFreshStorage() {
  const ls = makeLocalStorage();
  Object.defineProperty(globalThis, "localStorage", {
    get: () => ls,
    configurable: true,
  });
  return ls;
}

/** Swap in an isolated localStorage for the duration of a test, then restore.
 *  Returns { ls, restore }. Use in try/finally blocks. */
export function makeIsolatedStorage() {
  const ls = makeLocalStorage();
  const prevWindow = globalThis.window;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const prevLs = globalThis.window.localStorage;
  globalThis.window.localStorage = ls;
  return {
    ls,
    restore: () => {
      if (prevWindow === undefined) {
        delete globalThis.window;
      } else {
        globalThis.window.localStorage = prevLs;
      }
    },
  };
}

/**
 * Run fn with an isolated localStorage instance, then restore automatically.
 * Equivalent to makeIsolatedStorage() + try/finally restore() but less noisy.
 */
export function withIsolatedStorage(fn) {
  const { restore } = makeIsolatedStorage();
  try {
    fn();
  } finally {
    restore();
  }
}

// ── Event factories ───────────────────────────────────────────────────────────

const _NOW_S = Math.floor(Date.now() / 1_000);

/** Build a minimal observed-unread event object. All fields optional. */
export function makeObservedEvent({
  id = "event-1",
  createdAt = _NOW_S,
  rootId,
  highPriority = false,
  countsTowardBadge = true,
  countsTowardAppBadge = false,
} = {}) {
  return {
    id,
    createdAt,
    rootId: rootId ?? `root-${id}`,
    highPriority,
    countsTowardBadge,
    countsTowardAppBadge,
  };
}

/** Build a Map<channelId, Map<eventId, event>> from [[channelId, [events]]]. */
export function makeEventsByChannel(entries) {
  const map = new Map();
  for (const [channelId, events] of entries) {
    const byId = new Map();
    for (const e of events) {
      byId.set(e.id, e);
    }
    map.set(channelId, byId);
  }
  return map;
}

// ── Hook mount harnesses ──────────────────────────────────────────────────────
// These are imported AFTER the caller runs installDOMShim() and before any
// test body executes, so React/react-dom are resolved in the shimmed env.

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { useObservedUnreadPersistence } from "./useObservedUnreadPersistence.ts";
import { useUnreadChannels } from "./useUnreadChannels.ts";
import { writeObservedUnreadToStorage } from "./observedUnreadStorage.ts";

/**
 * Mount useObservedUnreadPersistence in a harness component.
 * Returns { api, render, unmount }.
 */
export async function mountHook(props, refs) {
  const apiRef = { current: null };

  function Harness({
    pubkey,
    relay,
    isReady,
    readStateVersion,
    getTs,
    getOwn,
    onPruned,
    membershipSeed,
  }) {
    apiRef.current = useObservedUnreadPersistence(
      pubkey,
      relay,
      isReady,
      readStateVersion,
      getTs,
      getOwn,
      refs.eventsRef,
      refs.latestRef,
      { onPruned: onPruned ?? (() => {}), membershipSeed },
    );
    return null;
  }

  const root = createRoot(document.createElement("div"));
  const render = async (p) => {
    await act(async () => {
      root.render(React.createElement(Harness, p));
    });
  };
  await render(props);

  return {
    get api() {
      return apiRef.current;
    },
    render,
    unmount: async () => {
      await act(async () => root.unmount());
    },
  };
}

/**
 * Pre-seed storage for pubkey+relay with one channel's events, then return
 * an ISO timestamp equal to the seeded event's createdAt (for use as readAt).
 */
export function seedStorage(pubkey, relay, channelId, eventId = "evt-1") {
  const inner = new Map();
  const ts = Math.floor(Date.now() / 1_000);
  inner.set(eventId, makeObservedEvent({ id: eventId, createdAt: ts }));
  const events = new Map();
  events.set(channelId, inner);
  writeObservedUnreadToStorage(pubkey, relay, events);
  return new Date(ts * 1_000).toISOString();
}

/**
 * Mount useUnreadChannels inside a QueryClientProvider harness.
 * Returns { markChannelRead, markAllChannelsRead, render, unmount, flushStorage }.
 */
export async function mountUnreadChannels({
  pubkey,
  relay = "wss://relay.example.com",
  channels = [],
  relayClient,
}) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  let capturedMarkChannelRead = null;
  let capturedMarkAllChannelsRead = null;
  let capturedResult = null;

  function Inner({ pubkey: pk }) {
    const result = useUnreadChannels(channels, null, {
      pubkey: pk,
      relayClient,
      relayUrl: relay,
    });
    capturedResult = result;
    capturedMarkChannelRead = result.markChannelRead;
    capturedMarkAllChannelsRead = result.markAllChannelsRead;
    return null;
  }

  function Harness({ pubkey: pk }) {
    return React.createElement(
      QueryClientProvider,
      { client: qc },
      React.createElement(Inner, { pubkey: pk }),
    );
  }

  const container = document.createElement("div");
  const root = createRoot(container);

  const render = async (pk) => {
    await act(async () => {
      root.render(React.createElement(Harness, { pubkey: pk }));
    });
  };

  await render(pubkey);

  return {
    get result() {
      return capturedResult;
    },
    get markChannelRead() {
      return capturedMarkChannelRead;
    },
    get markAllChannelsRead() {
      return capturedMarkAllChannelsRead;
    },
    render,
    unmount: async () => {
      await act(async () => root.unmount());
    },
    flushStorage: () => {
      dispatchEvent(
        new (class extends Event {
          constructor() {
            super("pagehide");
          }
        })(),
      );
    },
  };
}
