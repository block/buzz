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
  globalThis.HTMLIFrameElement = NodeShim;
  globalThis.HTMLElement = NodeShim;
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

const PINNED_CENTER_MESSAGES = [{ id: "selected" }];
const ALIGNED_TARGET_MESSAGES = [{ id: "target" }];

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
      return { top: 0 };
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
      this.target = target;
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

function Harness({ channelId, refs }) {
  const anchoredScroll = useAnchoredScroll({
    channelId,
    contentRef: refs.content,
    isLoading: false,
    messages: PINNED_CENTER_MESSAGES,
    pinTargetCentered: true,
    scrollContainerRef: refs.container,
    targetMessageId: "selected",
  });
  refs.onScroll = anchoredScroll.onScroll;
  return null;
}

function makeAlignedTargetNodes({
  dividerContentTop = 600,
  dividerVisible = true,
  scrollHeight = 1_200,
} = {}) {
  const mutationObservers = [];
  const resizeObservers = [];
  const content = {};
  const container = {
    clientHeight: 400,
    scrollHeight,
    scrollTop: 0,
    getBoundingClientRect() {
      return { top: 0 };
    },
    querySelector(selector) {
      return selector === '[data-testid="message-unread-divider"]'
        ? dividerVisible
          ? divider
          : null
        : row;
    },
    querySelectorAll() {
      return [row];
    },
    scrollTo({ top }) {
      this.scrollTop = Math.min(
        top,
        Math.max(0, this.scrollHeight - this.clientHeight),
      );
    },
  };
  const divider = {
    getBoundingClientRect() {
      const top = dividerContentTop - container.scrollTop;
      return { bottom: top + 20, height: 20, top };
    },
  };
  const wrapper = {
    querySelector(selector) {
      return selector === '[data-testid="message-unread-divider"]'
        ? dividerVisible
          ? divider
          : null
        : null;
    },
  };
  const row = {
    dataset: { messageId: "target" },
    parentElement: wrapper,
    getBoundingClientRect() {
      const top = dividerContentTop + 20 - container.scrollTop;
      return { bottom: top + 80, height: 80, top };
    },
  };

  globalThis.ResizeObserver = class {
    constructor(callback) {
      this.callback = callback;
      resizeObservers.push(this);
    }

    disconnect() {}

    observe(target) {
      this.target = target;
    }
  };

  globalThis.MutationObserver = class {
    constructor(callback) {
      this.callback = callback;
      mutationObservers.push(this);
    }

    disconnect() {}

    observe(target, options) {
      this.options = options;
      this.target = target;
    }
  };

  return {
    container,
    content,
    mutationObservers,
    resizeObservers,
    showDivider() {
      dividerVisible = true;
    },
  };
}

function AlignedTargetHarness({
  alignment,
  messages = ALIGNED_TARGET_MESSAGES,
  onTargetReached,
  refs,
  targetMessageId = "target",
}) {
  const anchoredScroll = useAnchoredScroll({
    channelId: alignment,
    contentRef: refs.content,
    isLoading: false,
    messages,
    onTargetReached,
    scrollContainerRef: refs.container,
    targetAlignment: alignment,
    targetMessageId,
  });
  refs.onScroll = anchoredScroll.onScroll;
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
  assert.equal(nodes.resizeObservers[0].target, nodes.content);
  assert.equal(nodes.container.listeners.get("wheel")?.length, 1);

  await act(async () => {
    nodes.container.dispatchEvent({ type: "wheel" });
  });
  nodes.moveSelectedRowBy(96);
  await act(async () => {
    nodes.resizeObservers[0].callback();
  });
  assert.deepEqual(
    nodes.container.scrollWrites,
    [],
    "wheel release prevents a later resize from re-pinning the selected row",
  );

  await act(async () => {
    root.unmount();
  });
});

test("a stale scroll event from a retired conversation cannot release the new pinned target", async () => {
  const firstNodes = makePinnedCenterNodes();
  const refs = {
    container: { current: firstNodes.container },
    content: { current: firstNodes.content },
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(Harness, { channelId: "first-conversation", refs }),
    );
  });

  const secondNodes = makePinnedCenterNodes();
  refs.container.current = secondNodes.container;
  refs.content.current = secondNodes.content;
  await act(async () => {
    root.render(
      React.createElement(Harness, { channelId: "second-conversation", refs }),
    );
  });

  secondNodes.moveSelectedRowBy(24);
  await act(async () => {
    secondNodes.resizeObservers[0].callback();
  });
  assert.deepEqual(secondNodes.container.scrollWrites, [24]);
  secondNodes.container.scrollWrites.length = 0;

  await act(async () => {
    refs.onScroll({ currentTarget: firstNodes.container });
  });
  secondNodes.moveSelectedRowBy(96);
  await act(async () => {
    secondNodes.resizeObservers[0].callback();
  });
  assert.deepEqual(
    secondNodes.container.scrollWrites,
    [96],
    "the new conversation must retain its pin after the old container's delayed scroll event",
  );

  await act(async () => {
    root.unmount();
  });
});

test("bottom-aligned id target reaches and follows the physical floor", async () => {
  const nodes = makeAlignedTargetNodes();
  const refs = {
    container: { current: nodes.container },
    content: { current: nodes.content },
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(AlignedTargetHarness, {
        alignment: "bottom",
        refs,
      }),
    );
  });
  assert.equal(nodes.container.scrollTop, 800);

  nodes.container.scrollHeight = 1_600;
  await act(async () => {
    nodes.resizeObservers[0].callback();
  });
  assert.equal(
    nodes.container.scrollTop,
    1_200,
    "late content growth must keep an all-read thread at the new floor",
  );

  await act(async () => {
    root.unmount();
  });
});

test("top-with-divider target leaves the unread marker inset from the top", async () => {
  const nodes = makeAlignedTargetNodes();
  const refs = {
    container: { current: nodes.container },
    content: { current: nodes.content },
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(AlignedTargetHarness, {
        alignment: "top-with-divider",
        refs,
      }),
    );
  });
  assert.equal(
    nodes.container.scrollTop,
    588,
    "the divider at content offset 600 should retain the 12px top inset",
  );

  await act(async () => {
    root.unmount();
  });
});

test("a non-first unread target does not resolve until its divider exists", async () => {
  const nodes = makeAlignedTargetNodes({ dividerVisible: false });
  const messages = [{ id: "before" }, { id: "target" }];
  let resolvedTargets = 0;
  const onTargetReached = () => {
    resolvedTargets += 1;
  };
  const refs = {
    container: { current: nodes.container },
    content: { current: nodes.content },
  };
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(AlignedTargetHarness, {
        alignment: "top-with-divider",
        messages,
        onTargetReached,
        refs,
        targetMessageId: null,
      }),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  assert.equal(
    nodes.container.scrollTop,
    800,
    "the mount fallback may hold the floor while the expected divider is absent",
  );

  await act(async () => {
    root.render(
      React.createElement(AlignedTargetHarness, {
        alignment: "top-with-divider",
        messages,
        onTargetReached,
        refs,
      }),
    );
  });
  assert.equal(resolvedTargets, 0);
  assert.equal(nodes.container.scrollTop, 800);

  nodes.showDivider();
  await act(async () => {
    nodes.mutationObservers.at(-1).callback();
  });
  assert.equal(resolvedTargets, 1);
  assert.equal(
    nodes.container.scrollTop,
    588,
    "the same target must place the divider after it commits",
  );

  await act(async () => {
    root.unmount();
  });
});

test("a near-floor top target survives its programmatic scroll event", async () => {
  const nodes = makeAlignedTargetNodes({ dividerContentTop: 790 });
  const refs = {
    container: { current: nodes.container },
    content: { current: nodes.content },
  };
  const root = createRoot(document.createElement("div"));
  const pendingFrames = new Map();
  let nextFrameId = 1;
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
  const originalCancelAnimationFrame = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = (callback) => {
    const id = nextFrameId++;
    pendingFrames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = (id) => {
    pendingFrames.delete(id);
  };

  try {
    await act(async () => {
      root.render(
        React.createElement(AlignedTargetHarness, {
          alignment: "top-with-divider",
          refs,
        }),
      );
    });
    assert.equal(nodes.container.scrollTop, 778);
    assert.equal(pendingFrames.size, 1);

    await act(async () => {
      refs.onScroll();
      refs.onScroll();
    });
    await act(async () => {
      nodes.resizeObservers[0].callback();
    });
    assert.equal(
      nodes.container.scrollTop,
      778,
      "duplicate native events from the target's own near-floor scroll must not turn its message anchor into bottom glue",
    );

    nodes.container.scrollTop = 800;
    await act(async () => {
      refs.onScroll();
    });
    nodes.container.scrollHeight = 1_600;
    await act(async () => {
      nodes.resizeObservers[0].callback();
    });
    assert.equal(
      nodes.container.scrollTop,
      1_200,
      "a later user scroll to the floor must still restore bottom glue",
    );
  } finally {
    globalThis.requestAnimationFrame = originalRequestAnimationFrame;
    globalThis.cancelAnimationFrame = originalCancelAnimationFrame;

    await act(async () => {
      root.unmount();
    });
  }
});

test("a resolved target cancels the stale mount bottom-pin frame", async () => {
  const nodes = makeAlignedTargetNodes();
  const refs = {
    container: { current: nodes.container },
    content: { current: nodes.content },
  };
  const root = createRoot(document.createElement("div"));
  const pendingFrames = new Map();
  let nextFrameId = 1;
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
  const originalCancelAnimationFrame = globalThis.cancelAnimationFrame;
  globalThis.requestAnimationFrame = (callback) => {
    const id = nextFrameId++;
    pendingFrames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = (id) => {
    pendingFrames.delete(id);
  };

  try {
    await act(async () => {
      root.render(
        React.createElement(AlignedTargetHarness, {
          alignment: "top-with-divider",
          refs,
          targetMessageId: null,
        }),
      );
    });
    assert.equal(nodes.container.scrollTop, 800);
    assert.equal(pendingFrames.size, 1);

    await act(async () => {
      root.render(
        React.createElement(AlignedTargetHarness, {
          alignment: "top-with-divider",
          refs,
          targetMessageId: "target",
        }),
      );
    });
    assert.equal(nodes.container.scrollTop, 588);

    await act(async () => {
      refs.onScroll();
    });
    assert.equal(
      nodes.container.scrollTop,
      588,
      "the resolved target scroll must supersede the mount bottom settle guard",
    );

    for (const callback of pendingFrames.values()) callback(0);
    assert.equal(
      nodes.container.scrollTop,
      588,
      "the mount fallback must not overwrite the resolved unread target",
    );
  } finally {
    globalThis.requestAnimationFrame = originalRequestAnimationFrame;
    globalThis.cancelAnimationFrame = originalCancelAnimationFrame;
    await act(async () => {
      root.unmount();
    });
  }
});
