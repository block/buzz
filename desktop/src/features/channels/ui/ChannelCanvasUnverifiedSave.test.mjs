/**
 * Unverified-save regression: when set_canvas returns `verified: false` (the
 * relay accepted the write but the post-write verification read failed), the
 * save is durable — ChannelCanvas must close the editor and show a
 * non-destructive informational note, not a conflict or error. A `verified:
 * true` save shows no such note.
 *
 * Mounts the shipping ChannelCanvas, edits an existing canvas, and drives
 * set_canvas to return `verified: false`, then asserts the editor closes and
 * the unverified note renders.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

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

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let ChannelNavigationProvider;
let ChannelCanvas;

// Controls the `verified` flag the mocked set_canvas returns.
let nextVerified = false;

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
    invoke: async (cmd) => {
      if (cmd === "get_canvas") {
        return { content: "hi", event_id: HEAD, updated_at: 1, author: HEAD };
      }
      if (cmd === "set_canvas") {
        return { ok: true, event_id: "e".repeat(64), verified: nextVerified };
      }
      if (cmd === "get_canvas_history") {
        return { revisions: [], next_cursor: null };
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

async function settle(iterations = 6) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

async function mount(queryClient) {
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
  return { container, root };
}

test("verified:false save closes the editor and shows the non-destructive note", async () => {
  nextVerified = false;
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
  const { container, root } = await mount(queryClient);

  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-edit']")),
  );
  assert.ok(container.querySelector("[data-testid='channel-canvas-editor']"));

  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-save']"));
  });
  await settle(12);

  assert.equal(
    container.querySelector("[data-testid='channel-canvas-editor']"),
    null,
    "editor closes after an accepted-but-unverified save",
  );
  assert.ok(
    container.querySelector("[data-testid='channel-canvas-unverified-notice']"),
    "the non-destructive unverified-save note renders",
  );

  await settle(12);
  await act(async () => root.unmount());
  queryClient.clear();
  container.remove();
});

test("verified:true save closes the editor with no unverified note", async () => {
  nextVerified = true;
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
  const { container, root } = await mount(queryClient);

  await act(async () =>
    click(container.querySelector("[data-testid='channel-canvas-edit']")),
  );
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-save']"));
  });
  await settle(12);

  assert.equal(
    container.querySelector("[data-testid='channel-canvas-editor']"),
    null,
    "editor closes after a verified save",
  );
  assert.equal(
    container.querySelector("[data-testid='channel-canvas-unverified-notice']"),
    null,
    "a verified save shows no unverified note",
  );

  await settle(12);
  await act(async () => root.unmount());
  queryClient.clear();
  container.remove();
});
