/**
 * Channel-switch reset regression: ChannelManagementSheet renders
 * KeyedChannelCanvas, the production wrapper that owns
 * `key={channelId ?? "none"}`, so a channel change remounts the ChannelCanvas
 * subtree and drops all edit state (isEditing, draft, editBaseRevision,
 * showHistory, unverifiedSaveNotice, the set-canvas mutation instance). Without
 * the key the sheet stays mounted and a draft typed against canvas-less channel
 * A would publish as channel B's canvas under A's retained `none` precondition.
 *
 * Mounts the production KeyedChannelCanvas directly (the same seam the sheet
 * consumes): starts creating on canvas-less A, types a draft, switches to
 * canvas-less B, and asserts the editor is gone (state reset) and nothing was
 * submitted. Deleting the wrapper's key turns this RED.
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

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let ChannelNavigationProvider;
let KeyedChannelCanvas;

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

  // Both channels are canvas-less: get_canvas returns a null head everywhere.
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd === "get_canvas") {
        return { content: "", event_id: null, updated_at: null, author: null };
      }
      if (cmd === "set_canvas") {
        setCanvasCalls.push(args);
        return { ok: true, event_id: "e".repeat(64), verified: true };
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
  ({ KeyedChannelCanvas } = await import("./KeyedChannelCanvas.tsx"));
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

// Renders the production KeyedChannelCanvas wrapper, which owns the
// channelId-derived key — the exact seam ChannelManagementSheet consumes. A
// re-render with a new channelId remounts the ChannelCanvas subtree.
function Harness({ channelId }) {
  return React.createElement(KeyedChannelCanvas, {
    channelId,
    canEdit: true,
    isArchived: false,
  });
}

test("switching channels mid-create resets edit state and submits nothing", async () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
      mutations: { gcTime: 0 },
    },
  });
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);

  function render(channelId) {
    return act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(
            ChannelNavigationProvider,
            { channels: [] },
            React.createElement(Harness, { channelId }),
          ),
        ),
      );
    });
  }

  // Load canvas-less channel A and start creating.
  let observed;
  try {
    await render("channel-a");
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: ["channel-canvas"] });
    });
    await settle();
    const createButton = container.querySelector(
      "[data-testid='channel-canvas-edit']",
    );
    assert.ok(createButton, "create button renders for canvas-less channel A");
    await act(async () => click(createButton));
    const editor = container.querySelector(
      "[data-testid='channel-canvas-editor']",
    );
    assert.ok(editor, "editor opens on channel A");

    // Type a draft against A.
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        dom.window.HTMLTextAreaElement.prototype,
        "value",
      ).set;
      setter.call(editor, "draft written for channel A");
      editor.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    });
    await settle();

    // Switch to canvas-less channel B — the key change remounts ChannelCanvas.
    await render("channel-b");
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: ["channel-canvas"] });
    });
    await settle();

    // Snapshot what the switch produced before tearing down. Capturing here and
    // asserting after teardown keeps a failing run (e.g. the production key
    // deleted) from leaving a live editing tree mounted, which would hang the
    // process instead of reporting a clean failure.
    observed = {
      editorStillOpen: container.querySelector(
        "[data-testid='channel-canvas-editor']",
      ),
      editButtonLabel: container
        .querySelector("[data-testid='channel-canvas-edit']")
        ?.textContent.trim(),
      submitCount: setCanvasCalls.length,
    };
  } finally {
    // Always tear down so the process exits whether or not the assertions hold.
    await settle(12);
    await act(async () => root.unmount());
    queryClient.clear();
    container.remove();
  }

  // Edit state must not survive the switch: the editor is gone, B shows its own
  // fresh Create action, and A's draft never published against B.
  assert.equal(
    observed.editorStillOpen,
    null,
    "editor does not carry across the channel switch",
  );
  assert.equal(
    observed.editButtonLabel,
    "Create canvas",
    "channel B is treated as canvas-less, not carrying A's edit session",
  );
  assert.equal(
    observed.submitCount,
    0,
    "no canvas save fired across the channel switch",
  );
});
