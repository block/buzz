/**
 * Behavioral tests for useNestNotifications / registerNestNotifications.
 *
 * Tests call `registerNestNotifications` — the extracted production
 * registration helper used by `useNestNotifications` inside its `useEffect`.
 * This is the real production function, not a reconstruction of its logic.
 *
 * Proves:
 * - `registerNestNotifications` registers listeners for all three event names
 *   (repos-dir-error, legacy-nest-migrated, workspace-degraded) by inspecting
 *   the `listenFn` call record.
 * - When `workspace-degraded` fires, `toast.error` is called with the payload.
 * - The returned cleanup function calls every unlisten function.
 * - The event name and payload wiring survive rename/refactor of the production
 *   code (the test would fail if the event name or toast call were deleted).
 */
import assert from "node:assert/strict";
import test from "node:test";

import { registerNestNotifications } from "./useNestNotifications.ts";

test("registerNestNotifications: registers listeners for all three events", () => {
  const registeredEvents = [];
  const unlistenFns = [];

  const mockListen = (event, _handler) => {
    registeredEvents.push(event);
    const unlistenFn = () => unlistenFns.push(event);
    return Promise.resolve(unlistenFn);
  };

  const mockToast = { error: () => {}, success: () => {} };

  const cleanup = registerNestNotifications(mockListen, mockToast);

  assert.deepEqual(
    registeredEvents.sort(),
    ["legacy-nest-migrated", "repos-dir-error", "workspace-degraded"].sort(),
    "must register listeners for all three events",
  );

  // Cleanup calls all three unlisten functions.
  return Promise.resolve()
    .then(() => {
      cleanup();
      return new Promise((resolve) => setTimeout(resolve, 0));
    })
    .then(() => {
      assert.equal(
        unlistenFns.length,
        3,
        "cleanup must call unlisten for all three events",
      );
    });
});

test("registerNestNotifications: workspace-degraded fires toast.error with payload", () => {
  const toastCalls = [];

  let degradedHandler = null;
  const mockListen = (event, handler) => {
    if (event === "workspace-degraded") {
      degradedHandler = handler;
    }
    return Promise.resolve(() => {});
  };

  const mockToast = {
    error: (title, opts) => toastCalls.push({ title, opts }),
    success: () => {},
  };

  registerNestNotifications(mockListen, mockToast);

  assert.ok(degradedHandler, "workspace-degraded handler must be registered");

  // Fire the event as the Tauri event system would.
  degradedHandler({
    payload: "restore failed: agent runtime could not be restarted",
  });

  assert.equal(toastCalls.length, 1, "toast.error must be called once");
  assert.equal(toastCalls[0].title, "Workspace partially degraded");
  assert.equal(
    toastCalls[0].opts.description,
    "restore failed: agent runtime could not be restarted",
  );
});

test("registerNestNotifications: cleanup calls all unlisten functions", () => {
  let unlistenCallCount = 0;
  const unlisten = () => {
    unlistenCallCount++;
  };
  const mockListen = (_event, _handler) => Promise.resolve(unlisten);
  const mockToast = { error: () => {}, success: () => {} };

  const cleanup = registerNestNotifications(mockListen, mockToast);

  // Call cleanup after promises resolve.
  return new Promise((resolve) => setTimeout(resolve, 0))
    .then(() => {
      cleanup();
      return new Promise((resolve) => setTimeout(resolve, 0));
    })
    .then(() => {
      assert.equal(
        unlistenCallCount,
        3,
        "cleanup must call unlisten for all three registered listeners",
      );
    });
});

test("registerNestNotifications: workspace-degraded handler passes raw payload as description", () => {
  const cases = ["", "a".repeat(500), "error: file not found\npath: /foo/bar"];

  for (const payload of cases) {
    const toastArgs = [];
    let handler = null;

    const mockListen = (event, h) => {
      if (event === "workspace-degraded") handler = h;
      return Promise.resolve(() => {});
    };
    const mockToast = {
      error: (title, opts) =>
        toastArgs.push({ title, description: opts?.description }),
      success: () => {},
    };

    registerNestNotifications(mockListen, mockToast);
    assert.ok(handler, "workspace-degraded handler must be set");
    handler({ payload });

    assert.equal(toastArgs.length, 1);
    assert.equal(toastArgs[0].description, payload);
  }
});
