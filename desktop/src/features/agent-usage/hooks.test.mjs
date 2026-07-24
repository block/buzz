/**
 * Fake-timer proof for the M4-frozen midnight-rollover contract:
 * `useLocalDayBoundaries` schedules exactly one `setTimeout` per local
 * midnight, and each fire both rebuilds the boundary set (so the query key
 * changes) AND reschedules the next fire — never `setInterval`, which would
 * drift across DST.
 *
 * Uses the same minimal DOM shim + `react-dom/client` `createRoot`/`act`
 * harness pattern as `useAnchoredScroll.lifecycle.test.mjs`, combined with
 * `node:test`'s `mock.timers` (as used in `activeAgentTurnsStore.test.mjs`)
 * to drive the wall clock deterministically across the rollover boundary.
 */

import assert from "node:assert/strict";
import test from "node:test";
import { mock } from "node:test";

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
  // Never route through the mocked `setTimeout` — React's scheduler falls
  // back to a real `MessageChannel` (present natively in Node), so mocking
  // only `setTimeout`/`Date` below cannot stall a commit.
  globalThis.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.CSS = { escape: (value) => value };
}

installDOMShim();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useLocalDayBoundaries } from "./hooks.ts";

function Harness({ days, onBoundaries }) {
  const boundaries = useLocalDayBoundaries(days);
  onBoundaries(boundaries);
  return null;
}

test("useLocalDayBoundaries reschedules across two local-midnight rollovers, rebuilding boundaries each time", async () => {
  // One minute before local midnight, so the first scheduled `setTimeout`
  // fires quickly under `mock.timers`.
  const beforeMidnight = new Date(2026, 5, 15, 23, 59, 0);
  mock.timers.enable({
    apis: ["setTimeout", "Date"],
    now: beforeMidnight.getTime(),
  });

  try {
    let latest = null;
    const captured = [];
    const onBoundaries = (boundaries) => {
      latest = boundaries;
    };

    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(React.createElement(Harness, { days: 7, onBoundaries }));
    });

    assert.equal(latest.length, 8, "7-day window yields 8 boundaries");
    captured.push(latest);

    // Advance 1 minute — crosses the Jun 16 local midnight. The single
    // scheduled `setTimeout` must fire, bump `rolloverTick`, and rebuild the
    // boundary set (React flushes the resulting state update inside `act`).
    await act(async () => {
      mock.timers.tick(60_000);
    });

    assert.notDeepEqual(
      latest,
      captured[0],
      "boundaries must rebuild after the first midnight rollover fires",
    );
    assert.equal(latest.length, 8, "boundary count is unchanged by a rollover");
    const juneSeventeenTomorrow = Math.floor(
      new Date(2026, 5, 17, 0, 0, 0, 0).getTime() / 1_000,
    );
    assert.equal(
      latest.at(-1),
      juneSeventeenTomorrow,
      "newest boundary must shift forward to tomorrow of the new window",
    );
    captured.push(latest);

    // Advance a full day — crosses the Jun 17 local midnight. This only
    // fires if the first rollover's effect RESCHEDULED a fresh `setTimeout`
    // rather than firing once and going silent.
    await act(async () => {
      mock.timers.tick(24 * 60 * 60 * 1000);
    });

    assert.notDeepEqual(
      latest,
      captured[1],
      "boundaries must rebuild again after the second midnight rollover fires, proving the timer rescheduled itself",
    );
    const juneEighteenTomorrow = Math.floor(
      new Date(2026, 5, 18, 0, 0, 0, 0).getTime() / 1_000,
    );
    assert.equal(
      latest.at(-1),
      juneEighteenTomorrow,
      "newest boundary must shift forward again after the rescheduled rollover",
    );

    await act(async () => {
      root.unmount();
    });
  } finally {
    mock.timers.reset();
  }
});

test("useLocalDayBoundaries clears its scheduled timeout on unmount (no post-unmount rollover)", async () => {
  const beforeMidnight = new Date(2026, 5, 15, 23, 59, 0);
  mock.timers.enable({
    apis: ["setTimeout", "Date"],
    now: beforeMidnight.getTime(),
  });

  try {
    let renderCount = 0;
    const onBoundaries = () => {
      renderCount++;
    };

    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(React.createElement(Harness, { days: 7, onBoundaries }));
    });
    const countAtUnmount = renderCount;

    await act(async () => {
      root.unmount();
    });

    // Crossing the midnight the pending timeout targeted must not throw or
    // invoke a setState-after-unmount path — `clearTimeout` in the effect's
    // cleanup must have already cancelled it.
    await act(async () => {
      mock.timers.tick(60_000);
    });

    assert.equal(
      renderCount,
      countAtUnmount,
      "no render (and no error) after the component unmounted",
    );
  } finally {
    mock.timers.reset();
  }
});
