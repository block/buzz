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
 * `useDraftPersistSnapshot` (extracted from `MessageComposer`) owns the
 * persist-snapshot ref and exposes `snapshotPendingImeta(imeta)` — a function
 * that sets the ref SYNCHRONOUSLY. `MessageComposer` calls
 * `snapshotPendingImeta(saved.pendingImeta)` in the effect body before the
 * async `setPendingImeta` call. Because the write is synchronous (same
 * microtask as the effect body), the cleanup closure always sees the restored
 * value even when StrictMode fires the simulate-unmount before React commits
 * the state update.
 *
 * ── What this test does ───────────────────────────────────────────────────────
 * We import the REAL `useDraftPersistSnapshot` hook from production code and
 * mount a thin harness component inside `<React.StrictMode>`. The harness
 * calls the real hook, exercises the same effect-body → cleanup path that
 * `MessageComposer` uses, and uses the real `persistDraftEntry` /
 * `loadDraftEntry` storage functions from `useDrafts.ts`.
 *
 * Removing `snapshotPendingImeta(saved.pendingImeta)` from the production
 * `snapshotPendingImeta` implementation (i.e. making it a no-op) causes the
 * first test to fail because the cleanup reads the stale `[]` and overwrites
 * the saved draft — verified in the revert-verification section below.
 *
 * ── StrictMode requirement ────────────────────────────────────────────────────
 * React strips StrictMode effect double-invocation in production builds.
 * This bug was reproduced in a dev build (`just desktop-dev`) where StrictMode
 * is active. This test MUST run under `<React.StrictMode>` to be meaningful;
 * a plain mount would pass regardless of the fix.
 *
 * ── CI surface ────────────────────────────────────────────────────────────────
 * Runs under `pnpm test` (node:test with the React dev build). Not Playwright.
 * A packaged-build E2E would not reproduce the bug.
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

// ── localStorage shim ─────────────────────────────────────────────────────────

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
  // Avoid the window === globalThis cycle by binding the getter to the captured
  // ls reference directly.
  Object.defineProperty(globalThis, "localStorage", {
    get: () => ls,
    configurable: true,
  });
  return ls;
}

installFreshLocalStorage();

// ── Imports ───────────────────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

// Production hook under test — the synchronous ref write lives here.
import { useDraftPersistSnapshot } from "./useDraftPersistSnapshot.ts";

// Real storage functions — the test uses them, not a replica.
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

/**
 * Test 1: the FIXED path.
 *
 * Mounts a harness component that uses the REAL `useDraftPersistSnapshot` hook
 * under `<React.StrictMode>`. The harness mirrors the exact pattern
 * `MessageComposer` uses:
 *   1. render-time: hook updates `pendingImetaForPersistRef` from `asyncState`
 *   2. effect body: calls `snapshotPendingImeta(saved.pendingImeta)` SYNCHRONOUSLY
 *      (via the real production hook) before the async state update
 *   3. cleanup: persists `[...pendingImetaForPersistRef.current]`
 *
 * StrictMode fires: body → cleanup → body → cleanup (on unmount). The first
 * cleanup (StrictMode simulate-unmount) fires before asyncState commits.
 * `snapshotPendingImeta` sets the ref synchronously in step 2, so the cleanup
 * reads `[IMG_A]` not `[]`.
 *
 * **Revert verification**: removing the body of `snapshotPendingImeta` in the
 * production hook (making it a no-op) causes imageCount to be 0 (the cleanup
 * reads the uncommitted `[]`), failing the assertion.
 */
test("strictmode_draft_restore_cleanup_preserves_images_via_production_hook", async () => {
  const DRAFT_KEY = "chan-hook-fixed";
  setupStore("pubkey-hook-fixed");

  // Outgoing persist: saved draft has an image.
  persistDraftEntry(DRAFT_KEY, "hello from A", DRAFT_KEY, [IMG_A], []);
  assert.equal(
    loadDraftEntry(DRAFT_KEY)?.pendingImeta.length,
    1,
    "precondition: store has the image",
  );

  // `asyncState` simulates media.pendingImeta — starts at [] (uncommitted
  // state on fresh mount, like the real composer on a nav return).
  let asyncState = [];

  // Harness component: uses the REAL useDraftPersistSnapshot hook.
  function HarnessComposer() {
    // render-time: hook keeps ref in sync with committed state (same as
    // MessageComposer line 207: pendingImetaForPersistRef.current = media.pendingImeta)
    const { pendingImetaForPersistRef, snapshotPendingImeta } =
      useDraftPersistSnapshot(asyncState);

    // biome-ignore lint/correctness/useExhaustiveDependencies: single-mount effect, ref and snapshot fn are stable
    React.useEffect(() => {
      const saved = loadDraftEntry(DRAFT_KEY);
      if (saved) {
        // THE FIX (production): calls snapshotPendingImeta synchronously —
        // same as MessageComposer.tsx line 330: snapshotPendingImeta(saved.pendingImeta)
        snapshotPendingImeta(saved.pendingImeta);
        // Async state update — won't commit before StrictMode simulate-unmount.
        asyncState = saved.pendingImeta;
      }

      return () => {
        // Cleanup mirrors MessageComposer.tsx line 353-356.
        persistDraftEntry(
          DRAFT_KEY,
          "hello from A",
          DRAFT_KEY,
          [...pendingImetaForPersistRef.current],
          [],
        );
      };
    }, []);

    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);

  // After StrictMode double-invoke, the store must still contain the image.
  // If snapshotPendingImeta were a no-op, the first cleanup would persist []
  // and this assertion would fail with imageCount = 0.
  const afterMount = loadDraftEntry(DRAFT_KEY);
  assert.ok(afterMount, "draft must still exist after StrictMode mount");
  assert.equal(
    afterMount.pendingImeta.length,
    1,
    "image must survive StrictMode simulate-unmount cleanup — requires snapshotPendingImeta to set the ref synchronously in the production hook",
  );
  assert.equal(afterMount.pendingImeta[0].url, IMG_A.url);

  await handle.unmount();
});

/**
 * Test 2: documents the pre-fix failure mode.
 *
 * Same harness, but `snapshotPendingImeta` is NOT called in the effect body —
 * only the async state update happens. The StrictMode simulate-unmount cleanup
 * fires before state commits and reads the stale `[]`, overwriting the draft.
 *
 * This test asserts the BROKEN behavior so a future reader understands exactly
 * what was wrong. If this test stops failing (i.e. the bug is somehow
 * self-correcting), that's a signal to re-examine the fix.
 */
test("strictmode_draft_restore_cleanup_loses_images_when_snapshot_not_called", async () => {
  const DRAFT_KEY = "chan-hook-buggy";
  setupStore("pubkey-hook-buggy");

  persistDraftEntry(DRAFT_KEY, "hello buggy", DRAFT_KEY, [IMG_A], []);
  assert.equal(
    loadDraftEntry(DRAFT_KEY)?.pendingImeta.length,
    1,
    "precondition: store has the image",
  );

  let asyncState = [];

  // Harness: uses the real hook but does NOT call snapshotPendingImeta
  // in the effect body — models the pre-fix MessageComposer.
  function BuggyHarnessComposer() {
    const { pendingImetaForPersistRef } = useDraftPersistSnapshot(asyncState);

    // biome-ignore lint/correctness/useExhaustiveDependencies: single-mount effect, ref is stable
    React.useEffect(() => {
      const saved = loadDraftEntry(DRAFT_KEY);
      if (saved) {
        // BUG: no snapshotPendingImeta call — ref stays at [] until next render.
        asyncState = saved.pendingImeta; // async, won't commit before StrictMode cleanup
      }

      return () => {
        // Cleanup reads pendingImetaForPersistRef.current, which is still []
        // because snapshotPendingImeta was never called.
        persistDraftEntry(
          DRAFT_KEY,
          "hello buggy",
          DRAFT_KEY,
          [...pendingImetaForPersistRef.current],
          [],
        );
      };
    }, []);

    return null;
  }

  await mountStrictMode(BuggyHarnessComposer);

  const afterMount = loadDraftEntry(DRAFT_KEY);
  const imageCount = afterMount?.pendingImeta?.length ?? 0;
  assert.equal(
    imageCount,
    0,
    "BUG DOCUMENTED: without snapshotPendingImeta call, StrictMode simulate-unmount overwrites images with []",
  );
});
