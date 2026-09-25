/**
 * Cache-invalidation regression: an accepted canvas write reported as
 * CANVAS_SUPERSEDED is a *post-write* rejection — the event is durable and in
 * history, but a concurrent write is now the visible head, so the UI tells the
 * user to reload and restore the retained revision. The current and history
 * caches must therefore refetch on that rejection, not only on success.
 *
 * `useSetCanvasMutation` invalidates `channel-canvas` + `channel-canvas-history`
 * in `onSettled`, so both the save path (ChannelCanvas) and the restore path
 * (CanvasHistoryPanel) refresh their stale data when the mutation rejects with
 * the supersession marker. Reverting `onSettled` to `onSuccess` (invalidating
 * only on a resolved mutation) drops these invalidations and turns this RED.
 *
 * The tests mount the shipping components with a mocked IPC that rejects
 * set_canvas with the frozen marker, and spy on invalidateQueries.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

import { installRadixDialogGlobals } from "./canvasDialogTestEnv.mjs";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/shared/ui/markdown") {
      return { shortCircuit: true, url: "buzz-canvas-stub:markdown" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-canvas-stub:markdown") {
      return {
        format: "module",
        shortCircuit: true,
        source: "export function Markdown() { return null; }\n",
      };
    }
    return nextLoad(url, context);
  },
});

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Byte-identical to CANVAS_SUPERSEDED in canvas.rs / the marker in
// canvasConflict.ts. The relay accepted the write; a concurrent head is now
// current, so set_canvas rejects with this after publishing.
const CANVAS_SUPERSEDED =
  "conflict: canvas save was superseded by a concurrent write";

const HEAD = "a".repeat(64);
const OLDER = "b".repeat(64);

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let ChannelNavigationProvider;
let CommunitiesProvider;
let ChannelCanvas;
let CanvasHistoryPanel;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  installRadixDialogGlobals(dom);

  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => {
      if (cmd === "get_canvas") {
        return { content: "hi", event_id: HEAD, updated_at: 1, author: HEAD };
      }
      if (cmd === "set_canvas") {
        // Accepted publish, then a stranger head is current: post-write
        // supersession rejects the mutation with the frozen marker.
        throw CANVAS_SUPERSEDED;
      }
      if (cmd === "get_canvas_history") {
        return {
          revisions: [
            { event_id: HEAD, content: "hi", created_at: 2, author: HEAD },
            { event_id: OLDER, content: "old", created_at: 1, author: HEAD },
          ],
          next_cursor: null,
        };
      }
      if (cmd === "get_users_batch") {
        return { profiles: {} };
      }
      throw new Error(`unexpected command: ${cmd}`);
    },
  };

  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ ChannelNavigationProvider } = await import(
    "@/shared/context/ChannelNavigationContext"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities"
  ));
  ({ ChannelCanvas } = await import("./ChannelCanvas.tsx"));
  ({ CanvasHistoryPanel } = await import("./CanvasHistoryPanel.tsx"));
});

after(() => dom.window.close());

function click(element) {
  element.dispatchEvent(
    new dom.window.MouseEvent("click", { bubbles: true, cancelable: true }),
  );
}

async function settle(iterations = 12) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

// A QueryClient that records the keys passed to invalidateQueries so a test can
// assert refetch of both canvas keys after a mutation settles.
function makeSpyingClient() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
  const invalidated = [];
  const original = client.invalidateQueries.bind(client);
  client.invalidateQueries = (filters, ...rest) => {
    const key = filters?.queryKey;
    if (Array.isArray(key)) {
      invalidated.push(key[0]);
    }
    return original(filters, ...rest);
  };
  return { client, invalidated };
}

function assertBothKeysInvalidated(invalidated, context) {
  assert.ok(
    invalidated.includes("channel-canvas"),
    `${context}: channel-canvas cache must invalidate on supersession`,
  );
  assert.ok(
    invalidated.includes("channel-canvas-history"),
    `${context}: channel-canvas-history cache must invalidate on supersession`,
  );
}

test("save path: supersession rejection invalidates both canvas caches", async () => {
  const { client, invalidated } = makeSpyingClient();
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          ChannelNavigationProvider,
          { channels: [] },
          React.createElement(ChannelCanvas, {
            channelId: "channel-1",
            canEdit: true,
            isArchived: false,
          }),
        ),
      ),
    );
  });
  await act(async () => {
    await client.refetchQueries({ queryKey: ["channel-canvas"] });
  });
  await settle();

  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-edit']")),
  );
  invalidated.length = 0; // Ignore the get_canvas fetch settling.
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-save']"));
  });
  await settle();

  assertBothKeysInvalidated(invalidated, "save");

  await act(async () => root.unmount());
  client.clear();
  container.remove();
});

test("restore path: supersession rejection invalidates both canvas caches", async () => {
  const { client, invalidated } = makeSpyingClient();
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(CanvasHistoryPanel, {
            channelId: "channel-1",
            currentContent: "hi",
            currentRevision: HEAD,
            canRestore: true,
          }),
        ),
      ),
    );
  });
  await settle();

  // Expand the older (non-current) revision to reveal its Restore action.
  const items = container.querySelectorAll(
    "[data-testid='channel-canvas-history-item'] button",
  );
  await act(async () => click(items[items.length - 1]));
  await settle();

  invalidated.length = 0; // Ignore history/profile fetches settling.
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-restore']"));
  });
  await settle();

  // Restore opens a confirmation dialog before mutating the shared canvas;
  // confirm to fire the (rejecting) set_canvas. The dialog portals into
  // document.body.
  await act(async () => {
    click(
      dom.window.document.querySelector(
        "[data-testid='channel-canvas-restore-confirm-action']",
      ),
    );
  });
  await settle();

  assertBothKeysInvalidated(invalidated, "restore");

  await act(async () => root.unmount());
  client.clear();
  container.remove();
});
