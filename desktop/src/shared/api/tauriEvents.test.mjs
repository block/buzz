import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";

import { listenTauriEvent } from "@/shared/api/tauriEvents.ts";

afterEach(() => {
  clearMocks();
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;
});

function installWindow() {
  Object.defineProperty(globalThis, "window", {
    value: globalThis,
    configurable: true,
  });
}

test("listenTauriEvent no-ops before touching Tauri internals outside Tauri", async () => {
  installWindow();
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;

  const unlisten = await listenTauriEvent("browser-only-event", () => {
    throw new Error("browser-only mode should not receive native events");
  });

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
});

test("listenTauriEvent no-ops when isTauri is truthy without the callback bridge", async () => {
  installWindow();
  globalThis.isTauri = true;
  delete globalThis.__TAURI_INTERNALS__;

  const unlisten = await listenTauriEvent("missing-bridge-event", () => {
    throw new Error("missing bridge should not receive native events");
  });

  assert.equal(typeof unlisten, "function");
  assert.doesNotThrow(() => unlisten());
});

test("listenTauriEvent preserves native listener registration and cleanup", async () => {
  installWindow();
  globalThis.isTauri = true;
  mockIPC(
    () => {
      throw new Error("event plugin calls should be handled by the Tauri mock");
    },
    { shouldMockEvents: true },
  );

  let payload;
  const unlisten = await listenTauriEvent("native-event", (event) => {
    payload = event.payload;
  });

  await emit("native-event", "hello");
  assert.equal(payload, "hello");

  await unlisten();
  const warnings = [];
  const originalWarn = console.warn;
  console.warn = (...args) => warnings.push(args);
  try {
    await emit("native-event", "ignored");
  } finally {
    console.warn = originalWarn;
  }
  assert.equal(payload, "hello");
  assert.equal(warnings.length, 1);
});
