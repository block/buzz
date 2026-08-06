import assert from "node:assert/strict";
import test from "node:test";

import {
  acquireBodySelectionLock,
  calculateOverlayScrollbarGeometry,
  calculateScrollTopFromThumbDrag,
} from "./useOverlayScrollbar.ts";

test("restores selection after overlapping drags finish in either order", () => {
  for (const releaseOrder of ["first-first", "second-first"]) {
    const style = { userSelect: "text" };
    const releaseFirstDrag = acquireBodySelectionLock(style);
    const releaseSecondDrag = acquireBodySelectionLock(style);

    assert.equal(style.userSelect, "none");

    if (releaseOrder === "first-first") {
      releaseFirstDrag();
      assert.equal(style.userSelect, "none");
      releaseSecondDrag();
    } else {
      releaseSecondDrag();
      assert.equal(style.userSelect, "none");
      releaseFirstDrag();
    }

    assert.equal(style.userSelect, "text");
  }
});

test("selection lock release is idempotent across finish and cleanup", () => {
  const style = { userSelect: "contain" };
  const releaseDrag = acquireBodySelectionLock(style);

  releaseDrag();
  releaseDrag();

  assert.equal(style.userSelect, "contain");

  const releaseNextDrag = acquireBodySelectionLock(style);
  assert.equal(style.userSelect, "none");
  releaseNextDrag();
  assert.equal(style.userSelect, "contain");
});

test("bounds the thumb above the composer at maximum scroll", () => {
  for (const bottomInset of [64, 96, 180]) {
    const clientHeight = 600;
    const scrollHeight = 3_000;
    const geometry = calculateOverlayScrollbarGeometry({
      bottomInset,
      clientHeight,
      scrollHeight,
      scrollTop: scrollHeight - clientHeight,
    });

    assert.ok(geometry);
    assert.equal(geometry.trackHeight, clientHeight - bottomInset);
    assert.equal(
      geometry.thumbOffset + geometry.thumbHeight,
      geometry.trackHeight,
    );
  }
});

test("uses a minimum grab target without exceeding the bounded track", () => {
  const geometry = calculateOverlayScrollbarGeometry({
    bottomInset: 120,
    clientHeight: 400,
    scrollHeight: 40_000,
    scrollTop: 20_000,
  });

  assert.ok(geometry);
  assert.equal(geometry.thumbHeight, 24);
  assert.ok(geometry.thumbOffset >= 0);
  assert.ok(
    geometry.thumbOffset + geometry.thumbHeight <= geometry.trackHeight,
  );
});

test("clamps overscrolled positions into the available thumb travel", () => {
  const beforeStart = calculateOverlayScrollbarGeometry({
    bottomInset: 100,
    clientHeight: 500,
    scrollHeight: 2_000,
    scrollTop: -50,
  });
  const afterEnd = calculateOverlayScrollbarGeometry({
    bottomInset: 100,
    clientHeight: 500,
    scrollHeight: 2_000,
    scrollTop: 2_000,
  });

  assert.ok(beforeStart);
  assert.ok(afterEnd);
  assert.equal(beforeStart.thumbOffset, 0);
  assert.equal(afterEnd.thumbOffset, afterEnd.maxThumbOffset);
});

test("hides when content does not overflow or the visible track is too small", () => {
  assert.equal(
    calculateOverlayScrollbarGeometry({
      bottomInset: 100,
      clientHeight: 500,
      scrollHeight: 500,
      scrollTop: 0,
    }),
    null,
  );
  assert.equal(
    calculateOverlayScrollbarGeometry({
      bottomInset: 480,
      clientHeight: 500,
      scrollHeight: 2_000,
      scrollTop: 0,
    }),
    null,
  );
});

test("maps a full bounded-thumb drag across the full scroll range", () => {
  const geometry = calculateOverlayScrollbarGeometry({
    bottomInset: 120,
    clientHeight: 600,
    scrollHeight: 3_000,
    scrollTop: 0,
  });

  assert.ok(geometry);
  assert.equal(
    calculateScrollTopFromThumbDrag({
      deltaY: geometry.maxThumbOffset,
      dragStartScrollTop: 0,
      maxThumbOffset: geometry.maxThumbOffset,
      scrollRange: geometry.scrollRange,
    }),
    geometry.scrollRange,
  );
});

test("clamps thumb drags at both ends of the scroll range", () => {
  assert.equal(
    calculateScrollTopFromThumbDrag({
      deltaY: -1_000,
      dragStartScrollTop: 300,
      maxThumbOffset: 200,
      scrollRange: 1_500,
    }),
    0,
  );
  assert.equal(
    calculateScrollTopFromThumbDrag({
      deltaY: 1_000,
      dragStartScrollTop: 300,
      maxThumbOffset: 200,
      scrollRange: 1_500,
    }),
    1_500,
  );
});
