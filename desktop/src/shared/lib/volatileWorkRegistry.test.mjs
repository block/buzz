import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

// The registry hook uses React.useId + React.useEffect, so the mounted-lifecycle
// tests need a DOM. The imperative acquire/release/predicate surface is pure.
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

async function loadRegistry() {
  const mod = await import("./volatileWorkRegistry.ts");
  mod._resetVolatileWorkForTests();
  return mod;
}

afterEach(async () => {
  const { _resetVolatileWorkForTests } = await import(
    "./volatileWorkRegistry.ts"
  );
  _resetVolatileWorkForTests();
});

// ── acquire / release ────────────────────────────────────────────────────────

test("isAnyVolatileWorkPending_no_registrations_returns_false", async () => {
  const { isAnyVolatileWorkPending } = await loadRegistry();
  assert.equal(isAnyVolatileWorkPending(), false);
});

test("acquireVolatileWork_holds_until_released", async () => {
  const { acquireVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  const release = acquireVolatileWork("op-1");
  assert.equal(isAnyVolatileWorkPending(), true);
  release();
  assert.equal(isAnyVolatileWorkPending(), false);
});

test("acquireVolatileWork_same_key_twice_is_idempotent", async () => {
  // Two acquisitions of the same key collapse to one hold; the key is either
  // held or not, so a single release clears it (no refcount drift).
  const { acquireVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  const releaseA = acquireVolatileWork("op-dup");
  const releaseB = acquireVolatileWork("op-dup");
  assert.equal(isAnyVolatileWorkPending(), true);
  releaseA();
  assert.equal(isAnyVolatileWorkPending(), false);
  // The second release is a harmless no-op on an already-clear key.
  releaseB();
  assert.equal(isAnyVolatileWorkPending(), false);
});

test("acquireVolatileWork_distinct_keys_each_hold_independently", async () => {
  const { acquireVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  const releaseA = acquireVolatileWork("op-a");
  const releaseB = acquireVolatileWork("op-b");
  releaseA();
  // b still outstanding — a concurrent picker/upload must keep blocking.
  assert.equal(isAnyVolatileWorkPending(), true);
  releaseB();
  assert.equal(isAnyVolatileWorkPending(), false);
});

// ── withVolatileWork (bounded async seam) ─────────────────────────────────────

test("withVolatileWork_holds_while_running_releases_on_resolve", async () => {
  const { withVolatileWork, isAnyVolatileWorkPending } = await loadRegistry();
  let releaseInner;
  const pending = withVolatileWork(
    "op",
    () => new Promise((resolve) => (releaseInner = () => resolve("done"))),
  );
  // The hold is live while the wrapped operation is in flight.
  assert.equal(isAnyVolatileWorkPending(), true);
  releaseInner();
  assert.equal(await pending, "done", "the wrapped result passes through");
  assert.equal(isAnyVolatileWorkPending(), false, "resolve releases the hold");
});

test("withVolatileWork_releases_on_reject", async () => {
  // A failed picker/upload must not leak its hold — the reload would be blocked
  // forever with nothing to surface it.
  const { withVolatileWork, isAnyVolatileWorkPending } = await loadRegistry();
  await assert.rejects(
    withVolatileWork("op", async () => {
      throw new Error("picker failed");
    }),
  );
  assert.equal(isAnyVolatileWorkPending(), false, "reject releases the hold");
});

test("withVolatileWork_concurrent_calls_hold_independently", async () => {
  // Two overlapping operations (composer paperclip + custom-emoji upload) must
  // get distinct keys so one settling does not release the other's hold.
  const { withVolatileWork, isAnyVolatileWorkPending } = await loadRegistry();
  let releaseA;
  let releaseB;
  const a = withVolatileWork(
    "op",
    () => new Promise((resolve) => (releaseA = resolve)),
  );
  const b = withVolatileWork(
    "op",
    () => new Promise((resolve) => (releaseB = resolve)),
  );
  assert.equal(isAnyVolatileWorkPending(), true);
  releaseA();
  await a;
  assert.equal(
    isAnyVolatileWorkPending(),
    true,
    "the second operation's hold must survive the first settling",
  );
  releaseB();
  await b;
  assert.equal(isAnyVolatileWorkPending(), false);
});

// ── predicates ───────────────────────────────────────────────────────────────

test("registerVolatileWorkPredicate_reports_pending_on_demand", async () => {
  const { registerVolatileWorkPredicate, isAnyVolatileWorkPending } =
    await loadRegistry();
  let busy = false;
  registerVolatileWorkPredicate(() => busy);
  assert.equal(isAnyVolatileWorkPending(), false);
  busy = true;
  assert.equal(isAnyVolatileWorkPending(), true);
  busy = false;
  assert.equal(isAnyVolatileWorkPending(), false);
});

test("registerVolatileWorkPredicate_unregister_stops_reads", async () => {
  const { registerVolatileWorkPredicate, isAnyVolatileWorkPending } =
    await loadRegistry();
  const unregister = registerVolatileWorkPredicate(() => true);
  assert.equal(isAnyVolatileWorkPending(), true);
  unregister();
  assert.equal(isAnyVolatileWorkPending(), false);
});

test("isAnyVolatileWorkPending_holder_alone_is_enough", async () => {
  // A held key blocks even when every predicate reports idle.
  const {
    acquireVolatileWork,
    registerVolatileWorkPredicate,
    isAnyVolatileWorkPending,
  } = await loadRegistry();
  registerVolatileWorkPredicate(() => false);
  const release = acquireVolatileWork("op-solo");
  assert.equal(isAnyVolatileWorkPending(), true);
  release();
  assert.equal(isAnyVolatileWorkPending(), false);
});

// ── useRegisterVolatileWork (mounted lifecycle) ────────────────────────────────

test("useRegisterVolatileWork_holds_while_active_true", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    renderHook(() => useRegisterVolatileWork(true));
    assert.equal(isAnyVolatileWorkPending(), true);
  } finally {
    cleanup();
  }
});

test("useRegisterVolatileWork_active_false_does_not_hold", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    renderHook(() => useRegisterVolatileWork(false));
    assert.equal(isAnyVolatileWorkPending(), false);
  } finally {
    cleanup();
  }
});

test("useRegisterVolatileWork_releases_when_active_flips_false", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    const { rerender } = renderHook(
      ({ active }) => useRegisterVolatileWork(active),
      { initialProps: { active: true } },
    );
    assert.equal(isAnyVolatileWorkPending(), true);
    rerender({ active: false });
    assert.equal(
      isAnyVolatileWorkPending(),
      false,
      "clearing work while mounted must stop blocking",
    );
  } finally {
    cleanup();
  }
});

test("useRegisterVolatileWork_releases_on_unmount", async () => {
  // A component torn down mid-work (the forum view closing with unsent text)
  // must not leak its key — a leaked key blocks the reload forever.
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    const { unmount } = renderHook(() => useRegisterVolatileWork(true));
    assert.equal(isAnyVolatileWorkPending(), true);
    unmount();
    assert.equal(
      isAnyVolatileWorkPending(),
      false,
      "unmount must release the pending key",
    );
  } finally {
    cleanup();
  }
});

test("useRegisterVolatileWork_two_mounts_use_distinct_keys", async () => {
  // Two composers active at once must not collide on one key: releasing one
  // must leave the other's hold intact.
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    const first = renderHook(() => useRegisterVolatileWork(true));
    const second = renderHook(() => useRegisterVolatileWork(true));
    assert.equal(isAnyVolatileWorkPending(), true);
    first.unmount();
    assert.equal(
      isAnyVolatileWorkPending(),
      true,
      "the second mount's key must survive the first's unmount",
    );
    second.unmount();
    assert.equal(isAnyVolatileWorkPending(), false);
  } finally {
    cleanup();
  }
});

test("useRegisterVolatileWork_strict_mode_settles_to_single_hold", async () => {
  // StrictMode double-invokes the effect (acquire → release → acquire). Because
  // the key is stable across the replay and the registry is keyed, the final
  // state is a single hold that a single unmount fully releases.
  const React = await import("react");
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useRegisterVolatileWork, isAnyVolatileWorkPending } =
    await loadRegistry();
  try {
    const { unmount } = renderHook(() => useRegisterVolatileWork(true), {
      wrapper: ({ children }) =>
        React.createElement(React.StrictMode, null, children),
    });
    assert.equal(isAnyVolatileWorkPending(), true);
    unmount();
    assert.equal(
      isAnyVolatileWorkPending(),
      false,
      "StrictMode replay must not leave a residual hold after unmount",
    );
  } finally {
    cleanup();
  }
});
