/**
 * Empty-canvas existence regression: an empty-string canvas is still a valid
 * persisted kind:40100 revision (restore can republish one). ChannelCanvas must
 * key existence — the rendered content block, the Edit-vs-Create label, and the
 * History toggle — off the presence of a revision id, not content truthiness.
 *
 * Mounts the shipping ChannelCanvas with a canvas whose content is "" but whose
 * event id is non-null, then asserts the action reads "Edit" and History stays
 * available.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// The real Markdown component pulls in the remark/rehype/emoji stack, which
// never releases its jsdom handles and hangs the node:test process. This test
// only exercises existence gating, so serve an inert stub.
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

  // A persisted-but-empty canvas: content is "" while the head event id is set.
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => {
      if (cmd === "get_canvas") {
        return { content: "", event_id: HEAD, updated_at: 1, author: HEAD };
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

async function settle(iterations = 6) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

test("empty-string canvas with a revision id still exists — Edit label and History remain", async () => {
  const queryClient = new QueryClient({
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

  const editButton = container.querySelector(
    "[data-testid='channel-canvas-edit']",
  );
  assert.ok(editButton, "edit button renders for an existing empty canvas");
  assert.equal(
    editButton.textContent.trim(),
    "Edit canvas",
    "an existing empty canvas labels the action Edit, not Create",
  );
  assert.ok(
    container.querySelector("[data-testid='channel-canvas-history-toggle']"),
    "History remains available for an existing empty canvas",
  );

  await settle(12);
  await act(async () => root.unmount());
  queryClient.clear();
  container.remove();
});
