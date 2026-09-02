/**
 * Edit-session snapshot regression: ChannelCanvas must assert the save against
 * the head that existed when editing started, not the live head. A background
 * canvas refetch can move the head mid-edit; without the snapshot the save
 * would silently overwrite the newer revision instead of surfacing a conflict.
 *
 * Mounts the shipping ChannelCanvas, opens the editor at head A, moves the live
 * head to B via a refetch, then saves and asserts the submitted
 * `expectedRevision` is still A.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// The real Markdown component pulls in the remark/rehype/emoji stack, which
// never releases its jsdom handles and hangs the node:test process. This test
// only exercises the save-snapshot wiring, so serve an inert stub.
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

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let ChannelNavigationProvider;
let ChannelCanvas;

// Mutable relay-head mock the Tauri bridge reads on each get_canvas.
let currentHead = { content: "original", eventId: HEAD_A };
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

  // Stub the Tauri IPC bridge invokeTauri ultimately calls.
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === "get_canvas") {
        return {
          content: currentHead.content,
          event_id: currentHead.eventId,
          updated_at: 1,
          author: HEAD_A,
        };
      }
      if (cmd === "set_canvas") {
        setCanvasCalls.push(args);
        return { ok: true, event_id: HEAD_B };
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

// Flush microtasks, pending query promises, and React's scheduler (the
// deferred canvas render is posted on a MessageChannel) so nothing is left
// pending at teardown.
async function settle(iterations = 6) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

test("head moves mid-edit — save still asserts the head snapshotted at edit-start", async () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      // gcTime: 0 lets the settled mutation drop from cache immediately so no
      // mutation promise is left pending when the node:test process tears down.
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

  // Canvas at head A has loaded; open the editor.
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: ["channel-canvas"] });
  });
  await settle();
  const editButton = container.querySelector(
    "[data-testid='channel-canvas-edit']",
  );
  assert.ok(editButton, "edit button renders after head A loads");
  await act(async () => click(editButton));
  assert.ok(container.querySelector("[data-testid='channel-canvas-editor']"));

  // Head moves to B under the open editor via a background refetch.
  currentHead = { content: "moved", eventId: HEAD_B };
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: ["channel-canvas"] });
  });
  await settle();

  // Save — the submitted expected revision must be the snapshot (A), not B.
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-save']"));
  });

  assert.equal(setCanvasCalls.length, 1);
  assert.equal(setCanvasCalls[0].expectedRevision, HEAD_A);

  // Drain the refetch the save's onSuccess invalidation triggers, plus any
  // deferred render still scheduled, so no work is pending at teardown.
  await settle(12);
  await act(async () => root.unmount());
  queryClient.clear();
  container.remove();
});
