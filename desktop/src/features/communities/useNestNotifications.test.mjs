import { afterEach, mock, test } from "node:test";

import assert from "node:assert/strict";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { clearMocks as clearTauriMocks, mockIPC } from "@tauri-apps/api/mocks";
import { toast } from "sonner";

import { installMinimalReactDom } from "@/testing/minimalReactDom.mjs";
import { expectNoUnhandledRejection } from "@/testing/unhandledRejection.mjs";
import { useNestNotifications } from "@/features/communities/useNestNotifications.ts";

installMinimalReactDom();

afterEach(() => {
  mock.reset();
  clearTauriMocks();
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;
  delete globalThis.localStorage;
});

function installLocalStorage() {
  const values = new Map();
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
  };
}

test("useNestNotifications mounts in browser-only Vite mode", async () => {
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;

  function Host() {
    useNestNotifications();
    return null;
  }

  const root = createRoot(document.createElement("div"));

  await expectNoUnhandledRejection(async () => {
    await act(async () => {
      root.render(React.createElement(Host));
    });
  });

  await act(async () => {
    root.unmount();
  });
});

test("useNestNotifications handles native events and unregisters on cleanup", async () => {
  globalThis.isTauri = true;
  installLocalStorage();
  mockIPC(
    () => {
      throw new Error("event plugin calls should be handled by the Tauri mock");
    },
    { shouldMockEvents: true },
  );

  const errors = [];
  const successes = [];
  mock.method(toast, "error", (...args) => errors.push(args));
  mock.method(toast, "success", (...args) => successes.push(args));

  function Host() {
    useNestNotifications();
    return null;
  }

  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(React.createElement(Host));
  });

  await emit("repos-dir-error", "missing workspace");
  await emit("legacy-nest-migrated");

  assert.equal(errors.length, 1);
  assert.deepEqual(errors[0], [
    "Repos directory not applied",
    { description: "missing workspace" },
  ]);
  assert.equal(successes.length, 1);
  assert.equal(successes[0][0], "Migrated notes from ~/.sprout");

  await act(async () => {
    root.unmount();
  });
  const warnings = [];
  const originalWarn = console.warn;
  console.warn = (...args) => warnings.push(args);
  try {
    await emit("repos-dir-error", "after cleanup");
  } finally {
    console.warn = originalWarn;
  }
  assert.equal(errors.length, 1);
  assert.equal(warnings.length, 1);
});
