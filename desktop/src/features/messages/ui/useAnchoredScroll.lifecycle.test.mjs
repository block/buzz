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

    closest() {
      return null;
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

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

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
    virtualScrollToMessage: (messageId, options) => {
      refs.targetScrolls.push({ messageId, options });
      return refs.targetResolves;
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

async function renderVirtualTarget({ targetResolves }) {
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
  const targetContentTop = 250;
  const row = {
    getBoundingClientRect: () => ({
      bottom: targetContentTop - scroller.scrollTop + 40,
      height: 40,
      top: targetContentTop - scroller.scrollTop,
    }),
  };
  scroller.querySelector = () => row;
  scroller.querySelectorAll = () => [];
  scroller.scrollTo = ({ top }) => {
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
    targetResolves,
    targetResult: { current: null },
    targetScrolls: [],
  };
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(React.createElement(VirtualTargetHarness, { refs }));
  });

  assert.deepEqual(bottomWrites, [{ index: 4, options: { align: "end" } }]);
  const settleBottomGeometry = async () => {
    const bottomGeometryObserver = resizeObservers.find(
      (observer) =>
        observer.targets.includes(content) &&
        observer.targets.includes(scroller),
    );
    assert.ok(bottomGeometryObserver);
    bottomGeometryObserver.callback();
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  };
  return { bottomWrites, refs, root, scroller, settleBottomGeometry };
}

test("mounted virtual target retires bottom intent and scrolls through the virtualizer", async () => {
  const { bottomWrites, refs, root, scroller, settleBottomGeometry } =
    await renderVirtualTarget({ targetResolves: true });

  assert.equal(refs.targetResult.current, true);
  // Even an on-screen row goes through the virtualizer: a direct DOM scroll
  // would not supersede a bottom pin the virtualizer still has in flight.
  assert.deepEqual(refs.targetScrolls, [
    { messageId: "selected", options: { behavior: "auto" } },
  ]);
  assert.equal(scroller.scrollTop, 0, "no direct DOM centering write");
  await settleBottomGeometry();

  assert.equal(bottomWrites.length, 1, "geometry cannot re-pin to bottom");
  await act(async () => root.unmount());
});

test("unresolved virtual target keeps the bottom pin", async () => {
  const { bottomWrites, refs, root, settleBottomGeometry } =
    await renderVirtualTarget({ targetResolves: false });

  // A target that is not loaded yet retries on later commits. Until it
  // resolves, the channel must keep following its floor.
  assert.equal(refs.targetResult.current, false);
  await settleBottomGeometry();

  assert.equal(bottomWrites.length, 2, "geometry still re-pins to bottom");
  await act(async () => root.unmount());
});

test("target arriving before the second mount frame supersedes the bottom pin", async () => {
  const nodes = makePinnedCenterNodes();
  nodes.container.querySelector = () => null;
  const frames = new Map();
  let nextFrameId = 0;
  const previousRaf = globalThis.requestAnimationFrame;
  const previousCancelRaf = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = (callback) => {
    nextFrameId += 1;
    frames.set(nextFrameId, callback);
    return nextFrameId;
  };
  globalThis.cancelAnimationFrame = (id) => frames.delete(id);
  const writes = [];
  let targetLoaded = false;
  const scrollContainerRef = { current: nodes.container };
  const contentRef = { current: nodes.content };
  function RaceHarness({ messages }) {
    useAnchoredScroll({
      channelId: "conversation",
      contentRef,
      isLoading: false,
      messages,
      scrollContainerRef,
      targetMessageId: "selected",
      virtualizerOwnsPrependAnchoring: true,
      virtualScrollToBottom: () => writes.push("bottom"),
      virtualScrollToMessage: () => {
        if (!targetLoaded) return false;
        writes.push("target");
        return true;
      },
    });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    // The target is not loaded on mount, so the hook pins to bottom now and
    // queues a second pin for the next frame.
    await act(async () =>
      root.render(
        React.createElement(RaceHarness, { messages: [{ id: "tail" }] }),
      ),
    );
    // The target lands before that frame runs.
    targetLoaded = true;
    await act(async () =>
      root.render(
        React.createElement(RaceHarness, {
          messages: [{ id: "selected" }, { id: "tail" }],
        }),
      ),
    );
    await act(async () => {
      for (const [id, callback] of [...frames]) {
        frames.delete(id);
        callback();
      }
    });

    assert.equal(writes.at(-1), "target", JSON.stringify(writes));
  } finally {
    await act(async () => root.unmount());
    globalThis.requestAnimationFrame = previousRaf;
    globalThis.cancelAnimationFrame = previousCancelRaf;
  }
});

function TargetHoldHarness({ fake, onState, ...props }) {
  onState(
    useAnchoredScroll({
      channelId: "conversation",
      contentRef: fake.content,
      isLoading: false,
      messages: fake.messages,
      scrollContainerRef: fake.scroller,
      virtualizerOwnsPrependAnchoring: true,
      virtualScrollToBottom: fake.scrollToBottom,
      virtualScrollToMessage: fake.scrollToMessage,
      virtualSettleAtBottom: fake.settleAtBottom,
      ...props,
    }),
  );
  return null;
}

// Accepts a virtual route target, then replays the layout that carried it
// away in CI: the route clears the target, the thread panel opens on it, and a
// later layout pass reports the virtualizer at its floor. Each reflow
// re-centres the held target rather than settling at the bottom.
async function renderHeldTarget() {
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
  const scroller = document.createElement("div");
  scroller.querySelector = () => null;
  const settles = [];
  const targetScrolls = [];
  const fake = {
    content: { current: null },
    messages: [{ id: "selected" }, { id: "latest" }],
    scrollToBottom: () => settles.push("jump"),
    scrollToMessage: (messageId) => {
      targetScrolls.push(messageId);
      return true;
    },
    scroller: { current: scroller },
    settleAtBottom: () => settles.push("settle"),
  };
  let state = null;
  let props = { splitPanelOpen: false, targetMessageId: "selected" };
  const root = createRoot(document.createElement("div"));
  const render = async (nextProps) => {
    props = { ...props, ...nextProps };
    await act(async () =>
      root.render(
        React.createElement(TargetHoldHarness, {
          fake,
          onState: (next) => {
            state = next;
          },
          ...props,
        }),
      ),
    );
  };
  const resizeViewport = async () => {
    await act(async () => {
      for (const observer of resizeObservers) {
        if (observer.targets.includes(scroller)) observer.callback();
      }
    });
  };

  await render({});
  assert.equal(state.isAtBottom, false, "the target is accepted");
  assert.deepEqual(targetScrolls, ["selected"]);
  await render({ splitPanelOpen: true, targetMessageId: null });
  assert.equal(targetScrolls.length, 2, "the panel opening re-centres it");
  await act(async () => state.onVirtualizerAtBottomStateChange(true));
  await resizeViewport();
  assert.deepEqual(settles, [], "layout cannot settle a held target");
  assert.deepEqual(
    targetScrolls,
    ["selected", "selected", "selected"],
    "viewport reflow re-centres the held target",
  );
  return {
    fake,
    render,
    resizeViewport,
    root,
    scroller,
    settles,
    state: () => state,
    targetScrolls,
  };
}

test("an accepted virtual target holds through the thread panel opening and floor reports", async () => {
  const {
    fake,
    render,
    resizeViewport,
    root,
    scroller,
    settles,
    state,
    targetScrolls,
  } = await renderHeldTarget();

  // The floor report still drives the affordances.
  assert.equal(state().isAtBottom, true);
  fake.messages = [...fake.messages, { id: "live" }];
  await render({});
  assert.deepEqual(settles, [], "a live append cannot settle a held target");
  // The append lands below the held row without a scroll event, so the hook
  // itself must bring back the Jump to latest / new-message pill.
  assert.equal(state().isAtBottom, false, "the pill shows again");
  assert.equal(state().newMessageCount, 1);
  // The reader is no longer at the floor, so taking the viewport back must
  // not snap past the new row on the next reflow.
  await act(async () =>
    scroller.dispatchEvent({ target: scroller, type: "pointerdown" }),
  );
  await resizeViewport();
  assert.deepEqual(settles, [], "release after the append stays put");
  assert.equal(targetScrolls.length, 3, "and is not re-centred");
  await act(async () => root.unmount());
});

const RELEASES = [
  ["wheel", ({ scroller }) => scroller.dispatchEvent({ type: "wheel" })],
  [
    "touch drag",
    ({ scroller }) => scroller.dispatchEvent({ type: "touchmove" }),
  ],
  [
    "scrollbar pointerdown",
    ({ scroller }) =>
      scroller.dispatchEvent({ target: scroller, type: "pointerdown" }),
  ],
  [
    "scroll key",
    ({ scroller }) =>
      globalThis.dispatchEvent({
        altKey: false,
        ctrlKey: false,
        key: "PageUp",
        metaKey: false,
        target: scroller,
        type: "keydown",
      }),
  ],
  ["Jump to latest", ({ state }) => state().scrollToBottom("auto")],
  [
    "own send",
    async ({ fake, render, state }) => {
      state().scrollToBottomOnNextUpdate();
      fake.messages = [...fake.messages, { id: "sent" }];
      await render({});
    },
  ],
  [
    "channel change",
    ({ render }) => render({ channelId: "other", isLoading: true }),
  ],
  // Find-in-channel and the oldest-unread pill jump through the public
  // scrollToMessage. Neither holds, so the next floor report follows again.
  [
    "find jump",
    ({ state }) => {
      state().scrollToMessage("selected", {
        behavior: "smooth",
        highlight: true,
      });
      state().onVirtualizerAtBottomStateChange(true);
    },
  ],
];

for (const [name, release] of RELEASES) {
  test(`${name} releases a held virtual target`, async () => {
    const held = await renderHeldTarget();
    await act(async () => release(held));
    const before = held.settles.filter((kind) => kind === "settle").length;
    const targetScrollsBefore = held.targetScrolls.length;
    await held.resizeViewport();

    assert.equal(
      held.settles.filter((kind) => kind === "settle").length,
      before + 1,
      "the floor follows viewport reflow again",
    );
    assert.equal(
      held.targetScrolls.length,
      targetScrollsBefore,
      "a released target is not re-centred",
    );
    await act(async () => held.root.unmount());
  });
}

test("a descendant pointerdown keeps a virtual target held", async () => {
  const held = await renderHeldTarget();
  const row = document.createElement("div");
  held.scroller.appendChild(row);
  await act(async () =>
    held.scroller.dispatchEvent({ target: row, type: "pointerdown" }),
  );
  await held.resizeViewport();

  assert.deepEqual(held.settles, []);
  assert.equal(held.targetScrolls.length, 4, "the target is still re-centred");
  await act(async () => held.root.unmount());
});

test("history filling the gap under a held target is not counted as new", async () => {
  const held = await renderHeldTarget();
  // A cold target sits older than the head window. The page that fills the
  // gap between them keeps both ends of the list, so nothing arrived below.
  held.fake.messages = [
    { id: "selected" },
    { id: "gap-2" },
    { id: "gap-3" },
    { id: "latest" },
  ];
  await held.render({});
  assert.equal(held.state().isAtBottom, true, "the floor report stands");
  assert.equal(held.state().newMessageCount, 0);

  // Deleting the newest row changes the last id, but nothing is new.
  held.fake.messages = held.fake.messages.slice(0, -1);
  await held.render({});
  assert.equal(held.state().isAtBottom, true, "a deletion raises no pill");
  assert.equal(held.state().newMessageCount, 0);

  // More history and a live row in one commit: only the live row is new.
  held.fake.messages = [
    { id: "selected" },
    { id: "gap-1" },
    { id: "gap-2" },
    { id: "gap-3" },
    { id: "live" },
  ];
  await held.render({});
  assert.equal(held.state().isAtBottom, false);
  assert.equal(held.state().newMessageCount, 1);
  assert.deepEqual(held.settles, []);
  await act(async () => held.root.unmount());
});

test("reader input at the floor resumes following live messages", async () => {
  const held = await renderHeldTarget();
  // A wheel at the floor moves nothing, so no new floor report follows it.
  await act(async () => held.scroller.dispatchEvent({ type: "wheel" }));
  held.fake.messages = [...held.fake.messages, { id: "live" }];
  await held.render({});

  assert.deepEqual(held.settles, ["settle"]);
  await act(async () => held.root.unmount());
});
