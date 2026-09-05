import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
let waitFor,
  render,
  screen,
  cleanup,
  fireEvent,
  act,
  createElement,
  QueryClient,
  QueryClientProvider,
  AgentSyncWarning;
const clients = [];
let result = null;
let reads = 0;
before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      assert.equal(command, "get_managed_agent_sync_error");
      reads++;
      return result;
    },
  };
  ({ waitFor, render, screen, cleanup, fireEvent, act } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ AgentSyncWarning } = await import("./AgentSyncWarning.tsx"));
});
afterEach(() => {
  cleanup();
  for (const client of clients.splice(0)) client.clear();
});
after(() => dom.window.close());

test("persistent sync error renders without an agent row; reconnect invokes recovery and success clears it", async () => {
  result = "managed-agent history exceeds bootstrap page limit";
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  clients.push(client);
  let reconnects = 0;
  render(
    createElement(
      QueryClientProvider,
      { client },
      createElement(AgentSyncWarning, { onReconnect: () => reconnects++ }),
    ),
  );
  const alert = await screen.findByRole("alert");
  assert.match(alert.textContent, /startup changes have not been published/);
  assert.match(alert.textContent, /page limit/);
  assert.match(alert.textContent, /community operator/);
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["managed-agent-sync-error"] });
  });
  assert.ok(reads >= 2);
  assert.ok(screen.getByRole("alert"));
  fireEvent.click(screen.getByRole("button", { name: "Reconnect community" }));
  assert.equal(reconnects, 1);
  result = null;
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["managed-agent-sync-error"] });
  });
  await waitFor(() => assert.equal(screen.queryByRole("alert") === null, true));
});
