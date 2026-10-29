/**
 * Unverified-restore regression: when the restore's set_canvas call returns
 * `verified: false` (the relay accepted the write but the post-write
 * verification read failed), the restore is durable — CanvasHistoryPanel must
 * collapse the selection and show the same non-destructive informational note
 * as an unverified save, not treat it as a failure. A `verified: true` restore
 * shows no such note.
 *
 * Mounts the shipping CanvasHistoryPanel, expands an older revision, restores
 * it, and drives set_canvas to return `verified: false`, then asserts the
 * non-destructive restore note renders.
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
let CommunitiesProvider;
let CanvasHistoryPanel;

// Controls the `verified` flag the mocked set_canvas returns.
let nextVerified = false;

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
      if (cmd === "set_canvas") {
        return { ok: true, event_id: "e".repeat(64), verified: nextVerified };
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
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities"
  ));
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

async function mountAndRestore(nextVerifiedValue) {
  nextVerified = nextVerifiedValue;
  const client = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
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

  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-restore']"));
  });
  await settle();

  // Restore now opens a confirmation dialog (it rewrites the shared canvas);
  // confirm to publish. The dialog portals into document.body, not container.
  await act(async () => {
    click(
      dom.window.document.querySelector(
        "[data-testid='channel-canvas-restore-confirm-action']",
      ),
    );
  });
  await settle();

  return { client, container, root };
}

test("verified:false restore collapses selection and shows the non-destructive note", async () => {
  const { client, container, root } = await mountAndRestore(false);

  let observed;
  try {
    // Snapshot before teardown so a failing run reports cleanly rather than
    // leaving a mounted tree with a pending mutation that stalls the process.
    observed = {
      hasNotice:
        container.querySelector(
          "[data-testid='channel-canvas-restore-unverified-notice']",
        ) !== null,
      restoreGone:
        container.querySelector("[data-testid='channel-canvas-restore']") ===
        null,
    };
  } finally {
    await settle();
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }

  assert.ok(
    observed.hasNotice,
    "the non-destructive unverified-restore note renders",
  );
  assert.ok(
    observed.restoreGone,
    "the selection collapses after an accepted-but-unverified restore",
  );
});

test("verified:true restore shows no unverified note", async () => {
  const { client, container, root } = await mountAndRestore(true);

  let observed;
  try {
    observed = {
      hasNotice:
        container.querySelector(
          "[data-testid='channel-canvas-restore-unverified-notice']",
        ) !== null,
    };
  } finally {
    await settle();
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }

  assert.ok(!observed.hasNotice, "a verified restore shows no unverified note");
});
