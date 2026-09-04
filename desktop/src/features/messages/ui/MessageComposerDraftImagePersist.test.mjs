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
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
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
import { useAgentPrefillDraftBridge } from "./useAgentPrefillDraftBridge.ts";

// Real storage functions — the test uses them, not a replica.
import {
  clearAllDrafts,
  deleteDraftEntry,
  getInboxDraftEntries,
  initDraftStore,
  loadDraftEntry,
  persistDraftEntry,
  subscribeToStore,
} from "../lib/useDrafts.ts";
import {
  saveQueuedAttachmentsForDraft,
  takeQueuedAttachmentsForDraft,
} from "../lib/backgroundMediaUploadStore.ts";

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
    rerender: async () => {
      await act(async () => {
        root.render(
          React.createElement(
            React.StrictMode,
            null,
            React.createElement(Comp),
          ),
        );
      });
    },
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
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
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
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
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

test("draft_lifecycle_restores_and_repersists_mention_routing_refs", async () => {
  const DRAFT_KEY = "chan-lifecycle-mentions";
  const MENTION_REFS = [
    { displayName: "Agent Ada", pubkey: "abcdef1234", isAgent: true },
  ];
  setupStore("pubkey-lifecycle-mentions");
  persistDraftEntry(
    DRAFT_KEY,
    "hello @Agent Ada",
    DRAFT_KEY,
    [],
    [],
    MENTION_REFS,
  );

  let restoredRefs = null;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        content.includes("@Agent Ada") ? MENTION_REFS : [],
      restoreMentionRefs: (refs) => {
        restoredRefs = [...refs];
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: () => {},
      clearContent: () => {},
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => "hello @Agent Ada",
    });
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.deepEqual(restoredRefs, MENTION_REFS);
  assert.deepEqual(loadDraftEntry(DRAFT_KEY)?.mentionRefs, MENTION_REFS);
  await handle.unmount();
});

test("draft_lifecycle_switches_a_b_a_without_leaking_or_losing_mention_refs", async () => {
  const REFS_A = [
    { displayName: "Agent Ada", pubkey: "aaaaaaaa", isAgent: true },
  ];
  const REFS_B = [
    { displayName: "Person Bea", pubkey: "bbbbbbbb", isAgent: false },
  ];
  setupStore("pubkey-switch-mentions");
  persistDraftEntry("chan-a", "hello @Agent Ada", "chan-a", [], [], REFS_A);
  persistDraftEntry("chan-b", "hello @Person Bea", "chan-b", [], [], REFS_B);

  let draftKey = "chan-a";
  let editorContent = "";
  let activeRefs = [];
  const restored = [];
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        activeRefs.filter((ref) => content.includes(`@${ref.displayName}`)),
      restoreMentionRefs: (refs) => {
        activeRefs = [...refs];
        restored.push(activeRefs);
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    });
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.deepEqual(activeRefs, REFS_A);

  draftKey = "chan-b";
  await handle.rerender();
  assert.deepEqual(activeRefs, REFS_B, "B replaces A refs rather than merging");

  draftKey = "chan-a";
  await handle.rerender();
  assert.deepEqual(
    activeRefs,
    REFS_A,
    "A refs survive a full A → B → A switch",
  );
  assert.deepEqual(loadDraftEntry("chan-a")?.mentionRefs, REFS_A);
  assert.deepEqual(loadDraftEntry("chan-b")?.mentionRefs, REFS_B);
  assert.equal(
    restored.some((refs) => refs.length === 0),
    false,
    "saved channel switches never transiently restore empty routing refs",
  );

  await handle.unmount();
});

test("draft_lifecycle_empty_target_clears_stale_mention_refs", async () => {
  const REFS_A = [
    { displayName: "Agent Ada", pubkey: "aaaaaaaa", isAgent: true },
  ];
  setupStore("pubkey-empty-target-mentions");
  persistDraftEntry("chan-a", "hello @Agent Ada", "chan-a", [], [], REFS_A);

  let draftKey = "chan-a";
  let editorContent = "";
  let activeRefs = [];
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        activeRefs.filter((ref) => content.includes(`@${ref.displayName}`)),
      restoreMentionRefs: (refs) => {
        activeRefs = [...refs];
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    });
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.deepEqual(activeRefs, REFS_A);

  draftKey = "chan-empty";
  await handle.rerender();
  assert.deepEqual(activeRefs, []);
  assert.equal(editorContent, "");
  assert.equal(loadDraftEntry("chan-empty"), undefined);

  await handle.unmount();
});

test("draft_lifecycle_persists_an_explicit_clear_before_async_rerender", async () => {
  const DRAFT_KEY = "chan-clear-race";
  setupStore("pubkey-clear-race");
  persistDraftEntry(DRAFT_KEY, "draft text", DRAFT_KEY, [], []);

  let editorContent = "";
  let trackAuthoredContent;
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    ({ trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "draft text");

  trackAuthoredContent("");
  assert.equal(
    loadDraftEntry(DRAFT_KEY),
    undefined,
    "the authoritative update removes the stale body even while editor reads lag",
  );

  await handle.unmount();
  assert.equal(
    loadDraftEntry(DRAFT_KEY),
    undefined,
    "async settlement must not repersist the deleted body",
  );

  const remounted = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "", "the deleted body must not be restored");
  await remounted.unmount();
});

test("draft_lifecycle_clear_caption_preserves_image_and_spoiler_on_remount", async () => {
  const DRAFT_KEY = "chan-clear-caption-image";
  setupStore("pubkey-clear-caption-image");
  persistDraftEntry(DRAFT_KEY, "caption", DRAFT_KEY, [IMG_A], [IMG_A.url]);

  let editorContent = "";
  let pendingImeta = [];
  let spoileredUrls = new Set();
  let trackAuthoredContent;

  function HarnessComposer() {
    ({ trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
      livePendingImeta: pendingImeta,
      setPendingImeta: (imeta) => {
        pendingImeta = imeta;
      },
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: (urls) => {
        spoileredUrls = urls;
      },
      spoileredAttachmentUrlsRef: {
        get current() {
          return spoileredUrls;
        },
      },
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  trackAuthoredContent("");
  editorContent = "";
  await handle.unmount();

  const remounted = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "");
  assert.equal(pendingImeta[0]?.url, IMG_A.url);
  assert.deepEqual([...spoileredUrls], [IMG_A.url]);
  assert.equal(loadDraftEntry(DRAFT_KEY)?.content, "");
  await remounted.unmount();
});

test("draft_lifecycle_clear_caption_preserves_queued_file_on_remount", async () => {
  const DRAFT_KEY = "chan-clear-caption-file";
  setupStore("pubkey-clear-caption-file");
  persistDraftEntry(DRAFT_KEY, "caption", DRAFT_KEY, [], []);
  const FILE_A = {
    file: new File(["report"], "report.pdf", { type: "application/pdf" }),
    id: 9,
    spoilered: true,
  };

  let editorContent = "";
  let queuedAttachments = [FILE_A];
  let trackAuthoredContent;
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    ({ trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      getQueuedAttachments: () => queuedAttachments,
      saveQueuedAttachmentsForDraft,
      clearQueuedAttachments: () => {
        queuedAttachments = [];
      },
      restoreQueuedAttachments: (attachments) => {
        queuedAttachments = attachments;
      },
      takeQueuedAttachmentsForDraft,
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  saveQueuedAttachmentsForDraft(DRAFT_KEY, [FILE_A]);
  const handle = await mountStrictMode(HarnessComposer);
  trackAuthoredContent("");
  editorContent = "";
  await handle.unmount();

  const remounted = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "");
  assert.equal(queuedAttachments[0]?.file.name, "report.pdf");
  assert.equal(queuedAttachments[0]?.spoilered, true);
  await remounted.unmount();
});

test("draft_lifecycle_clear_authority_is_scoped_to_relay_and_identity", async () => {
  const DRAFT_KEY = "shared-key";
  installFreshLocalStorage();
  clearAllDrafts();
  initDraftStore("pubkey-a", "wss://relay-a.example");
  persistDraftEntry(DRAFT_KEY, "workspace A", DRAFT_KEY, [], []);

  let editorContent = "";
  let trackAuthoredContent;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: DRAFT_KEY,
      channelId: DRAFT_KEY,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const workspaceA = await mountStrictMode(HarnessComposer);
  trackAuthoredContent("");
  editorContent = "";
  await workspaceA.unmount();

  initDraftStore("pubkey-b", "wss://relay-b.example");
  persistDraftEntry(DRAFT_KEY, "workspace B", DRAFT_KEY, [], []);
  const workspaceB = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "workspace B");
  await workspaceB.unmount();
  assert.equal(loadDraftEntry(DRAFT_KEY)?.content, "workspace B");

  initDraftStore("pubkey-a", "wss://relay-a.example");
  const workspaceARemount = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "", "workspace A stale text must stay cleared");
  await workspaceARemount.unmount();
});

test("draft_lifecycle_preserves_local_files_across_a_b_a_switch", async () => {
  setupStore("pubkey-switch-files");
  const FILE_A = {
    file: new File(["report"], "report.pdf", { type: "application/pdf" }),
    id: 7,
    spoilered: false,
  };
  let draftKey = "chan-a";
  let editorContent = "";
  let queuedAttachments = [];
  const spoileredRef = { current: new Set() };

  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      getQueuedAttachments: () => queuedAttachments,
      saveQueuedAttachmentsForDraft,
      clearQueuedAttachments: () => {
        queuedAttachments = [];
      },
      restoreQueuedAttachments: (attachments) => {
        queuedAttachments = attachments;
      },
      takeQueuedAttachmentsForDraft,
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    });
    return null;
  }

  saveQueuedAttachmentsForDraft("chan-a", [FILE_A]);
  const handle = await mountStrictMode(HarnessComposer);
  assert.equal(queuedAttachments[0]?.file.name, "report.pdf");

  draftKey = "chan-b";
  await handle.rerender();
  assert.deepEqual(queuedAttachments, [], "B must not inherit A's local files");

  draftKey = "chan-a";
  await handle.rerender();
  assert.equal(
    queuedAttachments[0]?.file.name,
    "report.pdf",
    "A's attachment-only draft survives a full A → B → A switch",
  );

  await handle.unmount();
});

test("discarding_a_draft_drops_its_retained_local_files", () => {
  const retainedFile = {
    file: new File(["private"], "private.pdf", {
      type: "application/pdf",
    }),
    id: 8,
    spoilered: false,
  };

  saveQueuedAttachmentsForDraft("chan-deleted", [retainedFile]);
  deleteDraftEntry("chan-deleted");

  assert.deepEqual(takeQueuedAttachmentsForDraft("chan-deleted"), []);
});

const AGENT_PREFILL_REFS = [
  { displayName: "Jitter", pubkey: "agent-jitter", isAgent: true },
];

test("strictmode_lifecycle_restores_and_repersists_agent_prefill_kind", async () => {
  const draftKey = "chan-prefill-strictmode";
  setupStore("pubkey-prefill-strictmode");
  persistDraftEntry(
    draftKey,
    "@Jitter ",
    draftKey,
    [],
    [],
    AGENT_PREFILL_REFS,
    "agent-prefill",
  );

  let editorContent = "";
  let activeRefs = [];
  let getEntryKind;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ getEntryKind } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => activeRefs,
      restoreMentionRefs: (refs) => {
        activeRefs = [...refs];
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.equal(getEntryKind(), "agent-prefill");
  assert.equal(editorContent, "@Jitter ");
  await handle.unmount();
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "agent-prefill");
});

test("automatic_prefill_persists_immediately_without_waiting_for_cleanup", async () => {
  const draftKey = "chan-prefill-immediate";
  setupStore("pubkey-prefill-immediate");

  let persistAgentPrefill;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ persistAgentPrefill } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => AGENT_PREFILL_REFS,
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: () => {},
      clearContent: () => {},
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => "@Jitter ",
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  persistAgentPrefill("@Jitter ");
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "agent-prefill");
  assert.equal(loadDraftEntry(draftKey)?.content, "@Jitter ");
  await handle.unmount();
});

test("repersisting_an_identical_prefill_snapshot_is_a_storage_noop", async () => {
  const draftKey = "chan-prefill-noop";
  setupStore("pubkey-prefill-noop");
  const GROWN_PREFILL_REFS = [
    ...AGENT_PREFILL_REFS,
    { displayName: "Scout", pubkey: "agent-scout", isAgent: true },
  ];

  let persistAgentPrefill;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ persistAgentPrefill } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        content.includes("@Scout") ? GROWN_PREFILL_REFS : AGENT_PREFILL_REFS,
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: () => {},
      clearContent: () => {},
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => "@Jitter ",
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  persistAgentPrefill("@Jitter ");
  const first = loadDraftEntry(draftKey);
  assert.equal(first?.entryKind, "agent-prefill");

  // ISO timestamps have millisecond precision — space the writes out so an
  // updatedAt bump from a non-skipped duplicate persist is observable.
  await new Promise((resolve) => setTimeout(resolve, 10));
  let notifications = 0;
  const unsubscribe = subscribeToStore(() => {
    notifications += 1;
  });
  persistAgentPrefill("@Jitter ");
  persistAgentPrefill("@Jitter ");
  assert.equal(
    notifications,
    0,
    "re-persisting an identical prefill snapshot must not notify subscribers",
  );
  assert.equal(
    loadDraftEntry(draftKey)?.updatedAt,
    first.updatedAt,
    "re-persisting an identical prefill snapshot must not bump updatedAt",
  );

  // A genuinely changed snapshot (audience grew) must still write and notify.
  persistAgentPrefill("@Jitter @Scout ");
  unsubscribe();
  assert.equal(notifications, 1, "a changed prefill snapshot must persist");
  const grown = loadDraftEntry(draftKey);
  assert.equal(grown?.content, "@Jitter @Scout ");
  assert.equal(grown?.entryKind, "agent-prefill");
  assert.notEqual(grown?.updatedAt, first.updatedAt);
  await handle.unmount();
});

test("authored_edit_promotes_prefill_monotonically_even_if_text_returns", async () => {
  const draftKey = "chan-prefill-authored";
  setupStore("pubkey-prefill-authored");
  persistDraftEntry(
    draftKey,
    "@Jitter ",
    draftKey,
    [],
    [],
    AGENT_PREFILL_REFS,
    "agent-prefill",
  );

  let editorContent = "";
  let activeRefs = [];
  let getEntryKind;
  let trackAuthoredContent;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ getEntryKind, trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => activeRefs,
      restoreMentionRefs: (refs) => {
        activeRefs = [...refs];
      },
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  editorContent = "@Jitter hello";
  trackAuthoredContent(editorContent);
  assert.equal(getEntryKind(), "draft");
  editorContent = "@Jitter ";
  trackAuthoredContent(editorContent);
  assert.equal(getEntryKind(), "draft");
  await handle.unmount();
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "draft");
});

test("retained_attachment_promotes_an_agent_prefill_to_draft", async () => {
  const draftKey = "chan-prefill-media";
  setupStore("pubkey-prefill-media");
  persistDraftEntry(
    draftKey,
    "@Jitter ",
    draftKey,
    [],
    [],
    AGENT_PREFILL_REFS,
    "agent-prefill",
  );

  let editorContent = "";
  let activeRefs = [];
  let pendingImeta = [];
  let getEntryKind;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ getEntryKind } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: () => activeRefs,
      restoreMentionRefs: (refs) => {
        activeRefs = [...refs];
      },
      livePendingImeta: pendingImeta,
      setPendingImeta: (imeta) => {
        pendingImeta = imeta;
      },
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  pendingImeta = [IMG_A];
  await handle.rerender();
  assert.equal(getEntryKind(), "draft");
  await handle.unmount();
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "draft");
  assert.deepEqual(loadDraftEntry(draftKey)?.pendingImeta, [IMG_A]);
});

test("failed_send_restores_the_captured_entry_kind", async () => {
  const draftKey = "chan-send-kind-recovery";
  setupStore("pubkey-send-kind-recovery");
  persistDraftEntry(draftKey, "authored message", draftKey, [], []);

  let editorContent = "";
  let getEntryKind;
  let resetToAgentPrefill;
  let restoreEntryKind;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ getEntryKind, resetToAgentPrefill, restoreEntryKind } =
      useDraftPersistLifecycle({
        effectiveDraftKey: draftKey,
        channelId: draftKey,
        loadDraft: loadDraftEntry,
        persistDraft: persistDraftEntry,
        getMentionRefs: () => [],
        restoreMentionRefs: () => {},
        livePendingImeta: [],
        setPendingImeta: () => {},
        setContent: (content) => {
          editorContent = content;
        },
        clearContent: () => {
          editorContent = "";
        },
        setSpoileredAttachmentUrls: () => {},
        spoileredAttachmentUrlsRef: spoileredRef,
        syncComposerContentFromEditor: () => editorContent,
      }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  const submittedKind = getEntryKind();
  editorContent = "@Jitter ";
  resetToAgentPrefill();
  assert.equal(getEntryKind(), "agent-prefill");

  editorContent = "authored message";
  restoreEntryKind(submittedKind);
  await handle.unmount();
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "draft");
  assert.equal(loadDraftEntry(draftKey)?.content, "authored message");
});

test("unmount_cleanup_keeps_a_pure_prefill_durable_with_a_real_prefix_getter", async () => {
  const draftKey = "chan-prefill-unmount-strip";
  setupStore("pubkey-prefill-unmount-strip");
  persistDraftEntry(
    draftKey,
    "@Jitter ",
    draftKey,
    [],
    [],
    AGENT_PREFILL_REFS,
    "agent-prefill",
  );

  let editorContent = "";
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        content.includes("@Jitter") ? AGENT_PREFILL_REFS : [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
      // The production wiring supplies the implicit-prefix getter, so the
      // cleanup's persistedContent() would strip a pure prefill to "" — and an
      // empty persist clears the stored record. In-app navigation unmounts the
      // composer through exactly this path.
      getImplicitAgentMentionPrefix: () => "@Jitter ",
    });
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  assert.equal(editorContent, "@Jitter ");
  await handle.unmount();

  assert.equal(
    loadDraftEntry(draftKey)?.entryKind,
    "agent-prefill",
    "the stored agent-prefill classification must survive composer unmount",
  );
  assert.equal(loadDraftEntry(draftKey)?.content, "@Jitter ");
  assert.equal(
    getInboxDraftEntries().some(({ key }) => key === draftKey),
    false,
    "the surviving prefill record must stay excluded from Inbox drafts",
  );
});

test("unmount_cleanup_still_strips_the_implicit_prefix_from_authored_drafts", async () => {
  const draftKey = "chan-authored-unmount-strip";
  setupStore("pubkey-authored-unmount-strip");
  persistDraftEntry(
    draftKey,
    "@Jitter ",
    draftKey,
    [],
    [],
    AGENT_PREFILL_REFS,
    "agent-prefill",
  );

  let editorContent = "";
  let trackAuthoredContent;
  const spoileredRef = { current: new Set() };
  function HarnessComposer() {
    ({ trackAuthoredContent } = useDraftPersistLifecycle({
      effectiveDraftKey: draftKey,
      channelId: draftKey,
      loadDraft: loadDraftEntry,
      persistDraft: persistDraftEntry,
      getMentionRefs: (content) =>
        content.includes("@Jitter") ? AGENT_PREFILL_REFS : [],
      restoreMentionRefs: () => {},
      livePendingImeta: [],
      setPendingImeta: () => {},
      setContent: (content) => {
        editorContent = content;
      },
      clearContent: () => {
        editorContent = "";
      },
      setSpoileredAttachmentUrls: () => {},
      spoileredAttachmentUrlsRef: spoileredRef,
      syncComposerContentFromEditor: () => editorContent,
      getImplicitAgentMentionPrefix: () => "@Jitter ",
    }));
    return null;
  }

  const handle = await mountStrictMode(HarnessComposer);
  editorContent = "@Jitter hello";
  trackAuthoredContent(editorContent);
  await handle.unmount();

  assert.equal(
    loadDraftEntry(draftKey)?.content,
    "hello",
    "authored drafts keep stripping the implicit prefix on unmount",
  );
  assert.equal(loadDraftEntry(draftKey)?.entryKind, "draft");
});

function mountAgentPrefillBridge(overrides = {}) {
  let bridge;
  const contentRef = { current: "@Jitter " };
  const persisted = [];
  const rendered = [];
  let entryKind = "agent-prefill";
  let resetCount = 0;
  function HarnessComposer() {
    bridge = useAgentPrefillDraftBridge({
      // The deferred restore only runs for agents the audience confirmed as
      // pinned, so the agent under test has to be in it.
      audiencePubkeys: ["agent-jitter"],
      channelId: "chan-prefill-bridge",
      contentRef,
      getEntryKind: () => entryKind,
      keepMentionedAgentsPinned: false,
      pendingImetaRef: { current: [] },
      persistAgentPrefill: (content) => persisted.push(content),
      queuedAttachmentsRef: { current: [] },
      resetToAgentPrefill: () => {
        entryKind = "agent-prefill";
        resetCount += 1;
      },
      setComposerContent: (content) => rendered.push(content),
      ...overrides,
    });
    return null;
  }
  return {
    HarnessComposer,
    contentRef,
    persisted,
    rendered,
    get bridge() {
      return bridge;
    },
    get entryKind() {
      return entryKind;
    },
    set entryKind(value) {
      entryKind = value;
    },
    get resetCount() {
      return resetCount;
    },
  };
}

test("agent_prefill_bridge_classifies_the_optimistic_clear", async () => {
  const harness = mountAgentPrefillBridge();
  const handle = await mountStrictMode(harness.HarnessComposer);
  harness.bridge.restoreAddressedAgentMentionsRef.current = () => "@Jitter ";

  let restored;
  await act(async () => {
    restored = harness.bridge.onAddressedAgentsComposerCleared([
      "agent-jitter",
    ]);
  });

  assert.equal(restored, "@Jitter ");
  assert.equal(harness.entryKind, "agent-prefill");
  assert.equal(harness.resetCount, 1);
  assert.deepEqual(harness.rendered, ["@Jitter "]);
  await handle.unmount();
});

test("agent_prefill_bridge_persists_the_post_send_prefill_immediately", async () => {
  const harness = mountAgentPrefillBridge();
  const handle = await mountStrictMode(harness.HarnessComposer);

  await act(async () => {
    harness.bridge.onAddressedAgentsSendSucceeded(["agent-jitter"], []);
  });

  assert.deepEqual(harness.persisted, ["@Jitter "]);
  await handle.unmount();
});

test("agent_prefill_bridge_never_persists_a_delayed_restore_over_authored_text", async () => {
  let scheduledFrame = null;
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
  const originalCancelAnimationFrame = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = (callback) => {
    scheduledFrame = callback;
    return 1;
  };
  globalThis.cancelAnimationFrame = () => {
    scheduledFrame = null;
  };
  const harness = mountAgentPrefillBridge({
    keepMentionedAgentsPinned: true,
  });
  const handle = await mountStrictMode(harness.HarnessComposer);
  harness.bridge.restoreAddressedAgentMentionsRef.current = () =>
    "@Jitter authored";

  await act(async () => {
    harness.bridge.onAddressedAgentsSendSucceeded(
      ["agent-jitter"],
      ["agent-jitter"],
    );
  });
  harness.entryKind = "draft";
  harness.contentRef.current = "authored";
  await act(async () => scheduledFrame());

  assert.equal(harness.contentRef.current, "@Jitter authored");
  assert.deepEqual(harness.persisted, ["@Jitter "]);
  await handle.unmount();
  globalThis.requestAnimationFrame = originalRequestAnimationFrame;
  globalThis.cancelAnimationFrame = originalCancelAnimationFrame;
});
