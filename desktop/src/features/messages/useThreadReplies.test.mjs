import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";

import { useThreadRepliesForRoots } from "./useThreadReplies.ts";

test("thread replies trust the relay-provided aux closure", async () => {
  const source = await readFile(
    new URL("./useThreadReplies.ts", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(
    source,
    /withThreadAux|fetchStructuralAuxForMessages|fetchAuxEventsByReference/,
  );
  assert.match(source, /replies\.push\(\.\.\.response\.events\)/);
});

function reply(id, rootId) {
  return {
    id,
    pubkey: "a".repeat(64),
    kind: 9,
    created_at: 1_700_000_000,
    content: "reply",
    tags: [["e", rootId, "", "reply"]],
    sig: "sig",
  };
}

async function withHookEnvironment(run) {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost/",
  });
  const previousGlobals = {
    window: globalThis.window,
    document: globalThis.document,
    navigator: globalThis.navigator,
    tauri: globalThis.__TAURI_INTERNALS__,
    act: globalThis.IS_REACT_ACT_ENVIRONMENT,
  };
  const pendingRequests = new Map();
  const tauriInternals = {
    invoke: (command, args) => {
      assert.equal(command, "get_thread_replies");
      return new Promise((resolve) => {
        pendingRequests.set(args.rootEventId, resolve);
      });
    },
    transformCallback: () => 1,
  };
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    __TAURI_INTERNALS__: tauriInternals,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  dom.window.__TAURI_INTERNALS__ = tauriInternals;

  try {
    const { act, renderHook } = await import("@testing-library/react");
    await run({ act, pendingRequests, renderHook });
  } finally {
    dom.window.close();
    Object.assign(globalThis, {
      window: previousGlobals.window,
      document: previousGlobals.document,
      __TAURI_INTERNALS__: previousGlobals.tauri,
      IS_REACT_ACT_ENVIRONMENT: previousGlobals.act,
    });
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: previousGlobals.navigator,
    });
  }
}

function hookWrapper(client) {
  return ({ children }) =>
    React.createElement(QueryClientProvider, { client }, children);
}

const channel = { id: "huddle", channelType: "stream" };

test("window-seeded huddle roots fetch and settle without a visible gap", async () => {
  await withHookEnvironment(async ({ act, pendingRequests, renderHook }) => {
    const seededRootId = "3".repeat(64);
    const seededReply = reply("4".repeat(64), seededRootId);
    const authoritativeReply = reply("5".repeat(64), seededRootId);
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: 0 } },
    });
    const placeholderDataByRoot = new Map([[seededRootId, [seededReply]]]);
    const view = renderHook(
      () =>
        useThreadRepliesForRoots(channel, [seededRootId], {
          placeholderDataByRoot,
        }),
      { wrapper: hookWrapper(client) },
    );

    try {
      assert.deepEqual(view.result.current.events, [seededReply]);
      assert.equal(pendingRequests.has(seededRootId), true);
      await act(async () => {
        pendingRequests.get(seededRootId)({
          events: [authoritativeReply],
          next_cursor: null,
        });
        for (
          let attempts = 0;
          view.result.current.events[0]?.id !== authoritativeReply.id &&
          attempts < 50;
          attempts += 1
        ) {
          await new Promise((resolve) => setTimeout(resolve, 10));
        }
      });
      assert.deepEqual(view.result.current.events, [authoritativeReply]);
    } finally {
      view.unmount();
      await client.cancelQueries();
      client.clear();
      client.unmount();
    }
  });
});
