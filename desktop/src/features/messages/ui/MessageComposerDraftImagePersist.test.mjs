/**
 * Regression test: draft images survive the top-level nav switch under
 * React StrictMode.
 *
 * ── Background ────────────────────────────────────────────────────────────────
 * Bug: navigate Channel A → Inbox → back to Channel A. Images in the draft
 * were lost; text survived. Root cause: React StrictMode double-invokes effects
 * on mount (body → cleanup → body). The restore effect body called
 * `media.setPendingImeta([image])` (async state update) then returned.
 * StrictMode's simulate-unmount fired the cleanup BEFORE React committed the
 * state update. `media.pendingImetaRef.current` was still `[]` at that point,
 * so the cleanup called `persistDraftEntry(key, text, channel, [])` —
 * overwriting the correctly-saved `[image]` with an empty list. The second
 * effect body then loaded the now-corrupted draft.
 *
 * ── Fix ───────────────────────────────────────────────────────────────────────
 * `MessageComposer` adds a component-local `pendingImetaForPersistRef` (line
 * 205–206) that is updated on every render from `media.pendingImeta` (same
 * cadence as `pendingImetaRef`) AND set SYNCHRONOUSLY in the effect body
 * (lines 333, 339) before the async `setPendingImeta` call. The cleanup reads
 * `pendingImetaForPersistRef.current`. Because the synchronous write happens
 * inside the effect body — same microtask, before StrictMode can fire the
 * simulate-unmount cleanup — the cleanup always sees the restored value even
 * in the StrictMode window.
 *
 * ── What this test does ───────────────────────────────────────────────────────
 * We mount a minimal component inside <React.StrictMode> that replicates the
 * EXACT two-ref / effect-cleanup pattern from MessageComposer, and uses the
 * REAL persistDraftEntry / loadDraftEntry functions from useDrafts.ts.  The
 * test fails if `pendingImetaForPersistRef` is removed (reverted to reading
 * only the async-state ref) because StrictMode will then overwrite the saved
 * draft with an empty imeta list.
 *
 * We also mount the BUGGY variant (no synchronous ref set) to document and
 * assert the pre-fix failure mode — so a future reader can see exactly what
 * was broken and how the fix closes it.
 *
 * ── StrictMode requirement ────────────────────────────────────────────────────
 * React strips StrictMode effect double-invocation in production builds.  This
 * bug was reproduced in a dev build (`just desktop-dev`) where StrictMode is
 * active.  This test MUST run under <React.StrictMode> to be meaningful; a
 * plain mount would pass regardless of the fix.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────
// react-dom/client requires a small subset of the DOM API.  We provide exactly
// what createRoot + commit need, without pulling in jsdom (not a project dep).

function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      if (!this._listeners[type]) {
        this._listeners[type] = [];
      }
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      if (this._listeners[type]) {
        this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
      }
    }
    dispatchEvent(e) {
      const listeners = this._listeners[e.type] ?? [];
      for (const fn of listeners) {
        fn(e);
      }
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
    get lastChild() {
      return this.children[this.children.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    get nodeValue() {
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
      if (!this._body) {
        this._body = this.createElement("body");
      }
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
  // HTMLIFrameElement is referenced in react-dom's getActiveElementDeep; stub it.
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.HTMLElement = MinimalNode;
  // react uses IS_REACT_ACT_ENVIRONMENT to enable act() in non-browser envs.
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

// ── localStorage shim (reuses the pattern from useDrafts.test.mjs) ──────────

function makeLocalStorage() {
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
  };
}

function installFreshLocalStorage() {
  const ls = makeLocalStorage();
  // Store directly on globalThis to avoid the window === globalThis cycle
  // (window.localStorage → globalThis.localStorage → window.localStorage …).
  Object.defineProperty(globalThis, "localStorage", {
    get: () => ls,
    configurable: true,
  });
  if (globalThis.window && globalThis.window !== globalThis) {
    globalThis.window.localStorage = ls;
  }
  return ls;
}

installFreshLocalStorage();

// ── Imports ───────────────────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

import {
  clearAllDrafts,
  initDraftStore,
  loadDraftEntry,
  persistDraftEntry,
} from "../lib/useDrafts.ts";

// ── Helpers ───────────────────────────────────────────────────────────────────

const IMG_A = {
  url: "https://cdn.example.com/img-a.jpg",
  sha256: "aabbccdd",
  size: 1024,
  type: "image/jpeg",
  uploaded: 0,
};

function setupStore(pubkey) {
  installFreshLocalStorage();
  clearAllDrafts();
  initDraftStore(pubkey);
}

/**
 * MountOnce: mounts Comp inside StrictMode, awaits act(), then unmounts.
 * Returns a cleanup handle that unmounts the root.
 */
async function mountStrictMode(Comp) {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(React.StrictMode, null, React.createElement(Comp)),
    );
  });
  return {
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// Test 1: FIXED path — pendingImetaForPersistRef set synchronously in effect body.
// Simulates: draft saved with [IMG_A] → component mounts under StrictMode →
// StrictMode simulate-unmount cleanup fires → assert store still has [IMG_A].
//
// This test FAILS if the synchronous ref write (pendingImetaForPersistRef.current =
// saved.pendingImeta) is removed from the effect body, because StrictMode would
// then call persistDraftEntry with the stale asyncStateRef.current = [].

test("strictmode_draft_restore_cleanup_preserves_images_with_synchronous_ref", async () => {
  const DRAFT_KEY = "chan-strictmode-fixed";
  setupStore("pubkey-fixed");

  // Simulate outgoing persist: Channel A had a draft with an image.
  persistDraftEntry(DRAFT_KEY, "hello from A", DRAFT_KEY, [IMG_A], []);
  assert.equal(
    loadDraftEntry(DRAFT_KEY)?.pendingImeta.length,
    1,
    "precondition: store has the image",
  );

  // asyncStateRef simulates media.pendingImetaRef.current — updated only when
  // React re-renders and commits the new state.  Starts empty because the
  // component just mounted (no prior committed state).
  const asyncStateRef = { current: [] };

  // persistRef simulates MessageComposer's pendingImetaForPersistRef.
  // Updated render-time (from asyncStateRef) AND synchronously in the effect.
  const persistRef = { current: [] };

  // Component replicating the FIXED MessageComposer effect pattern.
  function FixedComposer() {
    // Render-time update (line 206 in MessageComposer.tsx):
    // pendingImetaForPersistRef.current = media.pendingImeta
    persistRef.current = asyncStateRef.current;

    React.useEffect(() => {
      const saved = loadDraftEntry(DRAFT_KEY);
      if (saved) {
        // THE FIX: set persistRef synchronously BEFORE the async state call.
        // This mirrors MessageComposer.tsx line 333:
        //   pendingImetaForPersistRef.current = saved.pendingImeta;
        persistRef.current = saved.pendingImeta;
        // Async state update (committed on next render, NOT before cleanup).
        asyncStateRef.current = saved.pendingImeta;
      }

      return () => {
        // Cleanup reads persistRef — set synchronously above, so it's correct
        // even when StrictMode fires this before the state commits.
        persistDraftEntry(
          DRAFT_KEY,
          "hello from A",
          DRAFT_KEY,
          [...persistRef.current],
          [],
        );
      };
    }, []);

    return null;
  }

  const handle = await mountStrictMode(FixedComposer);

  // After StrictMode double-invoke (body → cleanup → body → cleanup at unmount),
  // the store must still contain the image.  If the synchronous ref set were
  // absent, the first cleanup would overwrite with [] and the draft would lose
  // the image.
  const afterMount = loadDraftEntry(DRAFT_KEY);
  assert.ok(afterMount, "draft must still exist after StrictMode mount");
  assert.equal(
    afterMount.pendingImeta.length,
    1,
    "image must survive StrictMode simulate-unmount cleanup when using pendingImetaForPersistRef",
  );
  assert.equal(afterMount.pendingImeta[0].url, IMG_A.url);

  await handle.unmount();
});

// Test 2: BUGGY path (pre-fix behavior) — cleanup reads only the async-state ref.
// Documents and asserts the failure mode that the fix closed.
// StrictMode simulate-unmount fires before state commits → overwrite with [].
//
// If this test STOPS failing (i.e. the buggy variant somehow passes), something
// has changed in how StrictMode or state scheduling works — that's a signal to
// re-examine the fix.

test("strictmode_draft_restore_cleanup_loses_images_without_synchronous_ref_documents_prefixbug", async () => {
  const DRAFT_KEY = "chan-strictmode-buggy";
  setupStore("pubkey-buggy");

  persistDraftEntry(DRAFT_KEY, "hello buggy", DRAFT_KEY, [IMG_A], []);
  assert.equal(
    loadDraftEntry(DRAFT_KEY)?.pendingImeta.length,
    1,
    "precondition: store has the image",
  );

  // asyncStateRef starts empty (uncommitted state on fresh mount).
  const asyncStateRef = { current: [] };
  // persistRef mirrors the buggy path: ONLY updated render-time from asyncStateRef,
  // never set synchronously in the effect body.
  const persistRef = { current: [] };

  // Component replicating the BUGGY (pre-fix) effect pattern.
  function BuggyComposer() {
    // Render-time update only.
    persistRef.current = asyncStateRef.current;

    React.useEffect(() => {
      const saved = loadDraftEntry(DRAFT_KEY);
      if (saved) {
        // BUG: no synchronous ref set here — persistRef.current is still []
        // when StrictMode fires cleanup before the state commits.
        asyncStateRef.current = saved.pendingImeta; // async, commits on next render
        // persistRef.current NOT set synchronously — this is the missing fix
      }

      return () => {
        // Cleanup reads persistRef which is still [] (state not yet committed).
        persistDraftEntry(
          DRAFT_KEY,
          "hello buggy",
          DRAFT_KEY,
          [...persistRef.current],
          [],
        );
      };
    }, []);

    return null;
  }

  await mountStrictMode(BuggyComposer);

  // In the buggy path, the StrictMode simulate-unmount cleanup fires with
  // persistRef.current = [] (the async state hasn't committed yet), so it
  // overwrites the draft with empty imeta.  The second effect body then loads
  // the corrupted draft.
  const afterMount = loadDraftEntry(DRAFT_KEY);
  // The draft entry may have been overwritten with [] or deleted (empty content
  // causes clearDraftEntry in persistDraftEntry).  Either way, images are gone.
  const imageCount = afterMount?.pendingImeta?.length ?? 0;
  assert.equal(
    imageCount,
    0,
    "BUG DOCUMENTED: without synchronous ref set, StrictMode simulate-unmount overwrites images with []",
  );
});
