/**
 * Workflow approval actions must cross the production React Query -> Tauri
 * mutation seam exactly once. The relay remains the authorization authority;
 * Desktop renders rejection text and never treats a click as approval.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const calls = [];
const queryClients = [];
let invoke = async () => ({ event_id: "event-1" });

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  globalThis.__TAURI_INTERNALS__ = {
    invoke: (command, args) => {
      calls.push({ args, command });
      return invoke(command, args);
    },
  };
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const queryClient of queryClients.splice(0)) {
    queryClient.getQueryCache().clear();
    queryClient.getMutationCache().clear();
    queryClient.clear();
    queryClient.unmount();
  }
  calls.length = 0;
  invoke = async () => ({ event_id: "event-1" });
});

after(() => dom.window.close());

function approval(overrides = {}) {
  return {
    approvalRef: "ab".repeat(32),
    approverPubkey: null,
    approverSpec: "cd".repeat(32),
    createdAt: 1,
    expiresAt: "2999-01-01T00:00:00Z",
    note: null,
    runId: "run-1",
    status: "pending",
    stepId: "review",
    stepIndex: 1,
    workflowId: "workflow-1",
    ...overrides,
  };
}

async function renderCard(value = approval()) {
  const React = await import("react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { render } = await import("@testing-library/react");
  const { WorkflowApprovalCard } = await import("./WorkflowApprovalCard.tsx");
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0, retry: false },
      queries: { gcTime: 0, retry: false },
    },
  });
  queryClients.push(queryClient);
  const rendered = render(
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(WorkflowApprovalCard, { approval: value }),
    ),
  );
  return { ...rendered, queryClient };
}

test("Approve submits the opaque approval reference once and locks both actions", async () => {
  const { act, fireEvent, screen, waitFor } = await import(
    "@testing-library/react"
  );
  let resolve;
  invoke = () =>
    new Promise((promiseResolve) => {
      resolve = promiseResolve;
    });
  const { queryClient } = await renderCard();

  const approve = screen.getByTestId("workflow-approval-grant");
  const deny = screen.getByTestId("workflow-approval-deny");
  fireEvent.click(approve);
  fireEvent.click(approve);

  await waitFor(() => assert.equal(calls.length, 1));
  assert.deepEqual(calls[0], {
    args: { note: null, token: "ab".repeat(32) },
    command: "grant_approval",
  });
  assert.equal(approve.disabled, true);
  assert.equal(deny.disabled, true);
  await act(async () => {
    resolve({ event_id: "event-1" });
  });
  await waitFor(() =>
    assert.equal(
      queryClient.getMutationCache().getAll()[0]?.state.status,
      "success",
    ),
  );
});

test("relay authorization rejection is visible and permits an explicit retry", async () => {
  const { fireEvent, screen, waitFor } = await import("@testing-library/react");
  invoke = async () => {
    throw new Error("forbidden: not the designated approver");
  };
  await renderCard();

  fireEvent.click(screen.getByTestId("workflow-approval-deny"));
  await waitFor(() =>
    assert.match(
      screen.getByRole("alert").textContent,
      /not the designated approver/,
    ),
  );
  assert.equal(screen.getByTestId("workflow-approval-grant").disabled, false);
  fireEvent.click(screen.getByTestId("workflow-approval-deny"));
  await waitFor(() => assert.equal(calls.length, 2));
  assert.equal(calls[0].command, "deny_approval");
});

test("expired pending approvals stay visible without actionable controls", async () => {
  const { screen } = await import("@testing-library/react");
  await renderCard(approval({ expiresAt: "2000-01-01T00:00:00Z" }));

  assert.match(document.body.textContent, /approval request has expired/);
  assert.equal(screen.queryByTestId("workflow-approval-grant"), null);
  assert.equal(screen.queryByTestId("workflow-approval-deny"), null);
  assert.equal(calls.length, 0);
});

test("malformed expiry fails closed without actionable controls", async () => {
  const { screen } = await import("@testing-library/react");
  await renderCard(approval({ expiresAt: "not-a-date" }));

  assert.match(document.body.textContent, /approval request has expired/);
  assert.equal(screen.queryByTestId("workflow-approval-grant"), null);
  assert.equal(screen.queryByTestId("workflow-approval-deny"), null);
});
