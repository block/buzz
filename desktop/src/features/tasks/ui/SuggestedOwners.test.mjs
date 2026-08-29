/**
 * REG-16 cleaning debt: mounted consumer test for SuggestedOwners.tsx.
 *
 * ownerSuggestion.test.mjs covers the pure ranker; this covers the two things
 * only the render path can prove: the ranked order actually reaches the DOM
 * as `data-testid="suggested-owner"` buttons in rank order, clicking one
 * calls the `tasks_set_assignee` Tauri command with the clicked pubkey (not
 * some other candidate), and FM1 (empty candidate set renders nothing, not
 * an empty panel).
 */
import assert from "node:assert/strict";
import { after, afterEach, before, describe, it } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

let invokeCalls = [];
globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    invokeCalls.push({ command, args });
    if (command === "tasks_set_assignee") {
      return Promise.resolve({
        id: args.taskId,
        channelId: "chan-1",
        title: "irrelevant to this test",
        status: "open",
        assignee: args.assignee,
        createdBy: null,
        updatedAt: 2,
      });
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

let React, act, createRoot, QueryClient, QueryClientProvider, SuggestedOwners;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ SuggestedOwners } = await import("./SuggestedOwners.tsx"));
});

afterEach(() => {
  invokeCalls = [];
});

after(() => dom.window.close());

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

function task(overrides = {}) {
  return {
    id: "task-1",
    channelId: "chan-1",
    title: "Ship the relay migration",
    status: "open",
    assignee: null,
    createdBy: null,
    updatedAt: 1,
    ...overrides,
  };
}

function candidate(overrides = {}) {
  return {
    kind: "identity",
    displayName: "Someone",
    isAgent: false,
    isMember: true,
    pubkey: ALICE,
    ...overrides,
  };
}

function makeQueryClient() {
  return new QueryClient({
    // gcTime: 0 avoids react-query's post-success mutation GC timer (default
    // 5 minutes, not unref'd), which otherwise keeps `node --test` alive
    // long after the test itself completes.
    defaultOptions: {
      mutations: { gcTime: 0, retry: false },
      queries: { retry: false },
    },
  });
}

function renderSurface() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  return { container, root };
}

describe("SuggestedOwners — mounted consumer of ownerSuggestion + useSetChannelTaskAssignee", () => {
  it("renders ranked suggestions and clicking one calls tasks_set_assignee with the clicked pubkey", async () => {
    const queryClient = makeQueryClient();
    const { container, root } = renderSurface();

    // Bob is named in the task and Alice authored it (demoted), so Bob must
    // lead — this is the same K3 reordering ownerSuggestion.test.mjs proves,
    // now asserted through the actual rendered DOM rather than the pure fn.
    const candidates = [
      candidate({ displayName: "Alice", pubkey: ALICE, isMember: true }),
      candidate({ displayName: "Bob", pubkey: BOB, isMember: true }),
    ];

    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(SuggestedOwners, {
            candidates,
            task: task({ createdBy: ALICE, title: "Bob should ship this" }),
          }),
        ),
      );
    });

    const buttons = container.querySelectorAll(
      '[data-testid="suggested-owner"]',
    );
    assert.equal(buttons.length, 2, "both candidates must render");
    assert.equal(
      buttons[0].getAttribute("data-pubkey"),
      BOB,
      "the mentioned, non-author candidate must rank first",
    );
    assert.equal(buttons[1].getAttribute("data-pubkey"), ALICE);

    await act(async () => {
      buttons[0].dispatchEvent(
        new dom.window.MouseEvent("click", { bubbles: true }),
      );
    });
    // The mutation resolves through a dynamic import + async Tauri round trip
    // (several microtask hops). A fixed sleep here raced that chain and failed
    // 3/8 runs under load (invokeCalls still empty at 50ms). Poll for the call
    // to land instead: deterministic when it works, bounded so it cannot hang.
    await act(async () => {
      const deadline = Date.now() + 2000;
      while (invokeCalls.length === 0 && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 10));
      }
    });

    assert.equal(invokeCalls.length, 1, "clicking must issue exactly one call");
    assert.equal(invokeCalls[0].command, "tasks_set_assignee");
    assert.equal(invokeCalls[0].args.taskId, "task-1");
    assert.equal(
      invokeCalls[0].args.assignee,
      BOB,
      "must assign the CLICKED pubkey, not any other candidate",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("FM1: an empty candidate set renders nothing, not an empty panel", async () => {
    const queryClient = makeQueryClient();
    const { container, root } = renderSurface();

    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(SuggestedOwners, {
            candidates: [],
            task: task(),
          }),
        ),
      );
    });

    assert.equal(
      container.querySelector('[data-testid="suggested-owners"]'),
      null,
    );
    assert.equal(container.innerHTML, "");

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });
});
