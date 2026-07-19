/**
 * Regression test: the observer feed's bottom-tail auto-pin must fire once
 * content has actually rendered, not merely once at mount.
 *
 * ── Bug this pins ────────────────────────────────────────────────────────────
 * `AgentSessionThreadPanel`'s previous scroll owner, `useStickToBottom`,
 * pinned to the bottom in an effect with an EMPTY dependency array — once, at
 * mount (`el.scrollTop = el.scrollHeight`). Observer events render
 * asynchronously (relay connect handshake, archive backfill); if they hadn't
 * committed by the time that effect ran, `scrollHeight` still equalled
 * `clientHeight` and the pin was a no-op with no keyed retry. Whether the
 * panel visibly "snapped" depended entirely on a race between content
 * arrival and the mount effect — the "sometimes it does, sometimes it
 * doesn't" behavior reported against the live panel.
 *
 * `useAnchoredScroll` fixes this by gating its own mount-pin on `isLoading`
 * (held while the connecting skeleton shows) and re-running its restoration
 * effect on every `messages` change, so the pin fires against whatever
 * `scrollHeight` is current when loading clears or content lands — never a
 * frozen mount-time snapshot.
 *
 * ── CI surface ────────────────────────────────────────────────────────────────
 * Runs under `pnpm test` (node:test with the React dev build).
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

// Minimal scroll-container shim: only the geometry + `scrollTo` the hook's
// bottom-tail path touches. No `[data-message-id]` rows — this pins the
// at-bottom fast path, not the mid-history anchor walk.
function makeContainer({ clientHeight, scrollHeight, scrollTop = 0 }) {
  return {
    clientHeight,
    scrollHeight,
    scrollTop,
    getBoundingClientRect() {
      return { top: 0 };
    },
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    scrollTo({ top }) {
      this.scrollTop = top;
    },
  };
}

// Mirrors AgentSessionThreadPanel's exact wiring: bottom-tail only (no
// targetMessageId, pinTargetCentered omitted/false), isLoading derived from
// the observer store's connection state.
function ObserverFeedHarness({ isLoading, messages, refs }) {
  useAnchoredScroll({
    channelId: "agent-pubkey:channel-id",
    contentRef: refs.content,
    isLoading,
    messages,
    scrollContainerRef: refs.container,
  });
  return null;
}

test("re-pins to the new floor once observer content commits after mount, even with no isLoading transition", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  // Mount with `isLoading: false` from the very first render (e.g. the
  // agent was already "open" when the panel mounted) and no rows yet —
  // observer events land a beat later over the relay. This is the exact
  // race `useStickToBottom`'s empty-deps mount effect lost: nothing else
  // ever re-fires the pin once the mount effect has run.
  refs.container.current = makeContainer({
    clientHeight: 0,
    scrollHeight: 0,
    scrollTop: 0,
  });

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: false,
        messages: [],
        refs,
      }),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  // Observer events commit into the DOM on the next render — `isLoading`
  // never changes again, only `messages` grows. A mount-once effect has
  // nothing left to key a retry on here.
  refs.container.current.scrollHeight = 3_000;
  refs.container.current.clientHeight = 400;
  const messages = Array.from({ length: 40 }, (_, i) => ({ id: String(i) }));

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: false,
        messages,
        refs,
      }),
    );
  });

  assert.equal(
    refs.container.current.scrollTop,
    refs.container.current.scrollHeight,
    "re-pin must be keyed on message arrival, not just the mount commit",
  );

  await act(async () => {
    root.unmount();
  });
});

test("auto-pins to bottom once loading clears, against content that already committed", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  // Skeleton phase: container is attached but nothing has rendered yet.
  refs.container.current = makeContainer({
    clientHeight: 0,
    scrollHeight: 0,
    scrollTop: 0,
  });

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: true,
        messages: [],
        refs,
      }),
    );
  });

  // isLoading holds the mount-pin: no scroll write while the skeleton shows.
  assert.equal(refs.container.current.scrollTop, 0);

  // Observer events (relay + archive backfill) commit into the DOM WHILE
  // `connectionState` is still resolving — the exact race that made
  // `useStickToBottom`'s empty-deps mount effect a no-op. `scrollHeight`
  // grows well past `clientHeight` before the loading flag flips.
  refs.container.current.scrollHeight = 3_000;
  refs.container.current.clientHeight = 400;
  const messages = Array.from({ length: 40 }, (_, i) => ({ id: String(i) }));

  // connectionState resolves out of "connecting" on the next render.
  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: false,
        messages,
        refs,
      }),
    );
  });
  assert.equal(
    refs.container.current.scrollTop,
    refs.container.current.scrollHeight,
    "the first bottom pin happens in the layout effect before the next frame",
  );

  // Flush the rAF settling pass. Late measurements must preserve the same floor.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(
    refs.container.current.scrollTop,
    refs.container.current.scrollHeight,
    "the settling pass keeps the view pinned against the committed DOM",
  );

  await act(async () => {
    root.unmount();
  });
});

test("stays glued to the floor as further messages stream in after the initial pin", async () => {
  const refs = {
    container: { current: null },
    content: { current: {} },
  };
  const root = createRoot(document.createElement("div"));

  refs.container.current = makeContainer({
    clientHeight: 400,
    scrollHeight: 1_000,
    scrollTop: 0,
  });
  const initialMessages = Array.from({ length: 10 }, (_, i) => ({
    id: String(i),
  }));

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: false,
        messages: initialMessages,
        refs,
      }),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  assert.equal(refs.container.current.scrollTop, 1_000);

  // A streaming wave lands: content grows and the container's own
  // `scrollHeight` grows with it. Still anchored at-bottom -> re-pin to the
  // NEW floor on this commit — no mount race to lose this time.
  refs.container.current.scrollHeight = 1_800;
  const nextMessages = [
    ...initialMessages,
    ...Array.from({ length: 5 }, (_, i) => ({ id: `next-${i}` })),
  ];

  await act(async () => {
    root.render(
      React.createElement(ObserverFeedHarness, {
        isLoading: false,
        messages: nextMessages,
        refs,
      }),
    );
  });

  assert.equal(refs.container.current.scrollTop, 1_800);

  await act(async () => {
    root.unmount();
  });
});
