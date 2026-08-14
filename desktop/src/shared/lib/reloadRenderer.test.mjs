import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import {
  RELOAD_TEARDOWN_TIMEOUT_MS,
  requestRendererReload,
  resetRendererReloadForTests,
} from "./reloadRenderer.ts";

// requestRendererReload takes injectable `reload`/`closeSockets`, so the
// teardown/timeout race and idempotence run headless without navigating the
// runner. Only `window.setTimeout` needs a stub for the timeout arm.
let originalWindow;

beforeEach(() => {
  originalWindow = globalThis.window;
  globalThis.window = {
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (id) => clearTimeout(id),
  };
});

afterEach(() => {
  globalThis.window = originalWindow;
  resetRendererReloadForTests();
});

test("requestRendererReload_tears_down_sockets_then_reloads", async () => {
  let reloadCount = 0;
  let closeCalls = 0;
  let resolveClose;
  const done = requestRendererReload({
    reload: () => {
      reloadCount += 1;
    },
    closeSockets: () => {
      closeCalls += 1;
      return new Promise((resolve) => {
        resolveClose = resolve;
      });
    },
  });
  // Teardown started; reload waits for the race to settle.
  assert.equal(closeCalls, 1);
  assert.equal(reloadCount, 0);
  resolveClose();
  await done;
  assert.equal(reloadCount, 1);
});

test("requestRendererReload_reloads_even_when_teardown_hangs_past_timeout", async () => {
  let reloadCount = 0;
  // Never resolve the teardown; only the timeout can win the race.
  await requestRendererReload({
    reload: () => {
      reloadCount += 1;
    },
    closeSockets: () => new Promise(() => {}),
  });
  assert.equal(reloadCount, 1);
  assert.ok(RELOAD_TEARDOWN_TIMEOUT_MS > 0);
});

test("requestRendererReload_reloads_even_when_teardown_rejects", async () => {
  let reloadCount = 0;
  // A rejecting teardown must not win the race and strand the stale renderer.
  await requestRendererReload({
    reload: () => {
      reloadCount += 1;
    },
    closeSockets: () => Promise.reject(new Error("socket wedged")),
  });
  assert.equal(reloadCount, 1);
});

test("requestRendererReload_collapses_concurrent_triggers_to_one", async () => {
  let reloadCount = 0;
  let closeCalls = 0;
  let resolveClose;
  const overrides = {
    reload: () => {
      reloadCount += 1;
    },
    closeSockets: () => {
      closeCalls += 1;
      return new Promise((resolve) => {
        resolveClose = resolve;
      });
    },
  };
  // Cmd+R and the idle backstop fire while the first teardown is mid-flight.
  const first = requestRendererReload(overrides);
  const second = requestRendererReload(overrides);
  assert.equal(first, second);
  assert.equal(closeCalls, 1);
  resolveClose();
  await Promise.all([first, second]);
  // Teardown ran once; the webview navigated once.
  assert.equal(closeCalls, 1);
  assert.equal(reloadCount, 1);
});

test("requestRendererReload_stays_collapsed_after_settling", async () => {
  let reloadCount = 0;
  const overrides = {
    reload: () => {
      reloadCount += 1;
    },
    closeSockets: () => Promise.resolve(),
  };
  await requestRendererReload(overrides);
  assert.equal(reloadCount, 1);
  // In production location.reload() discards the module; the guard is never
  // reset, so a settled reload cannot start a second on the dying page.
  await requestRendererReload(overrides);
  assert.equal(reloadCount, 1);
});
