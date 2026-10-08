/**
 * Rejected-save reset regression: after a save is rejected, `ChannelCanvas`
 * must clear the shared set-canvas mutation before the next edit session so a
 * stale error alert does not reappear in the fresh editor.
 *
 * `useSetCanvasMutation` (TanStack Query) retains the mutation `error` across
 * edit sessions, and the editor renders it whenever it opens. Without
 * `setCanvasMutation.reset()` in `handleStartEditing`, the sequence
 * reject -> Cancel -> Edit re-surfaces the prior error before the user acts.
 *
 * Mounts the shipping ChannelCanvas, drives a rejected save (asserting the
 * error alert renders), cancels, reopens the editor, and asserts the alert is
 * gone. `CanvasHistoryPanel` mirrors this via `restoreMutation.reset()`.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// The real Markdown component pulls in the remark/rehype/emoji stack, which
// never releases its jsdom handles and hangs the node:test process. This test
// only exercises the mutation-reset wiring, so serve an inert stub.
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

const HEAD_A = "a".repeat(64);
const HEAD_B = "b".repeat(64);
const REJECTION_MESSAGE = "stale revision — reload and retry";

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let ChannelNavigationProvider;
let ChannelCanvas;

// The bridge rejects the first save, then accepts the second.
let rejectNextSave = true;
const setCanvasCalls = [];

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
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

  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === "get_canvas") {
        return {
          content: "original",
          event_id: HEAD_A,
          updated_at: 1,
          author: HEAD_A,
        };
      }
      if (cmd === "set_canvas") {
        setCanvasCalls.push(args);
        if (rejectNextSave) {
          throw new Error(REJECTION_MESSAGE);
        }
        return { ok: true, event_id: HEAD_B, verified: true };
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
  ({ ChannelCanvas } = await import("./ChannelCanvas.tsx"));
});

after(() => dom.window.close());

function click(element) {
  element.dispatchEvent(
    new dom.window.MouseEvent("click", { bubbles: true, cancelable: true }),
  );
}

// Flush microtasks, pending query promises, and React's scheduler so nothing
// is left pending at teardown.
async function settle(iterations = 6) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

function saveErrorAlert(container) {
  return [...container.querySelectorAll("[role='alert']")].find((node) =>
    node.textContent?.includes(REJECTION_MESSAGE),
  );
}

test("rejected save error does not reappear when reopening the editor", async () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0, retry: false },
    },
  });
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
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
    await queryClient.refetchQueries({ queryKey: ["channel-canvas"] });
  });
  await settle();

  // Open the editor and drive a rejected save.
  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-edit']")),
  );
  assert.ok(container.querySelector("[data-testid='channel-canvas-editor']"));
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-save']"));
  });
  await settle();

  // The editor stays open on rejection and surfaces the error.
  const editorOpenAfterReject = Boolean(
    container.querySelector("[data-testid='channel-canvas-editor']"),
  );
  const errorRenderedAfterReject = Boolean(saveErrorAlert(container));

  // Cancel back to the read view, then reopen the editor.
  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-cancel']")),
  );
  await settle();
  rejectNextSave = false;
  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-edit']")),
  );
  await settle();

  const editorReopened = Boolean(
    container.querySelector("[data-testid='channel-canvas-editor']"),
  );
  const staleErrorPresent = Boolean(saveErrorAlert(container));

  // Tear down before asserting so a failure reports cleanly instead of hanging
  // the node:test process on still-mounted React/query handles.
  await settle(12);
  await act(async () => root.unmount());
  queryClient.clear();
  container.remove();

  assert.ok(editorOpenAfterReject, "editor stays open after a rejected save");
  assert.ok(
    errorRenderedAfterReject,
    "the rejected-save error alert renders in the editor",
  );
  assert.ok(editorReopened, "editor reopens on the second edit session");
  assert.equal(
    staleErrorPresent,
    false,
    "the stale rejected-save error is cleared in the fresh edit session",
  );
});
