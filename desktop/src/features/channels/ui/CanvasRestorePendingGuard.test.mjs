/**
 * Pending-restore guard regression: row buttons must be disabled while a
 * restore is pending so clicking a different row cannot call
 * `restoreMutation.reset()` — which would unobserve the running mutation,
 * hide a subsequent rejection, and allow a second concurrent restore.
 *
 * This test verifies three invariants:
 *
 * 1. **Guard**: while IPC is deferred, row buttons carry `disabled`, preventing
 *    `reset()` from being called mid-flight.
 * 2. **Rejection visibility**: rejecting the IPC makes the error render under
 *    the originating row, and `set_canvas` was called exactly once.
 * 3. **No spurious second dispatch**: attempting another restore while the
 *    mutation is pending (before the rejection settles) does not fire a second
 *    `set_canvas` call — the row expand buttons also carry `disabled` while
 *    pending, preventing `reset()` from being called mid-flight.
 *
 * Revert-causality: removing `disabled={restoreMutation.isPending}` from the
 * row button un-disables the row during the pending phase. In that case the
 * test's second-row click would call `restoreMutation.reset()`, which wipes the
 * mutation state — the rejection lands on a cleared mutation and the error is
 * never rendered. The rejection-visible assertion then fails.
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
const OLDER_A = "b".repeat(64);
const OLDER_B = "c".repeat(64);

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let CanvasHistoryPanel;

// Controls the IPC: null means the test has not triggered set_canvas yet.
let _deferredResolve = null;
let deferredReject = null;
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
        return new Promise((resolve, reject) => {
          _deferredResolve = resolve;
          deferredReject = reject;
        });
      }
      if (cmd === "get_canvas_history") {
        return {
          revisions: [
            { event_id: HEAD, content: "hi", created_at: 3, author: HEAD },
            {
              event_id: OLDER_A,
              content: "older-a",
              created_at: 2,
              author: HEAD,
            },
            {
              event_id: OLDER_B,
              content: "older-b",
              created_at: 1,
              author: HEAD,
            },
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

async function settle(iterations = 8) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
  }
}

/**
 * Full pending-guard contract:
 *
 * 1. Row expand buttons are disabled while restoreMutation.isPending is true.
 * 2. Rejecting the IPC makes the error visible under the originating row;
 *    set_canvas remains at exactly 1 call (no reset() fired mid-flight).
 * 3. Clicking another row's expand button while pending does not fire a second
 *    IPC (disabled prevents reset() from being called mid-flight).
 */
test("pending-guard: row disabled, rejection visible, no second dispatch", async () => {
  setCanvasCalls = 0;
  _deferredResolve = null;
  deferredReject = null;

  const client = new QueryClient({
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

  // Expand OLDER_A (second item) to reveal its Restore button.
  const items = container.querySelectorAll(
    "[data-testid='channel-canvas-history-item'] button",
  );
  await act(async () => click(items[1]));
  await settle();

  // Click Restore → opens confirmation dialog.
  await act(async () => {
    click(container.querySelector("[data-testid='channel-canvas-restore']"));
  });
  await settle();

  // Confirm → dispatches the deferred set_canvas call (isPending becomes true).
  await act(async () => {
    click(
      dom.window.document.querySelector(
        "[data-testid='channel-canvas-restore-confirm-action']",
      ),
    );
  });
  // Allow one tick for the mutation to enter isPending state without resolving.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 2));
  });

  // ── Invariant 1: row buttons are disabled while pending ────────────────
  const rowButtons = container.querySelectorAll(
    "[data-testid='channel-canvas-history-item'] button",
  );
  const secondRowIsDisabled =
    rowButtons[rowButtons.length - 1]?.disabled === true;

  // ── Invariant 3: clicking OLDER_B's row while pending must not call reset()
  // (which would unobserve the running mutation and swallow the rejection).
  // The disabled attribute prevents the click handler from firing, so
  // set_canvas must still be 1 after this click.
  const lastRowButton = rowButtons[rowButtons.length - 1];
  if (lastRowButton) {
    await act(async () => click(lastRowButton));
    await settle();
  }
  const callsAfterSecondClick = setCanvasCalls;

  // ── Invariant 2: reject the IPC and assert error is visible ───────────
  const rejectionMessage = "test conflict error";
  await act(async () => {
    deferredReject?.(new Error(rejectionMessage));
    deferredReject = null;
  });
  await settle(12);

  // After rejection the error must render under the OLDER_A row (the
  // originating row — OLDER_A was selected when set_canvas was dispatched).
  const olderALi = items[1]?.closest("li");
  const errorEl = olderALi?.querySelector("[role='alert']") ?? null;
  const errorVisible = errorEl !== null;
  const errorText = errorEl?.textContent ?? "";

  const finalCallCount = setCanvasCalls;

  try {
    // Teardown: unmount before assertions so cleanup always runs.
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  } finally {
    // Assertions after teardown so the test body always cleans up.
    assert.ok(
      secondRowIsDisabled,
      "row buttons must be disabled while restoreMutation.isPending is true — " +
        "a disabled button prevents reset() from being called mid-flight, " +
        "which would unobserve the pending mutation and hide subsequent errors",
    );
    assert.equal(
      callsAfterSecondClick,
      1,
      "clicking another row while pending must not fire a second set_canvas " +
        "(reset() would unobserve the running mutation)",
    );
    assert.ok(
      errorVisible,
      "rejecting the IPC must render an error under the originating row",
    );
    assert.ok(
      errorText.includes(rejectionMessage),
      `error text must contain the rejection message; got: "${errorText}"`,
    );
    assert.equal(
      finalCallCount,
      1,
      "exactly one set_canvas call must have fired throughout the test",
    );
  }
});
