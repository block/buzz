import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

after(() => dom.window.close());

test("a set-enabled failure that still commits refreshes the list, but keeps the mutation's own error", async () => {
  const { act, cleanup, renderHook, waitFor } = await import(
    "@testing-library/react"
  );
  const { clearMocks, mockIPC } = await import("@tauri-apps/api/mocks");
  const { usePluginRegistry } = await import("./usePluginRegistry.ts");

  // Simulates a command that partially commits (the registry mutation lands)
  // but then rejects on a later cleanup step: `plugin_list` reflects the
  // mutation having happened even though `plugin_set_enabled` itself throws.
  let enabled = true;
  mockIPC((command) => {
    if (command === "plugin_list") {
      return [
        {
          pluginId: "plugin-1",
          name: "Plugin One",
          version: "1.0.0",
          publisher: "acme",
          enabled,
          contributions: [],
        },
      ];
    }
    if (command === "plugin_set_enabled") {
      enabled = false;
      throw new Error("cleanup failed");
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const { result, unmount } = renderHook(() => usePluginRegistry());
    await waitFor(() => assert.equal(result.current.loading, false));
    assert.equal(result.current.plugins[0].enabled, true);

    await act(async () => {
      await result.current.setEnabled("plugin-1", false);
    });

    assert.equal(
      result.current.error,
      "cleanup failed",
      "the mutation's own error must survive the follow-up refresh",
    );
    assert.equal(
      result.current.plugins[0].enabled,
      false,
      "the list must reflect the commit even though the mutation's promise rejected",
    );

    unmount();
  } finally {
    cleanup();
    clearMocks();
  }
});

test("an uninstall failure that still commits refreshes the list, but keeps the mutation's own error", async () => {
  const { act, cleanup, renderHook, waitFor } = await import(
    "@testing-library/react"
  );
  const { clearMocks, mockIPC } = await import("@tauri-apps/api/mocks");
  const { usePluginRegistry } = await import("./usePluginRegistry.ts");

  let installed = true;
  mockIPC((command) => {
    if (command === "plugin_list") {
      return installed
        ? [
            {
              pluginId: "plugin-1",
              name: "Plugin One",
              version: "1.0.0",
              publisher: "acme",
              enabled: true,
              contributions: [],
            },
          ]
        : [];
    }
    if (command === "plugin_uninstall") {
      installed = false;
      throw new Error("cleanup failed");
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const { result, unmount } = renderHook(() => usePluginRegistry());
    await waitFor(() => assert.equal(result.current.loading, false));
    assert.equal(result.current.plugins.length, 1);

    await act(async () => {
      await result.current.uninstall("plugin-1");
    });

    assert.equal(result.current.error, "cleanup failed");
    assert.equal(
      result.current.plugins.length,
      0,
      "the list must reflect the commit even though the mutation's promise rejected",
    );

    unmount();
  } finally {
    cleanup();
    clearMocks();
  }
});
