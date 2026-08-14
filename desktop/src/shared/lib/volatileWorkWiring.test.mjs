/**
 * Mounted production-wiring tests for the volatile-work registry.
 *
 * ── What this proves ─────────────────────────────────────────────────────────
 * The registry itself is unit-tested in volatileWorkRegistry.test.mjs. This
 * file proves the OTHER half of the CRITICAL finding: that each real source
 * Thufir enumerated actually registers its reload-destroyed work, by mounting
 * the REAL production code (not a fixture with the blocker hand-set) and
 * asserting `isAnyVolatileWorkPending()` flips as the source's live state does.
 *
 * Covered sources, each empty → active → drained:
 *   - ForumComposer's unsent draft: full mount (real tiptap editor, router,
 *     providers); empty composer never blocks, real editor input blocks, and
 *     unmount releases (the forum view closing must not leak its key).
 *   - useMediaUpload, BOTH branches: the production composer takes the
 *     `deferUploadsUntilSend` branch (queued local videos), the non-deferred
 *     path is a foreground upload in flight. A mounted test on only one branch
 *     would "prove" coverage the other path never gets.
 *   - The native-picker choke points `pickAndUploadMedia` (tauri.ts) and
 *     `pickAndUploadImage` (tauriMedia.ts): the open-dialog + Rust-upload window
 *     blocks, and the `finally` releases on both resolve and reject. Testing at
 *     the choke point (not the caller) is what covers every picker caller —
 *     CustomEmojiSettingsCard and useSendFeedback have no uploadingCount.
 *   - backgroundMediaUploadStore's registered predicate: an off-channel
 *     retained local file (the in-memory `queuedAttachmentsByDraftKey` map) and
 *     an in-flight background task both block; draining either releases.
 *
 * ── DOM + process-exit note ──────────────────────────────────────────────────
 * The full ForumComposer mount pulls in tiptap/ProseMirror + TanStack Router,
 * whose deep tree leaves ref'd timers alive after unmount (visible only in
 * `process.getActiveResourcesInfo()`, not `_getActiveHandles()`). Left alone
 * they keep the event loop alive and hang `pnpm test`. We track every timer
 * created after setup and clear the survivors in `after()`, so this file exits
 * cleanly. node:test runs each file in its own process, so the timer shims and
 * the module-level store predicate cannot leak into other test files.
 */

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// Real timer fns captured before the tracking shims replace the globals.
const realSetTimeout = globalThis.setTimeout;
const realClearTimeout = globalThis.clearTimeout;
const realSetInterval = globalThis.setInterval;
const realClearInterval = globalThis.clearInterval;

// Every timer created after setup, cleared in after() to release the ref'd
// timers the tiptap/router tree leaves behind.
const liveTimeouts = new Set();
const liveIntervals = new Set();

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Swappable Tauri IPC handler; tests that drive the bridge replace it.
let invokeImpl = async () => undefined;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    DOMParser: dom.window.DOMParser,
    Node: dom.window.Node,
    Range: dom.window.Range,
    File: dom.window.File,
    Blob: dom.window.Blob,
    FileList: dom.window.FileList,
    localStorage: dom.window.localStorage,
    self: globalThis,
    scrollTo: () => {},
  });
  globalThis.setTimeout = (...args) => {
    const id = realSetTimeout(...args);
    liveTimeouts.add(id);
    return id;
  };
  globalThis.clearTimeout = (id) => {
    liveTimeouts.delete(id);
    return realClearTimeout(id);
  };
  globalThis.setInterval = (...args) => {
    const id = realSetInterval(...args);
    liveIntervals.add(id);
    return id;
  };
  globalThis.clearInterval = (id) => {
    liveIntervals.delete(id);
    return realClearInterval(id);
  };
  const raf = (fn) => globalThis.setTimeout(() => fn(Date.now()), 0);
  const caf = (id) => globalThis.clearTimeout(id);
  globalThis.requestAnimationFrame = raf;
  globalThis.cancelAnimationFrame = caf;
  dom.window.requestAnimationFrame = raf;
  dom.window.cancelAnimationFrame = caf;
  if (!globalThis.MutationObserver) {
    globalThis.MutationObserver = dom.window.MutationObserver;
  }
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  // jsdom's createObjectURL rejects a File; the composer only needs an opaque
  // handle, so stub it (and revoke) on both realms.
  globalThis.URL.createObjectURL = () => "blob:test";
  globalThis.URL.revokeObjectURL = () => {};
  dom.window.URL.createObjectURL = () => "blob:test";
  dom.window.URL.revokeObjectURL = () => {};
  const tauriInternals = {
    invoke: (cmd, args, opts) => invokeImpl(cmd, args, opts),
    transformCallback: () => 0,
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  dom.window.__TAURI_INTERNALS__ = tauriInternals;
  // The Tauri event plugin's _unlisten() reads this global on teardown; the
  // upload hooks register a byte-progress listener whose cleanup runs on
  // unmount, so without the stub every mounted-hook test throws there.
  const tauriEventPluginInternals = {
    unregisterListener: () => {},
  };
  globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = tauriEventPluginInternals;
  dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = tauriEventPluginInternals;
  // This jsdom build ships File/Blob without arrayBuffer(); uploadMediaFile
  // awaits it before the IPC, so polyfill it to reach the mocked invoke.
  if (!dom.window.Blob.prototype.arrayBuffer) {
    dom.window.Blob.prototype.arrayBuffer = async function arrayBuffer() {
      return new ArrayBuffer(this.size ?? 0);
    };
  }
});

after(() => {
  for (const id of liveTimeouts) realClearTimeout(id);
  for (const id of liveIntervals) realClearInterval(id);
  liveTimeouts.clear();
  liveIntervals.clear();
  dom.window.close();
});

// React core needs no DOM; react-dom is imported dynamically inside the mount
// helper so the timer shims installed in before() are active before its
// scheduler binds.
const React = await import("react");

/** Wait `ms` on the real (untracked) timer so the wait itself is never a
 * survivor we clear. */
function realDelay(ms) {
  return new Promise((resolve) => realSetTimeout(resolve, ms));
}

/** Mount a hook in a thin harness, exposing its latest return value. */
async function mountHook(useHook) {
  const { createRoot } = await import("react-dom/client");
  const { act } = await import("react");
  const ref = { current: null };
  function Harness() {
    ref.current = useHook();
    return null;
  }
  const container = dom.window.document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(Harness));
  });
  return {
    ref,
    act,
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

/** Mount the real ForumComposer inside the providers + router it needs. */
async function mountForumComposer() {
  const { createRoot } = await import("react-dom/client");
  const { act } = await import("react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { createRouter, createRootRoute, RouterProvider, createMemoryHistory } =
    await import("@tanstack/react-router");
  const { ForumComposer } = await import(
    "@/features/forum/ui/ForumComposer.tsx"
  );
  const { CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  );
  const { TooltipProvider } = await import("@/shared/ui/tooltip.tsx");

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.setQueryData(["identity"], { pubkey: "c".repeat(64) });
  const rootRoute = createRootRoute({
    component: () =>
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(ForumComposer, {
              placeholder: "Post to the forum",
              onSubmit: () => {},
            }),
          ),
        ),
      ),
  });
  const router = createRouter({
    routeTree: rootRoute,
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();

  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(RouterProvider, { router }));
  });
  await act(async () => {
    await realDelay(150);
  });
  return {
    container,
    act,
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

async function loadRegistry() {
  return import("@/shared/lib/volatileWorkRegistry.ts");
}

// ── ForumComposer unsent draft ───────────────────────────────────────────────

test("forum_composer_empty_draft_does_not_block_reload", async () => {
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  const view = await mountForumComposer();
  const editor = view.container.querySelector(
    '.ProseMirror, [contenteditable="true"]',
  );
  assert.ok(editor, "the real tiptap editor must mount");
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "an empty forum composer holds no reload-destroyed work",
  );
  await view.unmount();
});

test("forum_composer_unsent_text_blocks_then_unmount_releases", async () => {
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  const view = await mountForumComposer();
  const editor = view.container.querySelector(
    '.ProseMirror, [contenteditable="true"]',
  );
  assert.ok(editor, "the real tiptap editor must mount");

  // Type into the real editor; the content flows into ForumComposer's state,
  // which drives useRegisterVolatileWork.
  await view.act(async () => {
    editor.textContent = "an unsent forum reply";
    editor.dispatchEvent(new dom.window.InputEvent("input", { bubbles: true }));
  });
  await view.act(async () => {
    await realDelay(30);
  });
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "unsent forum text must block the idle reload",
  );

  // The forum view closing with unsent text must not leak its key.
  await view.unmount();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "unmounting the forum composer must release its pending key",
  );
});

// ── useMediaUpload — deferred (production composer) branch ────────────────────

test("media_upload_deferred_queued_video_blocks_then_clear_releases", async () => {
  const { useMediaUpload } = await import(
    "@/features/messages/lib/useMediaUpload.ts"
  );
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  const handle = await mountHook(() =>
    useMediaUpload({ deferUploadsUntilSend: true }),
  );
  // A video on the deferred branch is queued locally (never uploaded until
  // send) — reload-destroyed state with no in-flight upload behind it.
  const video = new dom.window.File([new Uint8Array(8)], "clip.mp4", {
    type: "video/mp4",
  });
  await handle.act(async () => {
    await handle.ref.current.uploadFile(video);
  });
  assert.equal(
    handle.ref.current.queuedAttachments.length,
    1,
    "the video must be queued locally, not uploaded",
  );
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "a queued local attachment must block the reload",
  );

  await handle.act(async () => {
    handle.ref.current.clearQueuedAttachments();
  });
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "clearing the queue drains the block",
  );
  await handle.unmount();
});

// ── useMediaUpload — non-deferred (foreground upload) branch ──────────────────

test("media_upload_foreground_in_flight_blocks_then_completion_releases", async () => {
  const { useMediaUpload } = await import(
    "@/features/messages/lib/useMediaUpload.ts"
  );
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  let resolveUpload;
  invokeImpl = (cmd) => {
    if (cmd === "upload_media_bytes_raw") {
      return new Promise((resolve) => {
        resolveUpload = () =>
          resolve({ url: "u", sha256: "s", size: 8, type: "image/png" });
      });
    }
    return undefined;
  };

  const handle = await mountHook(() =>
    useMediaUpload({ deferUploadsUntilSend: false }),
  );
  const image = new dom.window.File([new Uint8Array(8)], "pic.png", {
    type: "image/png",
  });
  await handle.act(async () => {
    handle.ref.current.uploadFile(image).catch(() => {});
    await realDelay(20);
  });
  assert.equal(
    handle.ref.current.isUploading,
    true,
    "the foreground upload must be in flight",
  );
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "an in-flight foreground upload must block the reload",
  );

  await handle.act(async () => {
    resolveUpload();
    await realDelay(20);
  });
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "a completed upload drains the block",
  );
  await handle.unmount();
  invokeImpl = async () => undefined;
});

test("media_upload_unmount_while_queued_releases", async () => {
  const { useMediaUpload } = await import(
    "@/features/messages/lib/useMediaUpload.ts"
  );
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  const handle = await mountHook(() =>
    useMediaUpload({ deferUploadsUntilSend: true }),
  );
  const video = new dom.window.File([new Uint8Array(8)], "clip.mp4", {
    type: "video/mp4",
  });
  await handle.act(async () => {
    await handle.ref.current.uploadFile(video);
  });
  assert.equal(isAnyVolatileWorkPending(), true, "queued video blocks");

  // A composer torn down with a queued attachment must not leak its key.
  await handle.unmount();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "unmounting a composer with a queued attachment must release its key",
  );
});

// ── Native-picker choke points ────────────────────────────────────────────────

test("pick_and_upload_media_open_dialog_blocks_then_release_on_resolve", async () => {
  const { pickAndUploadMedia } = await import("@/shared/api/tauri.ts");
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  let resolvePick;
  invokeImpl = (cmd) => {
    if (cmd === "pick_and_upload_media") {
      return new Promise((resolve) => {
        resolvePick = () => resolve([]);
      });
    }
    return undefined;
  };

  const pending = pickAndUploadMedia("progress-id");
  await realDelay(5);
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "the open picker + Rust upload window must block the reload",
  );
  resolvePick();
  await pending;
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "the finally must release when the picker resolves",
  );
  invokeImpl = async () => undefined;
});

test("pick_and_upload_media_release_on_reject", async () => {
  const { pickAndUploadMedia } = await import("@/shared/api/tauri.ts");
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  let rejectPick;
  invokeImpl = (cmd) => {
    if (cmd === "pick_and_upload_media") {
      return new Promise((_resolve, reject) => {
        rejectPick = () => reject(new Error("picker failed"));
      });
    }
    return undefined;
  };

  const pending = pickAndUploadMedia("progress-id");
  await realDelay(5);
  assert.equal(isAnyVolatileWorkPending(), true, "open picker blocks");
  rejectPick();
  await assert.rejects(pending);
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "the finally must release even when the picker rejects",
  );
  invokeImpl = async () => undefined;
});

test("pick_and_upload_image_open_dialog_blocks_then_release_on_resolve", async () => {
  const { pickAndUploadImage } = await import("@/shared/api/tauriMedia.ts");
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  let resolvePick;
  invokeImpl = (cmd) => {
    if (cmd === "pick_and_upload_image") {
      return new Promise((resolve) => {
        resolvePick = () => resolve(null);
      });
    }
    return undefined;
  };

  const pending = pickAndUploadImage();
  await realDelay(5);
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "the image picker choke point (feedback, no uploadingCount) must block",
  );
  resolvePick();
  await pending;
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "the finally must release when the image picker resolves",
  );
  invokeImpl = async () => undefined;
});

// ── backgroundMediaUploadStore registered predicate ───────────────────────────

test("background_store_retained_offchannel_file_blocks_then_taking_releases", async () => {
  const store = await import(
    "@/features/messages/lib/backgroundMediaUploadStore.ts"
  );
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  // A local file retained after the user leaves its channel lives only in the
  // in-memory queuedAttachmentsByDraftKey map — the clearest reload-loss case.
  const attachment = {
    file: new dom.window.File([new Uint8Array(4)], "note.pdf", {
      type: "application/pdf",
    }),
    id: 1,
    spoilered: false,
  };
  store.saveQueuedAttachmentsForDraft("chan-away", [attachment]);
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "an off-channel retained local file must block the reload",
  );

  const taken = store.takeQueuedAttachmentsForDraft("chan-away");
  assert.equal(taken.length, 1, "the retained file is returned when taken");
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "draining the retention map releases the block",
  );
});

test("background_store_in_flight_task_blocks_then_completion_releases", async () => {
  const store = await import(
    "@/features/messages/lib/backgroundMediaUploadStore.ts"
  );
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "precondition: registry idle",
  );

  let resolveUpload;
  invokeImpl = (cmd) => {
    if (cmd === "upload_media_bytes_raw") {
      return new Promise((resolve) => {
        resolveUpload = () =>
          resolve({ url: "u", sha256: "s", size: 4, type: "image/png" });
      });
    }
    return undefined;
  };

  let completed = false;
  const attachment = {
    file: new dom.window.File([new Uint8Array(4)], "pic.png", {
      type: "image/png",
    }),
    id: 2,
    spoilered: false,
  };
  store.enqueueBackgroundMediaUpload({
    attachments: [attachment],
    onComplete: async () => {
      completed = true;
    },
    onError: () => {},
  });
  await realDelay(20);
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "an in-flight background upload must block the reload",
  );

  resolveUpload();
  await realDelay(20);
  assert.equal(completed, true, "the background upload completes");
  assert.equal(
    isAnyVolatileWorkPending(),
    false,
    "the settled background task releases the block",
  );
  store.resetBackgroundMediaUploads();
  invokeImpl = async () => undefined;
});
