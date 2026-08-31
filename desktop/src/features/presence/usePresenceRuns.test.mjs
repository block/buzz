import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  CommunitiesProvider,
  useCommunities,
} from "../communities/useCommunities.tsx";
import { usePresenceRuns } from "./usePresenceRuns.ts";
import { AgentHostMarker } from "../agents/ui/AgentHostMarker.tsx";

const flush = () => new Promise((resolve) => setTimeout(resolve, 15));
test("live markers expire without a network callback and reject late previous-scope data", async (t) => {
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://fixture.invalid",
  });
  const globals = [
    "window",
    "document",
    "localStorage",
    "IS_REACT_ACT_ENVIRONMENT",
  ];
  const saved = Object.fromEntries(
    globals.map((key) => [key, globalThis[key]]),
  );
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      { id: "a", name: "A", relayUrl: "wss://a.invalid" },
      { id: "b", name: "B", relayUrl: "wss://b.invalid" },
    ]),
  );
  localStorage.setItem("buzz-active-community-id", "a");
  const client = new QueryClient({
    defaultOptions: { queries: { gcTime: Infinity, retry: false } },
  });
  const owner = "a".repeat(64),
    agent = "b".repeat(64);
  client.setQueryData(["identity"], { pubkey: owner });
  const calls = [],
    replies = [];
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (command, args) => {
      assert.equal(command, "get_presence_runs");
      calls.push(args);
      return new Promise((resolve, reject) =>
        replies.push({ resolve, reject }),
      );
    },
  };
  const root = createRoot(document.getElementById("root"));
  let communities, latest;
  function View() {
    communities = useCommunities();
    latest = usePresenceRuns([agent]);
    return React.createElement(AgentHostMarker, {
      runs: latest.data?.[agent],
      now: latest.now,
    });
  }
  t.after(async () => {
    await act(async () => root.unmount());
    client.clear();
    await flush();
    dom.window.close();
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete globalThis[key];
      else globalThis[key] = value;
    }
  });
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(View),
        ),
      ),
    );
    await flush();
  });
  assert.deepEqual(calls[0], {
    expectedOwner: owner,
    relayUrl: "wss://a.invalid",
    pubkeys: [agent],
  });
  const run = {
    run: "c".repeat(32),
    seq: 0,
    status: "online",
    expires_at: Date.now() / 1000 + 0.15,
    location: { host: owner, label: "Workshop" },
    registration: null,
  };
  await act(async () => {
    replies[0].resolve({ [agent]: [run] });
    await flush();
  });
  assert.equal(
    document.querySelector('[title="Running on Workshop"]')?.textContent,
    "Workshop",
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 180));
  });
  assert.equal(document.querySelector('[title="Running on Workshop"]'), null);
  assert.equal(calls.length, 1, "expiry must not need a poll");
  void latest.refetch();
  await act(async () => {
    communities.switchCommunity("b");
    await flush();
  });
  assert.equal(calls[2].relayUrl, "wss://b.invalid");
  await act(async () => {
    replies[1].resolve({
      [agent]: [{ ...run, expires_at: Date.now() / 1000 + 100 }],
    });
    await flush();
  });
  assert.equal(
    document.querySelector('[title="Running on Workshop"]'),
    null,
    "old community response must not render in B",
  );
  await act(async () => {
    replies[2].reject(new Error("unavailable"));
    await flush();
  });
  assert.equal(latest.isError, true);
  assert.equal(
    latest.data,
    undefined,
    "failed snapshot is unknown, not an empty offline snapshot",
  );
  await act(async () => {
    client.setQueryData(["identity"], { pubkey: "d".repeat(64) });
    await flush();
  });
  assert.equal(calls[3].expectedOwner, "d".repeat(64));
  await act(async () => {
    replies[3].resolve({ [agent]: [] });
    await flush();
  });
  assert.deepEqual(latest.data, { [agent]: [] });
});
