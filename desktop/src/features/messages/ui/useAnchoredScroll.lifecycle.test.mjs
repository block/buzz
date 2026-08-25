import assert from "node:assert/strict";
import test from "node:test";

function installDOMShim() {
  class EventTargetShim {
    constructor() {
      this.listeners = new Map();
    }

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

  class NodeShim extends EventTargetShim {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.nodeName = tagName.toUpperCase();
      this.nodeType = 1;
      this.namespaceURI = "http://www.w3.org/1999/xhtml";
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.parentNode = null;
    }

    get ownerDocument() {
      return globalThis.document;
    }

    get firstChild() {
      return this.children[0] ?? null;
    }

    get firstElementChild() {
      return this.children[0] ?? null;
    }

    get lastChild() {
      return this.children.at(-1) ?? null;
    }

    get nextSibling() {
      return null;
    }

    get nodeValue() {
      return null;
    }

    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }

    removeChild(child) {
      this.children = this.children.filter((current) => current !== child);
      this.childNodes = this.childNodes.filter((current) => current !== child);
      child.parentNode = null;
      return child;
    }

    insertBefore(child, reference) {
      if (!reference) return this.appendChild(child);
      const index = this.children.indexOf(reference);
      if (index < 0) return this.appendChild(child);
      this.children.splice(index, 0, child);
      this.childNodes.splice(index, 0, child);
      child.parentNode = this;
      return child;
    }

    contains(node) {
      return (
        this === node || this.children.some((child) => child.contains(node))
      );
    }
  }

  class DocumentShim extends EventTargetShim {
    constructor() {
      super();
      this.nodeType = 9;
      this.defaultView = globalThis;
    }

    createElement(tagName) {
      return new NodeShim(tagName);
    }

    createTextNode(value) {
      const node = new NodeShim("#text");
      node.nodeType = 3;
      node.nodeValue = value;
      return node;
    }

    createComment(value) {
      const node = new NodeShim("#comment");
      node.nodeType = 8;
      node.nodeValue = value;
      return node;
    }

    get activeElement() {
      return null;
    }
  }

  globalThis.document = new DocumentShim();
  const windowEvents = new EventTargetShim();
  globalThis.addEventListener =
    windowEvents.addEventListener.bind(windowEvents);
  globalThis.removeEventListener =
    windowEvents.removeEventListener.bind(windowEvents);
  globalThis.dispatchEvent = windowEvents.dispatchEvent.bind(windowEvents);
  globalThis.HTMLIFrameElement = NodeShim;
  globalThis.HTMLDivElement = NodeShim;
  globalThis.HTMLElement = NodeShim;
  globalThis.Node = NodeShim;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  globalThis.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.CSS = { escape: (value) => value };
}

installDOMShim();
globalThis.getComputedStyle = () => ({
  fontSize: "16px",
  getPropertyValue: () => "0px",
});

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { isTargetRowCentered } from "./targetRowCentering.ts";
import { useAnchoredScroll } from "./useAnchoredScroll.ts";
import { useVirtualizedBottomSettle } from "./useVirtualizedBottomSettle.ts";

function makePinnedCenterNodes() {
  const resizeObservers = [];
  const content = {};
  const container = {
    clientHeight: 400,
    listeners: new Map(),
    scrollHeight: 1_000,
    scrollTop: 100,
    scrollWrites: [],
    addEventListener(type, listener) {
      this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
    },
    dispatchEvent(event) {
      for (const listener of this.listeners.get(event.type) ?? [])
        listener(event);
    },
    getBoundingClientRect() {
      return { bottom: this.clientHeight, top: 0 };
    },
    querySelector() {
      return row;
    },
    querySelectorAll() {
      return [row];
    },
    removeEventListener(type, listener) {
      this.listeners.set(
        type,
        (this.listeners.get(type) ?? []).filter(
          (current) => current !== listener,
        ),
      );
    },
    scrollBy(_x, y) {
      this.scrollTop += y;
      this.scrollWrites.push(y);
    },
    scrollTo({ top }) {
      this.scrollTop = top;
    },
  };
  let contentTop = 300;
  const row = {
    dataset: { messageId: "selected" },
    getBoundingClientRect() {
      const top = contentTop - container.scrollTop;
      return { bottom: top + 40, height: 40, top };
    },
    scrollIntoView() {
      container.scrollTop = 100;
    },
  };

  globalThis.ResizeObserver = class {
    constructor(callback) {
      this.callback = callback;
      resizeObservers.push(this);
    }

    disconnect() {}

    observe(target) {
      this.targets ??= [];
      this.targets.push(target);
    }
  };

  return {
    container,
    content,
    moveSelectedRowBy: (pixels) => {
      contentTop += pixels;
    },
    resizeObservers,
  };
}

function Harness({ channelId, onTargetSettled, refs }) {
  useAnchoredScroll({
    channelId,
    contentRef: refs.content,
    isLoading: false,
    messages: [{ id: "selected" }],
    onTargetSettled,
    pinTargetCentered: true,
    scrollContainerRef: refs.container,
    targetMessageId: "selected",
  });
  return null;
}

function BottomStateHarness({
  messages,
  onState,
  refs,
  targetMessageId = null,
}) {
  const anchored = useAnchoredScroll({
    channelId: "conversation",
    contentRef: refs.content,
    isLoading: false,
    messages,
    pinTargetCentered: targetMessageId !== null,
    scrollContainerRef: refs.container,
    targetMessageId,
  });
  onState(anchored);
  return null;
}

function VirtualScrollBehaviorHarness({
  messages = [{ id: "selected" }],
  refs,
}) {
  const lastRunMessageCount = React.useRef(null);
  const anchored = useAnchoredScroll({
    channelId: "conversation",
    contentRef: refs.content,
    isLoading: false,
    messages,
    scrollContainerRef: refs.scroller,
    virtualizerOwnsPrependAnchoring: true,
    virtualScrollBy: (offset) => {
      refs.scrollOffsets.push(offset);
      refs.rowTop -= offset;
    },
    virtualScrollToMessage: (messageId, options) => {
      refs.targetJumps.push({ messageId, options });
      return true;
    },
  });
  React.useLayoutEffect(() => {
    if (lastRunMessageCount.current === messages.length) return;
    lastRunMessageCount.current = messages.length;
    refs.targetResult = anchored.scrollToMessage("selected", {
      behavior: "smooth",
    });
  }, [anchored.scrollToMessage, messages.length, refs]);
  return null;
}

function VirtualTargetHarness({ refs }) {
  const didRun = React.useRef(false);
  const bottomApi = useVirtualizedBottomSettle(
    refs.host,
    refs.list,
    refs.itemsLength,
  );
  const anchored = useAnchoredScroll({
    channelId: "conversation",
    contentRef: refs.content,
    isLoading: false,
    messages: Array.from({ length: 5 }, (_, index) => ({ id: String(index) })),
    scrollContainerRef: refs.scroller,
    virtualCancelBottomIntent: bottomApi.cancel,
    virtualizerOwnsPrependAnchoring: true,
    virtualScrollToMessage: (messageId) => {
      refs.targetJumps.current.push(messageId);
      return true;
    },
  });
  React.useLayoutEffect(() => {
    if (didRun.current) return;
    didRun.current = true;
    bottomApi.settle();
    refs.targetResult.current = anchored.scrollToMessage("selected");
  }, [anchored.scrollToMessage, bottomApi.settle, refs.targetResult]);
  return null;
}

test("channel change attaches pinned-center observers after refs mount", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(React.createElement(Harness, { channelId: null, refs }));
  });

  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;

  await act(async () => {
    root.render(
      React.createElement(Harness, { channelId: "conversation", refs }),
    );
  });

  assert.equal(nodes.resizeObservers.length, 1);
  assert.deepEqual(nodes.resizeObservers[0].targets, [
    nodes.content,
    nodes.container,
  ]);
  assert.equal(nodes.container.listeners.get("wheel")?.length, 1);

  await act(async () => {
    nodes.container.dispatchEvent({ type: "wheel" });
  });
  nodes.moveSelectedRowBy(96);
  nodes.resizeObservers[0].callback();
  assert.deepEqual(
    nodes.container.scrollWrites,
    [],
    "wheel release prevents a later resize from re-pinning the selected row",
  );

  await act(async () => {
    root.unmount();
  });
});

test("arrival at the physical floor does not preserve a stale unread state", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;
  let state = null;
  const render = (messages) =>
    root.render(
      React.createElement(BottomStateHarness, {
        messages,
        onState: (nextState) => {
          state = nextState;
        },
        refs,
      }),
    );

  await act(async () => render([{ id: "first" }]));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  assert.equal(state.isAtBottom, false);

  // Native anchoring can return the viewport to the floor without a scroll or
  // resize callback, leaving only the hook's cached message anchor stale.
  nodes.container.scrollTop =
    nodes.container.scrollHeight - nodes.container.clientHeight;
  await act(async () => render([{ id: "first" }, { id: "second" }]));

  assert.equal(state.isAtBottom, true);
  assert.equal(state.newMessageCount, 0);
  await act(async () => root.unmount());
});

test("arrival does not steal an active layout target during floor-like reflow", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;
  let state = null;
  const render = (messages, targetMessageId = null) =>
    root.render(
      React.createElement(BottomStateHarness, {
        messages,
        onState: (nextState) => {
          state = nextState;
        },
        refs,
        targetMessageId,
      }),
    );

  await act(async () => render([{ id: "selected" }]));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  assert.equal(state.isAtBottom, false);

  // A focus/split presentation switch can commit fresh replies while the old
  // container geometry momentarily reads as the physical floor. The explicit
  // layout target must win so the reading row is restored after reflow.
  nodes.container.scrollTop =
    nodes.container.scrollHeight - nodes.container.clientHeight;
  await act(async () =>
    render([{ id: "selected" }, { id: "second" }], "selected"),
  );

  assert.equal(state.isAtBottom, false);
  assert.equal(state.newMessageCount, 1);
  await act(async () => root.unmount());
});

test("container resize clears a stale new-message state at the physical floor", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;
  let state = null;
  const render = (messages) =>
    root.render(
      React.createElement(BottomStateHarness, {
        messages,
        onState: (nextState) => {
          state = nextState;
        },
        refs,
      }),
    );

  await act(async () => render([{ id: "first" }]));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  nodes.container.scrollTop = 100;
  await act(async () => state.onScroll());
  await act(async () => render([{ id: "first" }, { id: "second" }]));
  assert.equal(state.isAtBottom, false);
  assert.equal(state.newMessageCount, 1);

  // A taller viewport reaches the floor without producing a native scroll.
  nodes.container.clientHeight = 900;
  await act(async () => nodes.resizeObservers[0].callback());

  assert.equal(state.isAtBottom, true);
  assert.equal(state.newMessageCount, 0);
  await act(async () => root.unmount());
});

test("pinned target resize reconciles bottom state before retiring", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  const settled = [];
  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;

  await act(async () => {
    root.render(
      React.createElement(Harness, {
        channelId: "conversation",
        onTargetSettled: (messageId) => settled.push(messageId),
        refs,
      }),
    );
  });

  assert.deepEqual(settled, [], "initial target reach is not settled");
  nodes.moveSelectedRowBy(96);
  nodes.resizeObservers[0].callback();
  assert.deepEqual(settled, [], "resize callback waits for the paint frame");
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));

  assert.deepEqual(settled, ["selected"]);
  assert.deepEqual(nodes.container.scrollWrites, [96]);
  await act(async () => root.unmount());
});

test("user interaction releases and retires a pending pinned target", async () => {
  const refs = {
    container: { current: null },
    content: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  const settled = [];
  const nodes = makePinnedCenterNodes();
  refs.container.current = nodes.container;
  refs.content.current = nodes.content;

  await act(async () => {
    root.render(
      React.createElement(Harness, {
        channelId: "conversation",
        onTargetSettled: (messageId) => settled.push(messageId),
        refs,
      }),
    );
  });
  await act(async () => nodes.container.dispatchEvent({ type: "wheel" }));
  nodes.moveSelectedRowBy(96);
  nodes.resizeObservers[0].callback();
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));

  assert.deepEqual(settled, ["selected"]);
  assert.deepEqual(nodes.container.scrollWrites, []);
  await act(async () => root.unmount());
});

test("boundary-clamped targets settle only at their matching physical edge", () => {
  const container = document.createElement("div");
  container.scrollTop = 0;
  container.getBoundingClientRect = () => ({ bottom: 400, top: 0 });
  const row = {
    getBoundingClientRect: () => ({ bottom: 40, height: 40, top: 0 }),
  };
  const isAtBottom = () => false;

  assert.equal(isTargetRowCentered(row, container, "top", isAtBottom), true);
  assert.equal(isTargetRowCentered(row, container, "none", isAtBottom), false);
  assert.equal(
    isTargetRowCentered(row, container, "bottom", isAtBottom),
    false,
  );

  row.getBoundingClientRect = () => ({
    bottom: 2_740,
    height: 40,
    top: 2_700,
  });
  assert.equal(
    isTargetRowCentered(row, container, "top", isAtBottom),
    false,
    "an unrendered indexed jump can report scrollTop zero before the row arrives",
  );

  row.getBoundingClientRect = () => ({ bottom: 40, height: 40, top: 0 });
  container.scrollTop = 1;
  assert.equal(isTargetRowCentered(row, container, "top", isAtBottom), false);
  assert.equal(
    isTargetRowCentered(row, container, "bottom", () => true),
    true,
  );
});

test("virtual search scrolling stays smooth only for an already-rendered target", async () => {
  for (const rendered of [true, false]) {
    const content = document.createElement("div");
    const scroller = document.createElement("div");
    scroller.clientHeight = 400;
    scroller.scrollHeight = 1_000;
    scroller.scrollTop = 0;
    scroller.getBoundingClientRect = () => ({ bottom: 400, top: 0 });
    const row = {
      getBoundingClientRect: () => ({
        bottom: refs.rowTop + 40,
        height: 40,
        top: refs.rowTop,
      }),
    };
    scroller.querySelector = () => (rendered ? row : null);
    scroller.querySelectorAll = () => [];
    scroller.appendChild(content);
    const refs = {
      content: { current: content },
      scroller: { current: scroller },
      rowTop: rendered ? 180 : 1_000,
      scrollOffsets: [],
      targetJumps: [],
      targetResult: null,
    };
    const root = createRoot(document.createElement("div"));

    await act(async () => {
      root.render(React.createElement(VirtualScrollBehaviorHarness, { refs }));
    });

    assert.deepEqual(refs.targetJumps, [
      {
        messageId: "selected",
        options: { behavior: rendered ? "smooth" : "auto" },
      },
    ]);
    assert.equal(refs.targetResult, rendered ? "centered" : "pending");
    await act(async () => root.unmount());
  }
});

test("a pending virtual jump is retried when the indexed message model grows", async () => {
  const content = document.createElement("div");
  const scroller = document.createElement("div");
  scroller.clientHeight = 400;
  scroller.scrollHeight = 1_000;
  scroller.scrollTop = 0;
  scroller.getBoundingClientRect = () => ({ bottom: 400, top: 0 });
  scroller.querySelector = () => null;
  scroller.querySelectorAll = () => [];
  scroller.appendChild(content);
  const refs = {
    content: { current: content },
    scroller: { current: scroller },
    rowTop: 1_000,
    scrollOffsets: [],
    targetJumps: [],
    targetResult: null,
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(VirtualScrollBehaviorHarness, {
        messages: [{ id: "selected" }],
        refs,
      }),
    );
  });
  await act(async () => {
    root.render(
      React.createElement(VirtualScrollBehaviorHarness, {
        messages: [{ id: "selected" }, { id: "later" }],
        refs,
      }),
    );
  });

  assert.equal(refs.targetJumps.length, 2);
  assert.deepEqual(
    refs.targetJumps.map(({ messageId }) => messageId),
    ["selected", "selected"],
  );
  await act(async () => root.unmount());
});

test("virtual centering corrects rendered geometry on the following frame", async () => {
  const previousGetComputedStyle = globalThis.getComputedStyle;
  globalThis.getComputedStyle = (element) => ({
    fontSize: "16px",
    getPropertyValue: (name) =>
      element === document.documentElement
        ? "0px"
        : name === "--composer-overlay-height"
          ? "20px"
          : "0px",
  });
  const content = document.createElement("div");
  const scroller = document.createElement("div");
  scroller.clientHeight = 400;
  scroller.scrollHeight = 1_000;
  scroller.scrollTop = 0;
  scroller.getBoundingClientRect = () => ({ bottom: 400, top: 0 });
  const refs = {
    content: { current: content },
    scroller: { current: scroller },
    rowTop: 180,
    scrollOffsets: [],
    targetJumps: [],
    targetResult: null,
  };
  const row = {
    getBoundingClientRect: () => ({
      bottom: refs.rowTop + 40,
      height: 40,
      top: refs.rowTop,
    }),
  };
  scroller.querySelector = () => row;
  scroller.querySelectorAll = () => [];
  scroller.appendChild(content);
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(React.createElement(VirtualScrollBehaviorHarness, { refs }));
  });
  assert.equal(refs.targetResult, "pending");
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  assert.deepEqual(refs.scrollOffsets, [10]);
  assert.equal(refs.rowTop, 170);

  await act(async () => root.unmount());
  globalThis.getComputedStyle = previousGetComputedStyle;
});

test("mounted virtual target retires bottom intent and delegates the jump to the virtualizer", async () => {
  const resizeObservers = [];
  globalThis.ResizeObserver = class {
    constructor(callback) {
      this.callback = callback;
      this.targets = [];
      resizeObservers.push(this);
    }
    disconnect() {}
    observe(target) {
      this.targets.push(target);
    }
  };

  const content = document.createElement("div");
  const scroller = document.createElement("div");
  const host = document.createElement("div");
  scroller.appendChild(content);
  host.appendChild(scroller);
  scroller.clientHeight = 400;
  scroller.scrollHeight = 1_000;
  scroller.scrollTop = 0;
  scroller.getBoundingClientRect = () => ({ bottom: 400, top: 0 });
  const targetContentTop = 180;
  const row = {
    getBoundingClientRect: () => ({
      bottom: targetContentTop - scroller.scrollTop + 40,
      height: 40,
      top: targetContentTop - scroller.scrollTop,
    }),
  };
  scroller.querySelector = () => row;
  scroller.querySelectorAll = () => [];
  const directScrollWrites = [];
  scroller.scrollTo = ({ top }) => {
    directScrollWrites.push(top);
    scroller.scrollTop = top;
  };

  const bottomWrites = [];
  const refs = {
    content: { current: content },
    host: { current: host },
    itemsLength: { current: 5 },
    list: {
      current: {
        scrollToIndex: (index, options) =>
          bottomWrites.push({ index, options }),
      },
    },
    scroller: { current: scroller },
    targetJumps: { current: [] },
    targetResult: { current: null },
  };
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(React.createElement(VirtualTargetHarness, { refs }));
  });

  assert.deepEqual(bottomWrites, [{ index: 4, options: { align: "end" } }]);
  // The virtualizer is the only scroll writer: the hook hands it the jump and
  // never races it with a direct `scrollTo` that its in-flight correction
  // would overwrite. The row is already settled in view, so it is handled.
  assert.deepEqual(refs.targetJumps.current, ["selected"]);
  assert.deepEqual(directScrollWrites, []);
  assert.equal(refs.targetResult.current, "centered");
  const bottomGeometryObserver = resizeObservers.find(
    (observer) =>
      observer.targets.includes(content) && observer.targets.includes(scroller),
  );
  assert.ok(bottomGeometryObserver);
  bottomGeometryObserver.callback();
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));

  assert.equal(bottomWrites.length, 1, "geometry cannot re-pin to bottom");
  await act(async () => root.unmount());
});
