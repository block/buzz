import assert from "node:assert/strict";
import test from "node:test";

import {
  createVirtualizedViewportResizeHandler,
  readObservedViewportSize,
  shouldSettleVirtualizedViewportResize,
} from "./useVirtualizedViewportResize.ts";

test("viewport resize follows the virtualizer's explicit bottom state", () => {
  assert.equal(
    shouldSettleVirtualizedViewportResize({ virtualizerAtBottom: true }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedViewportResize({ virtualizerAtBottom: false }),
    false,
  );
});

test("a resize that does not change geometry does not settle", () => {
  const size = { width: 800, height: 600 };
  assert.equal(
    shouldSettleVirtualizedViewportResize({
      virtualizerAtBottom: true,
      previousSize: size,
      nextSize: { ...size },
    }),
    false,
  );
});

test("a real geometry change still settles", () => {
  const previousSize = { width: 800, height: 600 };
  assert.equal(
    shouldSettleVirtualizedViewportResize({
      virtualizerAtBottom: true,
      previousSize,
      nextSize: { width: 800, height: 540 },
    }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedViewportResize({
      virtualizerAtBottom: true,
      previousSize,
      nextSize: { width: 783, height: 600 },
    }),
    true,
  );
});

test("the first delivery settles because there is no baseline to compare", () => {
  assert.equal(
    shouldSettleVirtualizedViewportResize({
      virtualizerAtBottom: true,
      previousSize: null,
      nextSize: { width: 800, height: 600 },
    }),
    true,
  );
});

test("border-box sizing is preferred over contentRect", () => {
  assert.deepEqual(
    readObservedViewportSize({
      borderBoxSize: [{ inlineSize: 800, blockSize: 600 }],
      contentRect: { width: 1, height: 2 },
    }),
    { width: 800, height: 600 },
  );
  assert.deepEqual(
    readObservedViewportSize({ contentRect: { width: 640, height: 480 } }),
    { width: 640, height: 480 },
  );
  assert.equal(readObservedViewportSize({}), null);
});

// ── Observer wiring ──────────────────────────────────────────────────────────

function harness({ atBottom = true } = {}) {
  const frames = [];
  const cancelled = [];
  let settles = 0;
  const handler = createVirtualizedViewportResizeHandler({
    virtualizerAtBottomRef: { current: atBottom },
    settleAtBottom: () => {
      settles += 1;
    },
    requestFrame: (cb) => {
      frames.push(cb);
      return frames.length;
    },
    cancelFrame: (handle) => cancelled.push(handle),
  });
  return {
    handler,
    cancelled,
    settles: () => settles,
    pendingFrames: () => frames.length,
    runFrames: () => {
      const queued = frames.splice(0);
      for (const cb of queued) cb();
    },
    deliver: (width, height) =>
      handler.handleEntries([
        { borderBoxSize: [{ inlineSize: width, blockSize: height }] },
      ]),
  };
}

test("the settle write is deferred out of the delivery pass", () => {
  const h = harness();
  h.deliver(800, 600);
  // Nothing may run synchronously — a scroll write here resizes the observed
  // element inside the pass that is delivering to us.
  assert.equal(h.settles(), 0);
  assert.equal(h.pendingFrames(), 1);
  h.runFrames();
  assert.equal(h.settles(), 1);
});

test("a re-entrant delivery does not queue a second frame", () => {
  const h = harness();
  h.deliver(800, 600);
  h.deliver(800, 540);
  assert.equal(h.pendingFrames(), 1);
  h.runFrames();
  assert.equal(h.settles(), 1);
});

test("a resize back to the same geometry settles nothing", () => {
  const h = harness();
  h.deliver(800, 600);
  h.runFrames();
  assert.equal(h.settles(), 1);

  h.deliver(800, 600);
  assert.equal(h.pendingFrames(), 0);
  h.runFrames();
  assert.equal(h.settles(), 1, "identical geometry must not re-settle");
});

test("geometry is tracked even while the virtualizer is not at bottom", () => {
  const h = harness({ atBottom: false });
  h.deliver(800, 600);
  h.runFrames();
  assert.equal(h.settles(), 0);
});

test("cancel releases a pending frame", () => {
  const h = harness();
  h.deliver(800, 600);
  h.handler.cancel();
  assert.deepEqual(h.cancelled, [1]);
  // A second cancel is a no-op rather than a double free.
  h.handler.cancel();
  assert.deepEqual(h.cancelled, [1]);
});
