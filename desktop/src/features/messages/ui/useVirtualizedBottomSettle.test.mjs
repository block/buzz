import assert from "node:assert/strict";
import test from "node:test";

class EventTargetShim {
  listeners = new Map();
  addEventListener(type, listener) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  removeEventListener(type, listener) {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter(
        (current) => current !== listener,
      ),
    );
  }
  dispatchEvent(event) {
    for (const listener of this.listeners.get(event.type) ?? [])
      listener(event);
    return true;
  }
}

class ElementShim extends EventTargetShim {
  constructor(firstElementChild = null) {
    super();
    this.firstElementChild = firstElementChild;
    this.children = [];
    this.childNodes = [];
    this.isContentEditable = false;
    this.nodeName = "DIV";
    this.tagName = "DIV";
    this.nodeType = 1;
    this.namespaceURI = "http://www.w3.org/1999/xhtml";
  }
  get ownerDocument() {
    return globalThis.document;
  }
  appendChild(child) {
    this.children.push(child);
    this.childNodes.push(child);
    return child;
  }
  removeChild(child) {
    this.children = this.children.filter((current) => current !== child);
    this.childNodes = this.childNodes.filter((current) => current !== child);
    return child;
  }
  insertBefore(child, reference) {
    const index = this.children.indexOf(reference);
    if (index < 0) return this.appendChild(child);
    this.children.splice(index, 0, child);
    this.childNodes.splice(index, 0, child);
    return child;
  }
  closest() {
    return null;
  }
  contains(target) {
    return this === target || this.firstElementChild?.contains(target) === true;
  }
}

globalThis.document = {
  addEventListener() {},
  createElement: () => new ElementShim(),
  get defaultView() {
    return globalThis.window;
  },
  nodeType: 9,
  removeEventListener() {},
};

const animationFrames = new Map();
let nextAnimationFrameId = 1;
globalThis.requestAnimationFrame = (callback) => {
  const id = nextAnimationFrameId++;
  animationFrames.set(id, callback);
  return id;
};
globalThis.cancelAnimationFrame = (id) => animationFrames.delete(id);
globalThis.HTMLElement = ElementShim;
globalThis.HTMLDivElement = ElementShim;
globalThis.Node = ElementShim;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
process.env.IS_REACT_ACT_ENVIRONMENT = "true";
Object.defineProperty(globalThis, "window", {
  configurable: true,
  value: new EventTargetShim(),
});
globalThis.window.HTMLIFrameElement = ElementShim;

const resizeObservers = [];
globalThis.ResizeObserver = class {
  constructor(callback) {
    this.callback = callback;
    resizeObservers.push(this);
  }
  disconnect() {}
  observe(target) {
    this.targets = [...(this.targets ?? []), target];
  }
};

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { useVirtualizedBottomSettle } from "./useVirtualizedBottomSettle.ts";

function flushAnimationFrames() {
  const callbacks = [...animationFrames.values()];
  animationFrames.clear();
  for (const callback of callbacks) callback(performance.now());
}

function Harness({ apiRef, hostRef, itemsLengthRef, listRef }) {
  apiRef.current = useVirtualizedBottomSettle(hostRef, listRef, itemsLengthRef);
  return null;
}

async function mountHarness() {
  animationFrames.clear();
  resizeObservers.length = 0;
  const content = new ElementShim();
  const scroller = new ElementShim(content);
  const host = new ElementShim(scroller);
  const writes = [];
  const refs = {
    api: { current: null },
    host: { current: host },
    itemsLength: { current: 5 },
    list: {
      current: {
        scrollToIndex(index, options) {
          writes.push({ index, options });
        },
      },
    },
  };
  const root = createRoot(new ElementShim());
  await act(async () => {
    root.render(
      React.createElement(Harness, {
        apiRef: refs.api,
        hostRef: refs.host,
        itemsLengthRef: refs.itemsLength,
        listRef: refs.list,
      }),
    );
  });
  return { content, refs, root, scroller, writes };
}

test("bottom intent follows arbitrarily late virtual geometry changes", async () => {
  const { content, refs, root, scroller, writes } = await mountHarness();
  refs.api.current.settle();
  assert.deepEqual(writes, [{ index: 4, options: { align: "end" } }]);

  const geometryObserver = resizeObservers.find((observer) =>
    observer.targets?.includes(content),
  );
  assert.ok(geometryObserver);
  geometryObserver.callback();
  flushAnimationFrames();
  assert.equal(writes.length, 2);

  // There is no deadline: another geometry change much later still re-pins.
  geometryObserver.callback();
  flushAnimationFrames();
  assert.equal(writes.length, 3);

  // Viewport changes also move the physical floor without resizing content.
  assert.ok(geometryObserver.targets.includes(scroller));
  geometryObserver.callback();
  flushAnimationFrames();
  assert.equal(writes.length, 4);
  await act(async () => root.unmount());
});

for (const eventType of ["pointerdown", "touchmove", "wheel", "keydown"]) {
  test(`${eventType} transfers ownership away from bottom intent`, async () => {
    const { content, refs, root, scroller, writes } = await mountHarness();
    refs.api.current.settle();
    const target = eventType === "keydown" ? window : scroller;
    target.dispatchEvent({
      altKey: false,
      ctrlKey: false,
      key: eventType === "keydown" ? "PageUp" : undefined,
      metaKey: false,
      target:
        eventType === "keydown"
          ? scroller
          : eventType === "pointerdown"
            ? scroller
            : undefined,
      type: eventType,
    });

    resizeObservers
      .find((observer) => observer.targets?.includes(content))
      .callback();
    flushAnimationFrames();
    assert.equal(writes.length, 1);
    await act(async () => root.unmount());
    assert.equal(
      scroller.listeners.get(eventType)?.length ?? 0,
      0,
      `${eventType} listener is removed on unmount`,
    );
  });
}

test("descendant interaction and outside navigation preserve bottom intent", async () => {
  const { content, refs, root, scroller, writes } = await mountHarness();
  refs.api.current.settle();
  const rowControl = new ElementShim();
  scroller.firstElementChild.firstElementChild = rowControl;

  scroller.dispatchEvent({ target: rowControl, type: "pointerdown" });
  window.dispatchEvent({
    altKey: false,
    ctrlKey: false,
    key: "PageUp",
    metaKey: false,
    target: new ElementShim(),
    type: "keydown",
  });
  resizeObservers
    .find((observer) => observer.targets?.includes(content))
    .callback();
  flushAnimationFrames();

  assert.equal(writes.length, 2);
  await act(async () => root.unmount());
});

test("Ctrl+wheel zoom preserves bottom intent through geometry reflow", async () => {
  const { content, refs, root, scroller, writes } = await mountHarness();
  refs.api.current.settle();

  scroller.dispatchEvent({ ctrlKey: true, deltaY: -100, type: "wheel" });
  resizeObservers
    .find((observer) => observer.targets?.includes(content))
    .callback();
  flushAnimationFrames();

  assert.equal(writes.length, 2);
  await act(async () => root.unmount());
});

test("typing and editable navigation keys preserve bottom intent", async () => {
  const { content, refs, root, writes } = await mountHarness();
  refs.api.current.settle();

  window.dispatchEvent({
    altKey: false,
    ctrlKey: false,
    key: "a",
    metaKey: false,
    target: null,
    type: "keydown",
  });
  const editable = new ElementShim();
  editable.isContentEditable = true;
  window.dispatchEvent({
    altKey: false,
    ctrlKey: false,
    key: "ArrowUp",
    metaKey: false,
    target: editable,
    type: "keydown",
  });

  resizeObservers
    .find((observer) => observer.targets?.includes(content))
    .callback();
  flushAnimationFrames();
  assert.equal(writes.length, 2);
  await act(async () => root.unmount());
});
