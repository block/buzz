import assert from "node:assert/strict";
import { test } from "node:test";

import {
  listenForDeepLinks,
  listenForMessageDeepLinks,
} from "@/shared/deep-link.ts";

function installBrowserOnlyWindow() {
  Object.defineProperty(globalThis, "window", {
    value: globalThis,
    configurable: true,
  });
  delete globalThis.__TAURI_INTERNALS__;
}

test("listenForMessageDeepLinks no-ops in browser-only Vite mode", async () => {
  installBrowserOnlyWindow();

  const unlisten = await listenForMessageDeepLinks(() => {
    throw new Error("browser-only mode should not receive native events");
  });

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
});

test("listenForDeepLinks no-ops in browser-only Vite mode", async () => {
  installBrowserOnlyWindow();
  let availabilityListenerCount = 0;

  const unlisten = await listenForDeepLinks({
    startCommunityOnboarding: () => {
      throw new Error("browser-only mode should not start onboarding");
    },
    openAddCommunity: () => {
      throw new Error("browser-only mode should not open add-community");
    },
    onAddCommunityAvailable: () => {
      availabilityListenerCount += 1;
      return () => {
        availabilityListenerCount -= 1;
      };
    },
  });

  assert.equal(typeof unlisten, "function");
  assert.equal(availabilityListenerCount, 0);
  assert.doesNotThrow(() => unlisten());
  assert.equal(availabilityListenerCount, 0);
});
