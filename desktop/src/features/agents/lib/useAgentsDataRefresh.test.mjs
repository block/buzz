import { afterEach, mock, test } from "node:test";

import assert from "node:assert/strict";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks as clearTauriMocks, mockIPC } from "@tauri-apps/api/mocks";

import { useAgentsDataRefresh } from "@/features/agents/lib/useAgentsDataRefresh.ts";
import { installMinimalReactDom } from "@/testing/minimalReactDom.mjs";
import { expectNoUnhandledRejection } from "@/testing/unhandledRejection.mjs";

installMinimalReactDom();

afterEach(() => {
  mock.reset();
  clearTauriMocks();
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;
});

test("useAgentsDataRefresh mounts in browser-only Vite mode", async () => {
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;

  function Host() {
    useAgentsDataRefresh();
    return null;
  }

  const root = createRoot(document.createElement("div"));
  const queryClient = new QueryClient();

  await expectNoUnhandledRejection(async () => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Host),
        ),
      );
    });
  });

  await act(async () => {
    root.unmount();
  });
  queryClient.clear();
});

test("useAgentsDataRefresh handles native runtime status events and unregisters on cleanup", async () => {
  globalThis.isTauri = true;
  mockIPC(
    () => {
      throw new Error("event plugin calls should be handled by the Tauri mock");
    },
    { shouldMockEvents: true },
  );

  function Host() {
    useAgentsDataRefresh();
    return null;
  }

  const root = createRoot(document.createElement("div"));
  const queryClient = new QueryClient();
  const invalidations = [];
  mock.method(queryClient, "invalidateQueries", (options) => {
    invalidations.push(options.queryKey);
    return Promise.resolve();
  });

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Host),
      ),
    );
  });

  await emit("managed-agent-runtime-status", { runtime: "buzz-agent" });

  assert.deepEqual(invalidations, [
    ["managed-agent-runtimes"],
    ["managed-agents"],
  ]);

  await act(async () => {
    root.unmount();
  });
  const warnings = [];
  const originalWarn = console.warn;
  console.warn = (...args) => warnings.push(args);
  try {
    await emit("managed-agent-runtime-status", { runtime: "buzz-agent" });
  } finally {
    console.warn = originalWarn;
  }
  assert.deepEqual(invalidations, [
    ["managed-agent-runtimes"],
    ["managed-agents"],
  ]);
  assert.equal(warnings.length, 1);

  queryClient.clear();
});
