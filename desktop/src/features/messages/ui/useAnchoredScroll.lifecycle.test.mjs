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
  useAnchoredScroll({
    channelId,
    contentRef: refs.content,
    isLoading: false,
    messages: [{ id: "selected" }],
    pinTargetCentered: true,
    scrollContainerRef: refs.container,
    targetMessageId: "selected",
  });
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
