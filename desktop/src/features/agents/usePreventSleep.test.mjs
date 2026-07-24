import { test } from "node:test";

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { PreventSleepProvider } from "@/features/agents/usePreventSleep.ts";
import { installMinimalReactDom } from "@/testing/minimalReactDom.mjs";
import { expectNoUnhandledRejection } from "@/testing/unhandledRejection.mjs";

installMinimalReactDom();

function installLocalStorage() {
  const values = new Map();
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
  };
}

test("PreventSleepProvider mounts in browser-only Vite mode", async () => {
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  installLocalStorage();

  const root = createRoot(document.createElement("div"));
  const queryClient = new QueryClient();

  await expectNoUnhandledRejection(async () => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(PreventSleepProvider, null, null),
        ),
      );
    });
  });

  await act(async () => {
    root.unmount();
  });
  queryClient.clear();
});
