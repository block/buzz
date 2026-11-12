/**
 * Shift-mode scroll compensation in the patched Virtua build
 * (patches/virtua@0.49.3.patch), driven through the real `VList`.
 *
 * A head trim (a channel-window refresh dropping older pages) shrinks the
 * list from the top. When the reader is within the removed height of the
 * floor, the browser clamps `scrollTop` to the new floor before Virtua's
 * layout-effect compensation runs. Compensating from that clamped offset
 * counts the removal twice and throws the reader toward older history.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { VList } from "virtua";

const ROW = 50;
const VIEWPORT = 500;
const ROWS = Array.from({ length: 100 }, (_, index) => index);

// jsdom has no layout. Model the browser behavior under test: an
// `overflow-y: auto` element's scroll offset is clamped to its scrollable
// extent, which Virtua sets as its inner container's height.
const offsets = new WeakMap();
const isScroller = (element) => element.style?.overflowY === "auto";
const maxOffset = (element) =>
  Math.max(
    (Number.parseFloat(element.firstElementChild?.style.height) || 0) -
      VIEWPORT,
    0,
  );
const clampOffset = (element, value) =>
  Math.min(Math.max(value, 0), maxOffset(element));
Object.defineProperties(window.HTMLElement.prototype, {
  clientHeight: {
    configurable: true,
    get() {
      return isScroller(this) ? VIEWPORT : 0;
    },
  },
  scrollHeight: {
    configurable: true,
    get() {
      return isScroller(this) ? maxOffset(this) + VIEWPORT : 0;
    },
  },
  scrollTop: {
    configurable: true,
    get() {
      const offset = clampOffset(this, offsets.get(this) ?? 0);
      offsets.set(this, offset);
      return offset;
    },
    set(value) {
      offsets.set(this, clampOffset(this, value));
    },
  },
  offsetParent: {
    configurable: true,
    get() {
      return this.parentElement;
    },
  },
});

const resizeObservers = new Set();
window.ResizeObserver = class {
  constructor(callback) {
    this.callback = callback;
    this.targets = new Set();
    resizeObservers.add(this);
  }
  observe(target) {
    this.targets.add(target);
  }
  unobserve(target) {
    this.targets.delete(target);
  }
  disconnect() {
    resizeObservers.delete(this);
  }
};

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

function renderList(rows, shift) {
  act(() =>
    root.render(
      React.createElement(
        VList,
        { data: rows, itemSize: () => ROW, shift },
        (row) => React.createElement("div", { key: row }, `row ${row}`),
      ),
    ),
  );
}

/** Mounts the list and returns its scroller, with the viewport measured. */
function mountList(rows) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  renderList(rows, false);
  const scroller = container.firstElementChild;
  act(() => {
    for (const observer of resizeObservers) {
      if (observer.targets.has(scroller)) {
        observer.callback([
          { target: scroller, contentRect: { width: 300, height: VIEWPORT } },
        ]);
      }
    }
  });
  return scroller;
}

function readerScrollsTo(scroller, offset) {
  act(() => {
    scroller.scrollTop = offset;
    scroller.dispatchEvent(new window.Event("scroll"));
  });
}

// 100 rows of 50px in a 500px viewport: the floor is at 4500, and trimming
// 10 rows removes 500px, more than the reader's distance from the floor in
// the last two cases.
for (const { name, start, trim, expected } of [
  { name: "far from the floor", start: 2000, trim: 10, expected: 1500 },
  { name: "near the floor", start: 4400, trim: 10, expected: 3900 },
  { name: "at the floor", start: 4500, trim: 10, expected: 4000 },
]) {
  test(`a head trim ${name} keeps the reader's rows in place`, () => {
    const scroller = mountList(ROWS);
    readerScrollsTo(scroller, start);

    renderList(ROWS.slice(trim), true);

    assert.equal(scroller.scrollTop, expected);
  });
}

test("a head trim keeps a scroll the browser has not reported yet", () => {
  // The reader's latest move is still waiting for its scroll event, so the
  // live offset, not the last observed one, is where the reader is.
  const scroller = mountList(ROWS);
  readerScrollsTo(scroller, 3000);
  scroller.scrollTop = 2400;

  renderList(ROWS.slice(10), true);

  assert.equal(scroller.scrollTop, 1900);
});

test("shift compensations before the next scroll event accumulate", () => {
  // Successive shift writes land before the browser reports any of them, so
  // each must build on the last write, including across a floor clamp.
  const scroller = mountList(ROWS);
  readerScrollsTo(scroller, 4500);
  const older = Array.from({ length: 10 }, (_, index) => -10 + index);

  renderList([...older, ...ROWS], true);
  assert.equal(scroller.scrollTop, 5000);

  renderList(ROWS.slice(10), true);
  assert.equal(scroller.scrollTop, 4000);
});
