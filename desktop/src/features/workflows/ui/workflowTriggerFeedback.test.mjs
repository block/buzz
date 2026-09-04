import assert from "node:assert/strict";
import { after, test } from "node:test";
import { JSDOM } from "jsdom";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  verifyEvent,
} from "nostr-tools";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  DocumentFragment: dom.window.DocumentFragment,
  CustomEvent: dom.window.CustomEvent,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
window.matchMedia = () => ({
  matches: false,
  addListener() {},
  removeListener() {},
  addEventListener() {},
  removeEventListener() {},
});
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
window.HTMLElement.prototype.scrollIntoView = () => {};
window.HTMLElement.prototype.hasPointerCapture = () => false;
window.HTMLElement.prototype.setPointerCapture = () => {};
window.HTMLElement.prototype.releasePointerCapture = () => {};
after(() => dom.window.close());

// Actual card + detail hook + IPC adapters. Unlike the former invocation-count
// assertion, this peer signs and commits an event, loses the response, then
// verifies the exact retry bytes and returns the same persisted run identity.
test("card/detail share pending, accessible error/retry and confirmed-only success animation", async () => {
  const keys = generateSecretKey();
  const pubkey = getPublicKey(keys);
  localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "a",
        name: "A",
        relayUrl: "wss://a.example",
        pubkey,
        addedAt: "2026-01-01",
      },
    ]),
  );
  localStorage.setItem("buzz-active-community-id", "a");
  const workflowId = crypto.randomUUID();
  const workflow = {
    id: workflowId,
    revision: "ab".repeat(32),
    owner_pubkey: pubkey,
    channel_id: null,
    name: "Test run",
    status: "active",
    created_at: 1,
    updated_at: 1,
    definition: {
      name: "Test run",
      enabled: true,
      trigger: { on: "message_posted" },
      steps: [{ id: "send", action: "send_message", content: "hello" }],
    },
  };
  let rejectPost;
  let resolvePost;
  let prepares = 0;
  const posts = [];
  const runs = new Map();
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 1,
    invoke: async (command, args) => {
      if (command === "get_identity") return { pubkey, display_name: "Me" };
      if (command === "get_workflow") return workflow;
      if (command === "get_workflow_runs") return { runs: [], next: null };
      if (command === "get_run_approvals") return { approvals: [] };
      if (command === "prepare_workflow_trigger") {
        prepares++;
        assert.equal(args.expectedRelayUrl, "wss://a.example");
        assert.equal(args.expectedSignerPubkey, pubkey);
        return finalizeEvent(
          {
            kind: 46020,
            content: "",
            created_at: 42,
            tags: [
              ["d", workflowId],
              ["e", workflow.revision],
              ["request-id", crypto.randomUUID()],
            ],
          },
          keys,
        );
      }
      if (command === "trigger_workflow") {
        assert.ok(verifyEvent(args.event));
        posts.push(JSON.stringify(args.event));
        if (!runs.has(args.event.id))
          runs.set(args.event.id, "persisted-run-1");
        return new Promise((resolve, reject) => {
          rejectPost = () => reject(new Error("response lost after commit"));
          resolvePost = () =>
            resolve({
              run_id: runs.get(args.event.id),
              workflow_id: workflowId,
              status: "pending",
            });
        });
      }
      throw new Error(`unexpected command ${command}`);
    },
  };
  globalThis.__TAURI_INTERNALS__ = window.__TAURI_INTERNALS__;
  const React = await import("react");
  const { render, act, fireEvent, waitFor, within, cleanup } = await import(
    "@testing-library/react"
  );
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  );
  const { WorkflowCard } = await import("./WorkflowCard.tsx");
  const { WorkflowDetailPanel } = await import("./WorkflowDetailPanel.tsx");
  const { workflowTriggerOperations } = await import("../triggerOperations.ts");
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false, gcTime: 0 },
    },
  });
  const noop = () => {};
  const uiWorkflow = {
    ...workflow,
    ownerPubkey: pubkey,
    channelId: null,
    createdAt: 1,
    updatedAt: 1,
  };
  const rendered = render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(
        CommunitiesProvider,
        null,
        React.createElement(WorkflowCard, {
          workflow: uiWorkflow,
          onView: noop,
          onEdit: noop,
          onDelete: noop,
          onDuplicate: noop,
          onToggleEnabled: noop,
        }),
        React.createElement(WorkflowDetailPanel, { workflowId }),
      ),
    ),
  );
  try {
    const card = within(rendered.getByTestId(`workflow-card-${workflowId}`));
    const detail = within(rendered.getByTestId("workflow-detail-panel"));
    await waitFor(() =>
      assert.equal(
        detail.getByRole("button", { name: "Trigger", exact: true }).disabled,
        false,
      ),
    );
    const stack = card.getByTestId("workflow-card-action-stack");
    const originalTile = stack.firstChild;
    // Use the card menu's keyboard path (Radix opens on ArrowDown).
    fireEvent.keyDown(card.getByRole("button", { name: "Workflow actions" }), {
      key: "ArrowDown",
    });
    await waitFor(() =>
      assert.ok(
        rendered.getByRole("menuitem", { name: "Trigger", exact: true }),
      ),
    );
    fireEvent.click(
      rendered.getByRole("menuitem", { name: "Trigger", exact: true }),
    );
    await waitFor(() => assert.equal(posts.length, 1));
    assert.match(card.getByRole("status").textContent, /Triggering/);
    assert.equal(
      detail.getByRole("button", { name: "Triggering..." }).disabled,
      true,
    );
    assert.equal(
      stack.firstChild,
      originalTile,
      "no success animation before settlement",
    );
    // A second caller joins the same operation, never creates another event.
    let joined;
    await act(async () => {
      joined = workflowTriggerOperations
        .run(workflowId, {
          expectedRelayUrl: "wss://a.example",
          expectedSignerPubkey: pubkey,
        })
        .catch(() => {});
      rejectPost();
      await joined;
    });
    await waitFor(() =>
      assert.match(
        card.getByRole("alert").textContent,
        /response lost after commit/,
      ),
    );
    assert.match(detail.getByRole("alert").textContent, /same signed request/);
    assert.equal(
      stack.firstChild,
      originalTile,
      "failure must not animate success",
    );
    fireEvent.click(
      card.getByRole("button", { name: "Retry trigger", exact: true }),
    );
    await waitFor(() => assert.equal(posts.length, 2));
    assert.equal(posts[0], posts[1]);
    assert.equal(prepares, 1);
    assert.equal(runs.size, 1);
    await act(async () => {
      resolvePost();
    });
    await waitFor(() =>
      assert.match(
        card.getByRole("status").textContent,
        /Run created: persisted-run-1/,
      ),
    );
    assert.notEqual(
      stack.firstChild,
      originalTile,
      "confirmed success starts the action animation",
    );
    assert.equal(card.queryByRole("alert"), null);
  } finally {
    cleanup();
    client.clear();
  }
});
