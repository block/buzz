import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { relayClient } from "@/shared/api/relayClient";
import { useCanvasLiveUpdates, useCanvasQuery } from "./hooks.ts";

const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const originalSubscribeLive = relayClient.subscribeLive;
let getCanvasCalls = 0;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });

  const tauriInternals = {
    invoke(command) {
      if (command === "get_canvas") {
        getCanvasCalls += 1;
        return Promise.resolve({
          author: "agent-pubkey",
          content: `canvas-${getCanvasCalls}`,
          updated_at: getCanvasCalls,
        });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
    },
    transformCallback(callback) {
      return callback;
    },
    unregisterCallback() {},
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  dom.window.__TAURI_INTERNALS__ = tauriInternals;
});

afterEach(() => {
  relayClient.subscribeLive = originalSubscribeLive;
  getCanvasCalls = 0;
});

after(() => dom.window.close());

function CanvasProbe({ enabled }) {
  useCanvasQuery(CHANNEL, enabled);
  useCanvasLiveUpdates(CHANNEL, enabled);
  return null;
}

function mountCanvas(queryClient) {
  const container = document.createElement("div");
  const root = createRoot(container);

  return {
    async render(nextEnabled) {
      await act(async () => {
        root.render(
          React.createElement(
            QueryClientProvider,
            { client: queryClient },
            React.createElement(CanvasProbe, { enabled: nextEnabled }),
          ),
        );
      });
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
  });
}

test("canvas surface subscribes only while open and refetches on a remote event", async () => {
  let onEvent;
  let unsubscribeCalls = 0;
  const filters = [];
  relayClient.subscribeLive = async (filter, callback) => {
    filters.push(filter);
    onEvent = callback;
    return async () => {
      unsubscribeCalls += 1;
    };
  };

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const mounted = mountCanvas(queryClient, false);
  await mounted.render(false);
  assert.deepEqual(
    filters,
    [],
    "a closed canvas must spend no relay bandwidth",
  );

  await mounted.render(true);
  await settle();
  assert.deepEqual(filters, [{ kinds: [40100], "#h": [CHANNEL], limit: 1 }]);
  assert.equal(getCanvasCalls, 1, "opening performs the initial canvas read");

  await act(async () => {
    onEvent({
      id: "remote-canvas",
      pubkey: "agent-pubkey",
      created_at: 2,
      kind: 40100,
      tags: [["h", CHANNEL]],
      content: "remote update",
      sig: "signature",
    });
  });
  await settle();
  assert.equal(
    getCanvasCalls,
    2,
    "the live event invalidates and refetches the exact canvas query",
  );

  await mounted.render(false);
  await settle();
  assert.equal(
    unsubscribeCalls,
    1,
    "closing tears down the canvas subscription",
  );
  await mounted.unmount();
});

test("late subscription setup is disposed after the surface closes", async () => {
  let resolveSubscribe;
  let unsubscribeCalls = 0;
  relayClient.subscribeLive = () =>
    new Promise((resolve) => {
      resolveSubscribe = resolve;
    });

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const mounted = mountCanvas(queryClient, true);
  await mounted.render(true);
  await mounted.unmount();

  await act(async () => {
    resolveSubscribe(async () => {
      unsubscribeCalls += 1;
    });
    await Promise.resolve();
  });
  assert.equal(unsubscribeCalls, 1);
});
