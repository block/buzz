/**
 * Restore-confirmation regression: restore rewrites the shared channel canvas
 * for everyone, so activating "Restore this revision" must NOT mutate on its
 * own — it opens a confirmation dialog identifying the target revision.
 * Only confirming publishes the restore.
 *
 * Mounts the shipping CanvasHistoryPanel, expands an older revision, activates
 * Restore, and asserts no set_canvas call fired and the confirm dialog is
 * visible; then confirms and asserts exactly one set_canvas call fired.
 *
 * Mutation-killable: wiring the Restore button back to call handleRestore
 * directly (the pre-fix behavior) makes the "no mutation before confirm"
 * assertion fail — set_canvas fires on the first click.
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

// Records every set_canvas invocation so the test can assert a restore
// mutated only after confirmation.
let setCanvasCalls = 0;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
    self: dom.window,
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
        setCanvasCalls += 1;
        return { ok: true, event_id: "e".repeat(64), verified: true };
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

test("restore requires explicit confirmation before mutating the shared canvas", async () => {
  setCanvasCalls = 0;
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

  // Activate Restore — must open the confirm dialog, NOT mutate.
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-restore']"));
  });
  await settle();

  let observed;
  try {
    // The dialog renders through a Radix portal into document.body, not the
    // mounted container, so query the whole document.
    const confirmVisibleBeforeConfirm =
      dom.window.document.querySelector(
        "[data-testid='channel-canvas-restore-confirm-action']",
      ) !== null;
    const callsBeforeConfirm = setCanvasCalls;

    // Confirm — this is the only path that mutates.
    await act(async () => {
      click(
        dom.window.document.querySelector(
          "[data-testid='channel-canvas-restore-confirm-action']",
        ),
      );
    });
    await settle();

    observed = {
      confirmVisibleBeforeConfirm,
      callsBeforeConfirm,
      callsAfterConfirm: setCanvasCalls,
    };
  } finally {
    await settle();
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }

  assert.ok(
    observed.confirmVisibleBeforeConfirm,
    "activating Restore opens the confirmation dialog",
  );
  assert.equal(
    observed.callsBeforeConfirm,
    0,
    "no set_canvas call fires before the user confirms",
  );
  assert.equal(
    observed.callsAfterConfirm,
    1,
    "confirming publishes exactly one restore",
  );
});
