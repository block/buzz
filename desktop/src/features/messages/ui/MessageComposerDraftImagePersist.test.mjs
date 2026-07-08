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
 * `useDraftPersistLifecycle` (in useDraftPersistSnapshot.ts, extracted from
 * `MessageComposer`) owns the full restore/persist lifecycle. Inside the
 * effect body it writes `pendingImetaForPersistRef.current = saved.pendingImeta`
 * SYNCHRONOUSLY — before the async `setPendingImeta` call. Because the write
 * is synchronous (same microtask as the effect body), the cleanup closure
 * always sees the restored value even when StrictMode fires the
 * simulate-unmount before React commits the state update.
 *
 * ── What this test does ───────────────────────────────────────────────────────
 * We import and mount `useDraftPersistLifecycle` — the REAL production hook —
 * inside `<React.StrictMode>`. The harness component calls the hook directly
 * and provides thin stub collaborators. The hook owns the effect body and
 * cleanup; the harness does NOT replicate the restore/persist logic.
 *
 * **Hard requirement**: removing the synchronous
 * `pendingImetaForPersistRef.current = saved.pendingImeta` write from the
 * production hook's effect body causes test 1 to fail (imageCount 1 → 0),
 * because the cleanup reads the stale `[]` and overwrites the saved draft.
 * This was verified in isolation before commit.
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

// Production hook under test — owns the restore effect, cleanup, and the
// synchronous ref write that is the StrictMode fix.
import { useDraftPersistLifecycle } from "./useDraftPersistSnapshot.ts";

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
 * Mounts a thin harness component that calls the REAL `useDraftPersistLifecycle`
 * hook under `<React.StrictMode>`. The hook owns the effect body and cleanup —
 * the harness provides stub collaborators but does NOT replicate any of the
 * restore/persist logic.
 *
 * The hook's own effect body writes `pendingImetaForPersistRef.current =
 * saved.pendingImeta` synchronously before `setPendingImeta`. StrictMode's
 * simulate-unmount cleanup fires before `setPendingImeta` commits, but reads
 * the correct `[IMG_A]` from the ref.
 *
 * **Revert verification (confirmed before commit)**: removing the synchronous
 * `pendingImetaForPersistRef.current = saved.pendingImeta` line from the
 * production hook's effect body causes imageCount 1 → 0 and this test fails.
 */
test("strictmode_draft_restore_cleanup_preserves_images_via_production_hook", async () => {
  const DRAFT_KEY = "chan-lifecycle-fixed";
  setupStore("pubkey-lifecycle-fixed");

  // Seed: saved draft has an image.
  persistDraftEntry(DRAFT_KEY, "hello from A", DRAFT_KEY, [IMG_A], []);
  assert.equal(
    loadDraftEntry(DRAFT_KEY)?.pendingImeta.length,
    1,
    "precondition: store has the image",
  );

  // `asyncState` simulates media.pendingImeta: starts at [] on fresh mount
  // (no committed state yet — the state update from setPendingImeta hasn't
  // committed before StrictMode's simulate-unmount fires).
  let asyncState = [];
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    // The hook owns all draft-persist lifecycle — no restore/cleanup logic here.
    useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: (key) => loadDraftEntry(key),
      persistDraft: (key, content, channelId, pendingImeta, spoileredUrls) => {
        persistDraftEntry(key, content, channelId, pendingImeta, spoileredUrls);
      },
      livePendingImeta: asyncState,
      setPendingImeta: (imeta) => {
        asyncState = imeta; // async — won't commit before StrictMode cleanup
      },
      setContent: () => {},
      clearContent: () => {},
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => "hello from A",
    });

    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);

  // After StrictMode double-invoke, the store must still contain the image.
  // If the synchronous ref write were removed from the production hook,
  // the first cleanup would persist [] and this assertion fails (imageCount=0).
  const afterMount = loadDraftEntry(DRAFT_KEY);
  assert.ok(afterMount, "draft must still exist after StrictMode mount");
  assert.equal(
    afterMount.pendingImeta.length,
    1,
    "image must survive StrictMode simulate-unmount cleanup — requires the synchronous ref write in useDraftPersistLifecycle's effect body",
  );
  assert.equal(afterMount.pendingImeta[0].url, IMG_A.url);

  await handle.unmount();
});

/**
 * Test 2: no-draft path clears correctly under StrictMode.
 *
 * When there is no saved draft for the key, the hook takes the `else` branch
 * and sets `pendingImetaForPersistRef.current = []`. The cleanup persists `[]`.
 * Verifies the else-branch synchronous write is also correct under StrictMode.
 */
test("strictmode_draft_no_draft_cleanup_persists_empty_imeta", async () => {
  const DRAFT_KEY = "chan-lifecycle-nodraft";
  setupStore("pubkey-lifecycle-nodraft");
  // No draft seeded — loadDraftEntry returns undefined.

  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: (key) => loadDraftEntry(key),
      persistDraft: (key, content, channelId, pendingImeta, spoileredUrls) => {
        persistDraftEntry(key, content, channelId, pendingImeta, spoileredUrls);
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: () => {},
      clearContent: () => {},
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => "",
    });

    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);

  // No draft existed; the hook took the else branch. Cleanup writes an empty
  // entry (or skips if key is falsy, but DRAFT_KEY is defined here).
  const afterMount = loadDraftEntry(DRAFT_KEY);
  const imageCount = afterMount?.pendingImeta?.length ?? 0;
  assert.equal(
    imageCount,
    0,
    "no-draft path: cleanup must persist empty imeta, not a stale value",
  );

  await handle.unmount();
});
