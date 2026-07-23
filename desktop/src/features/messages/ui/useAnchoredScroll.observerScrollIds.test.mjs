/**
 * Hook-level regression tests for the observer feed scroll-id wiring.
 *
 * Corrective actions addressed:
 *  2. Zero-write assertions: same ids at bottom/mid-history → no React-effect
 *     scrollTo; new id at bottom → one floor write; new id mid-history → no floor.
 *  4. Mode-toggle reset: changing channelId (which encodes the mode) resets the
 *     hook and re-pins in both loaded and connecting→loaded states.
 *
 * Uses the same DOM shim and harness pattern as the adjacent
 * useAnchoredScroll.observerFeedMountPin.test.mjs.
 */

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
        (this.listeners.get(type) ?? []).filter((l) => l !== listener),
      );
    }
    dispatchEvent(event) {
      for (const l of this.listeners.get(event.type) ?? []) l(event);
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
      this.children = this.children.filter((c) => c !== child);
      this.childNodes = this.childNodes.filter((c) => c !== child);
      child.parentNode = null;
      return child;
    }
    insertBefore(child, ref) {
      if (!ref) return this.appendChild(child);
      const idx = this.children.indexOf(ref);
      if (idx < 0) return this.appendChild(child);
      this.children.splice(idx, 0, child);
      this.childNodes.splice(idx, 0, child);
      child.parentNode = this;
      return child;
    }
    contains(node) {
      return this === node || this.children.some((c) => c.contains(node));
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
  globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.CSS = { escape: (v) => v };
  globalThis.ResizeObserver = class {
    observe() {}
    disconnect() {}
  };
}

installDOMShim();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useAnchoredScroll } from "./useAnchoredScroll.ts";

/**
 * Scroll container shim that tracks scrollTo writes.
 * The `scrollTo` spy is the critical probe: it captures every React-effect
 * floor write the hook performs.
 */
function makeContainer({ clientHeight, scrollHeight, scrollTop = 0 }) {
  const writes = [];
  return {
    clientHeight,
    scrollHeight,
    scrollTop,
    writes,
    getBoundingClientRect() {
      return { top: 0 };
    },
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    scrollTo({ top, behavior }) {
      writes.push({ top, behavior });
      this.scrollTop = top;
    },
  };
}

/**
 * Harness wiring: mirrors AgentSessionThreadPanel's exact hook call —
 * bottom-tail only (no targetMessageId, pinTargetCentered omitted/false),
 * isLoading derived from the observer store's connection state.
 *
 * Exposes `onScroll` via a ref so tests can simulate user scroll events
 * (required to transition the anchor from at-bottom to mid-history).
 */
function ObserverFeedHarness({
  channelId,
  isLoading,
  messages,
  onScrollRef,
  refs,
}) {
  const { onScroll } = useAnchoredScroll({
    channelId,
    contentRef: refs.content,
    isLoading,
    messages,
    scrollContainerRef: refs.container,
  });
  if (onScrollRef) onScrollRef.current = onScroll;
  return null;
}

/** Flush pending rAF-deferred work. */
async function flushRaf() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

// ── Corrective action 2: hook-level zero-write assertions ───────────────────

test("at-bottom: same message ids → zero scrollTo writes after initial pin", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 1000,
    scrollTop: 0,
  });
  refs.container.current = container;

  const messages = [{ id: "turn:t1" }, { id: "turn:t2" }];

  // Mount → initial pin to bottom.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages,
        refs,
      }),
    );
  });
  await flushRaf();

  // Record the write count after initial pin.
  const writesAfterMount = container.writes.length;
  assert.ok(writesAfterMount > 0, "initial mount must pin to bottom");
  assert.equal(
    container.scrollTop,
    container.scrollHeight,
    "must be at the bottom after mount",
  );

  // Re-render with the SAME messages reference — simulates raw events
  // arriving that do not change the stabilized block-id array.
  // The hook should NOT write scrollTo again.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages, // same reference
        refs,
      }),
    );
  });
  await flushRaf();

  assert.equal(
    container.writes.length,
    writesAfterMount,
    "same messages reference at bottom → zero additional scrollTo writes " +
      "(this is the regression the fix prevents: without useStableArrayShallow, " +
      "every raw event would produce a new reference and fire scrollTo)",
  );

  await act(async () => {
    root.unmount();
  });
});

test("mid-history: same message ids → zero scrollTo writes", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const onScrollRef = { current: null };
  const root = createRoot(document.createElement("div"));

  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 2000,
    scrollTop: 0,
  });
  // Fake [data-message-id] row so computeAnchor can find a mid-history anchor.
  const fakeRow = {
    dataset: { messageId: "turn:t1" },
    getBoundingClientRect() {
      return { top: 100, bottom: 140, height: 40 };
    },
  };
  container.querySelectorAll = () => [fakeRow];
  refs.container.current = container;

  const messages = [{ id: "turn:t1" }, { id: "turn:t2" }, { id: "turn:t3" }];

  // Mount and pin to bottom.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages,
        onScrollRef,
        refs,
      }),
    );
  });
  await flushRaf();

  // Latch mid-history: double onScroll to clear the settle guard + latch anchor.
  container.scrollTop = 500;
  await act(async () => {
    onScrollRef.current?.();
  });
  container.scrollTop = 500;
  await act(async () => {
    onScrollRef.current?.();
  });
  await flushRaf();

  const writesAfterLatch = container.writes.length;

  // Re-render with the SAME messages reference while genuinely mid-history.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages, // same reference
        onScrollRef,
        refs,
      }),
    );
  });
  await flushRaf();

  assert.equal(
    container.writes.length,
    writesAfterLatch,
    "same messages reference mid-history → zero scrollTo writes (no yank)",
  );

  await act(async () => {
    root.unmount();
  });
});

test("at-bottom: new message id → one floor write", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 1000,
    scrollTop: 0,
  });
  refs.container.current = container;

  const messages1 = [{ id: "turn:t1" }];

  // Mount → pin to bottom.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages: messages1,
        refs,
      }),
    );
  });
  await flushRaf();

  const writesAfterMount = container.writes.length;
  assert.equal(container.scrollTop, container.scrollHeight);

  // New block arrives — new messages reference with an additional id.
  container.scrollHeight = 1400;
  const messages2 = [{ id: "turn:t1" }, { id: "turn:t2" }];

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages: messages2,
        refs,
      }),
    );
  });
  await flushRaf();

  assert.equal(
    container.writes.length,
    writesAfterMount + 1,
    "new block id at bottom → exactly one additional floor write",
  );
  assert.equal(container.scrollTop, 1400, "must be pinned to the new floor");

  await act(async () => {
    root.unmount();
  });
});

test("mid-history: new message id → no floor write (unread path)", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const onScrollRef = { current: null };
  const root = createRoot(document.createElement("div"));

  // Container with fake [data-message-id] rows so computeAnchor can find
  // a mid-history anchor instead of falling through to at-bottom.
  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 2000,
    scrollTop: 0,
  });
  // Add querySelectorAll that returns rows with data-message-id.
  const fakeRow = {
    dataset: { messageId: "turn:t1" },
    getBoundingClientRect() {
      // Position the row inside the visible area when scrollTop=500.
      return { top: 100, bottom: 140, height: 40 };
    },
  };
  container.querySelectorAll = () => [fakeRow];
  refs.container.current = container;

  const messages1 = [{ id: "turn:t1" }];

  // Mount → pin to bottom.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages: messages1,
        onScrollRef,
        refs,
      }),
    );
  });
  await flushRaf();

  // Simulate user scrolling up: move scrollTop, then call the hook's onScroll.
  // The first onScroll call after mount hits the settling guard (the mount pin
  // arms settling). That guard calls scrollTo(scrollHeight) and clears settling,
  // then returns early. We must call onScroll a second time after re-setting
  // scrollTop so computeAnchor can run and latch a mid-history anchor.
  container.scrollTop = 500;
  await act(async () => {
    onScrollRef.current?.(); // clears settling, but scrollTo moves us back to bottom
  });
  container.scrollTop = 500; // user scrolls up again after settling
  await act(async () => {
    onScrollRef.current?.(); // now computeAnchor runs, finds fakeRow → mid-history
  });
  await flushRaf();

  const scrollTopBeforeNewBlock = container.scrollTop;

  // New block arrives while mid-history.
  container.scrollHeight = 2400;
  const messages2 = [{ id: "turn:t1" }, { id: "turn:t2" }];

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages: messages2,
        onScrollRef,
        refs,
      }),
    );
  });
  await flushRaf();

  // scrollTop must not have been changed by the hook — no yank to bottom.
  assert.equal(
    container.scrollTop,
    scrollTopBeforeNewBlock,
    "new block mid-history must NOT yank to bottom",
  );

  await act(async () => {
    root.unmount();
  });
});

// ── Corrective action 4: mode-toggle reset + re-pin ─────────────────────────

test("mode toggle: changing channelId resets and re-pins to bottom (loaded state)", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 1000,
    scrollTop: 0,
  });
  refs.container.current = container;

  // Mount in transcript mode.
  const transcriptMessages = [{ id: "turn:t1" }, { id: "turn:t2" }];
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: false,
        messages: transcriptMessages,
        refs,
      }),
    );
  });
  await flushRaf();
  assert.equal(container.scrollTop, container.scrollHeight, "initial pin");

  // Simulate user scrolling up (mid-history in transcript mode).
  container.scrollTop = 200;

  // Toggle to raw mode — channelId changes, forces hook reset.
  container.scrollHeight = 2000;
  const rawMessages = [{ id: "1:ts1" }, { id: "2:ts2" }, { id: "3:ts3" }];
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:raw",
        isLoading: false,
        messages: rawMessages,
        refs,
      }),
    );
  });
  await flushRaf();

  assert.equal(
    container.scrollTop,
    container.scrollHeight,
    "mode toggle must reset the anchor and re-pin to bottom",
  );

  await act(async () => {
    root.unmount();
  });
});

test("mode toggle: connecting → loaded re-pins to bottom after reset", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  const container = makeContainer({
    clientHeight: 400,
    scrollHeight: 0,
    scrollTop: 0,
  });
  refs.container.current = container;

  // Mount in transcript mode, loading (connecting).
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:transcript",
        isLoading: true,
        messages: [],
        refs,
      }),
    );
  });
  await flushRaf();

  // Still connecting — no pin yet.
  assert.equal(container.scrollTop, 0, "no pin while loading");

  // Toggle mode while still loading.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:raw",
        isLoading: true,
        messages: [],
        refs,
      }),
    );
  });
  await flushRaf();

  // Connection resolves: content arrives and loading clears.
  container.scrollHeight = 1500;
  container.clientHeight = 400;
  const rawMessages = [{ id: "1:ts1" }, { id: "2:ts2" }];

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        channelId: "agent:chan:raw",
        isLoading: false,
        messages: rawMessages,
        refs,
      }),
    );
  });
  await flushRaf();

  assert.equal(
    container.scrollTop,
    container.scrollHeight,
    "after mode toggle + loading clears, must pin to bottom",
  );

  await act(async () => {
    root.unmount();
  });
});
