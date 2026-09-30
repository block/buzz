import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createElement } from "react";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  KnownAgentPubkeysProvider,
  useIsOtherSetupAgent,
} from "./useKnownAgentPubkeys.tsx";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
});
after(() => dom.window.close());

test("provenance context follows exact local inventory and rejects failed cached reads", async () => {
  const { act, renderHook, cleanup } = await import("@testing-library/react");
  const owner = "a".repeat(64),
    remote = "b".repeat(64),
    local = "c".repeat(64);
  const client = new QueryClient({
    defaultOptions: {
      queries: { enabled: false, retry: false, staleTime: Infinity },
    },
  });
  client.setQueryData(["identity"], { pubkey: owner });
  client.setQueryData(
    ["managed-agents"],
    [{ pubkey: local, status: "stopped" }],
  );
  client.setQueryData(
    ["relay-agents"],
    [{ pubkey: remote, ownerPubkey: owner }],
  );
  const wrapper = ({ children }) =>
    createElement(
      QueryClientProvider,
      { client },
      createElement(KnownAgentPubkeysProvider, null, children),
    );
  const { result } = renderHook(
    () => [
      useIsOtherSetupAgent(remote),
      useIsOtherSetupAgent(local, owner),
      useIsOtherSetupAgent("d".repeat(64), owner),
    ],
    { wrapper },
  );
  assert.deepEqual(result.current, [true, false, true]);
  await act(async () =>
    client.setQueryData(
      ["managed-agents"],
      [{ pubkey: remote, status: "deployed" }],
    ),
  );
  assert.deepEqual(result.current, [false, true, true]);
  await act(async () => {
    client
      .getQueryCache()
      .find({ queryKey: ["managed-agents"] })
      .setState({ error: new Error("inventory unavailable"), status: "error" });
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
  assert.deepEqual(result.current, [false, false, false]);
  cleanup();
  client.clear();
});
