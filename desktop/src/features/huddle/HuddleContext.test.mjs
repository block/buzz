import { test } from "node:test";

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { HuddleProvider } from "@/features/huddle/HuddleContext.tsx";
import { installMinimalReactDom } from "@/testing/minimalReactDom.mjs";
import { expectNoUnhandledRejection } from "@/testing/unhandledRejection.mjs";

installMinimalReactDom();

function installMediaDevices() {
  globalThis.navigator.mediaDevices = {
    addEventListener: () => {},
    removeEventListener: () => {},
    enumerateDevices: () => Promise.resolve([]),
  };
}

test("HuddleProvider mounts in browser-only Vite mode", async () => {
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  installMediaDevices();

  const root = createRoot(document.createElement("div"));

  await expectNoUnhandledRejection(async () => {
    await act(async () => {
      root.render(React.createElement(HuddleProvider, null, null));
    });
  });

  await act(async () => {
    root.unmount();
  });
});
