/**
 * Accessibility regression for canvas mutation outcomes (WCAG 2.1 AA, per
 * VISION.md). Informational states (loading, the unverified notice) expose
 * `role="status"` and error states expose `role="alert"` so assistive tech
 * announces them; and after a save/restore removes the focused control, focus
 * lands on a sensible destination (the unverified notice) instead of falling
 * back to the document body.
 *
 * Mounts the shipping ChannelCanvas and CanvasHistoryPanel through a mocked
 * IPC. Dropping the roles or the focus restoration turns these assertions RED.
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
        // Accepted but unverified: exercises the notice + focus destination.
        return { ok: true, event_id: "e".repeat(64), verified: false };
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

function makeClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
}

test("save: unverified notice is a status live region and receives focus", async () => {
  const client = makeClient();
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  let observed;
  try {
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
    await act(async () => {
      click(container.querySelector("[data-testid='channel-canvas-save']"));
    });
    await settle();

    // Snapshot before teardown so a failing run reports cleanly rather than
    // leaving a mounted tree that stalls the process.
    const notice = container.querySelector(
      "[data-testid='channel-canvas-unverified-notice']",
    );
    observed = {
      hasNotice: notice !== null,
      role: notice?.getAttribute("role"),
      focused: dom.window.document.activeElement === notice,
    };
  } finally {
    await settle();
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }

  assert.ok(observed.hasNotice, "the unverified notice renders");
  assert.equal(observed.role, "status", "notice is a status region");
  assert.ok(
    observed.focused,
    "focus lands on the notice after the editor's Save button unmounts",
  );
});

test("restore: unverified notice is a status live region and receives focus", async () => {
  const client = makeClient();
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  let observed;
  try {
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

    const items = container.querySelectorAll(
      "[data-testid='channel-canvas-history-item'] button",
    );
    await act(async () => click(items[items.length - 1]));
    await settle();

    await act(async () => {
      click(container.querySelector("[data-testid='channel-canvas-restore']"));
    });
    await settle();

    // Restore opens a confirmation dialog before mutating the shared canvas;
    // confirm to publish. The dialog portals into document.body.
    await act(async () => {
      click(
        dom.window.document.querySelector(
          "[data-testid='channel-canvas-restore-confirm-action']",
        ),
      );
    });
    await settle();

    const notice = container.querySelector(
      "[data-testid='channel-canvas-restore-unverified-notice']",
    );
    observed = {
      hasNotice: notice !== null,
      role: notice?.getAttribute("role"),
      focused: dom.window.document.activeElement === notice,
    };
  } finally {
    await settle();
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }

  assert.ok(observed.hasNotice, "the unverified restore notice renders");
  assert.equal(observed.role, "status", "notice is a status region");
  assert.ok(
    observed.focused,
    "focus lands on the notice after the Restore button unmounts",
  );
});
