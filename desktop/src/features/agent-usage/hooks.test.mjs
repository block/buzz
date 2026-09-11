/**
 * Fake-timer proof for `useLocalDayBoundaries`: verifies it schedules exactly
 * one `setTimeout` per local midnight, rebuilding boundaries and rescheduling
 * the next fire each time — never `setInterval` (which would drift across DST).
 * Uses `node:test`'s `mock.timers` to drive the wall clock deterministically.
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
        (this.listeners.get(type) ?? []).filter((l) => l !== listener),
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
  // React's scheduler uses MessageChannel (native in Node) as a fallback, so
  // mocking setTimeout/Date cannot stall commits — rAF just needs to exist.
  globalThis.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.CSS = { escape: (value) => value };
}

installDOMShim();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useLocalDayBoundaries } from "./hooks.ts";

function Harness({ onBoundaries, range }) {
  const boundaries = useLocalDayBoundaries(range);
  onBoundaries(boundaries);
  return null;
}

const SEVEN_DAY_RANGE = { kind: "preset", days: 7 };

/** Local midnight as unix seconds, the unit the hook emits. */
function midnight(year, monthIndex, day) {
  return Math.floor(
    new Date(year, monthIndex, day, 0, 0, 0, 0).getTime() / 1_000,
  );
}

/**
 * Mounts the hook with `initialRange` and calls `run` with a live view:
 * `boundaries` = newest emission, `renders` = emission count,
 * `render(range)` re-renders, `tick(ms)` advances the mocked clock.
 */
async function withMountedHook(initialRange, run) {
  const emissions = [];
  const root = createRoot(document.createElement("div"));
  const render = async (range) => {
    await act(async () => {
      root.render(
        React.createElement(Harness, {
          onBoundaries: (boundaries) => emissions.push(boundaries),
          range,
        }),
      );
    });
  };

  try {
    await render(initialRange);
    await run({
      emissions,
      get boundaries() {
        return emissions.at(-1);
      },
      get renders() {
        return emissions.length;
      },
      render,
      unmount: async () => {
        await act(async () => {
          root.unmount();
        });
      },
      /** Advance the mocked wall clock, flushing any React work it triggers. */
      tick: async (ms) => {
        await act(async () => {
          mock.timers.tick(ms);
        });
      },
    });
  } finally {
    await act(async () => {
      root.unmount();
    });
  }
}

/** Runs `fn` with the wall clock frozen one minute before a local midnight. */
async function atOneMinuteToMidnight(fn) {
  mock.timers.enable({
    apis: ["setTimeout", "Date"],
    now: new Date(2026, 5, 15, 23, 59, 0).getTime(),
  });
  try {
    await fn();
  } finally {
    mock.timers.reset();
  }
}

test("useLocalDayBoundaries reschedules across two local-midnight rollovers, rebuilding boundaries each time", async () => {
  await atOneMinuteToMidnight(async () => {
    await withMountedHook(SEVEN_DAY_RANGE, async (hook) => {
      assert.equal(
        hook.boundaries.length,
        8,
        "7-day window yields 8 boundaries",
      );
      const initial = hook.boundaries;

      // Crosses the Jun 16 local midnight. The single scheduled `setTimeout`
      // must fire, bump `rolloverTick`, and rebuild the boundary set.
      await hook.tick(60_000);

      assert.notDeepEqual(
        hook.boundaries,
        initial,
        "boundaries must rebuild after the first midnight rollover fires",
      );
      assert.equal(
        hook.boundaries.length,
        8,
        "boundary count is unchanged by a rollover",
      );
      assert.equal(
        hook.boundaries.at(-1),
        midnight(2026, 5, 17),
        "newest boundary must shift forward to tomorrow of the new window",
      );
      const afterFirst = hook.boundaries;

      // Crossing the Jun 17 local midnight only fires if the first rollover's
      // effect RESCHEDULED a fresh `setTimeout` rather than going silent.
      await hook.tick(24 * 60 * 60 * 1_000);

      assert.notDeepEqual(
        hook.boundaries,
        afterFirst,
        "boundaries must rebuild again after the second rollover, proving the timer rescheduled itself",
      );
      assert.equal(
        hook.boundaries.at(-1),
        midnight(2026, 5, 18),
        "newest boundary must shift forward again after the rescheduled rollover",
      );
    });
  });
});

test("useLocalDayBoundaries clears its scheduled timeout on unmount (no post-unmount rollover)", async () => {
  await atOneMinuteToMidnight(async () => {
    await withMountedHook(SEVEN_DAY_RANGE, async (hook) => {
      const rendersAtUnmount = hook.renders;
      await hook.unmount();

      // Crossing the midnight the pending timeout targeted must not throw or
      // invoke a setState-after-unmount path — `clearTimeout` in the effect's
      // cleanup must have already cancelled it.
      await hook.tick(60_000);

      assert.equal(
        hook.renders,
        rendersAtUnmount,
        "no render (and no error) after the component unmounted",
      );
    });
  });
});

test("useLocalDayBoundaries returns the same boundary array when re-rendered with an equal range literal", async () => {
  await withMountedHook({ kind: "preset", days: 7 }, async (hook) => {
    // A fresh object literal each render — the memo must key on the range's
    // fields, not its identity, or every render would refetch.
    await hook.render({ kind: "preset", days: 7 });

    assert.ok(hook.renders >= 2, "component rendered at least twice");
    assert.equal(
      hook.boundaries,
      hook.emissions[0],
      "an equal range literal must reuse the memoized boundary array",
    );
  });
});

test("useLocalDayBoundaries rebuilds when the range changes to a custom range", async () => {
  await withMountedHook(SEVEN_DAY_RANGE, async (hook) => {
    assert.equal(hook.boundaries.length, 8);

    await hook.render({
      kind: "custom",
      startDate: "2026-01-01",
      endDate: "2026-01-03",
    });

    assert.equal(
      hook.boundaries.length,
      4,
      "a 3-day custom range yields 4 boundaries closing the final day",
    );
    assert.equal(
      hook.boundaries[0],
      midnight(2026, 0, 1),
      "first boundary opens the requested start date in local time",
    );
  });
});

test("useLocalDayBoundaries returns no boundaries for an invalid custom range", async () => {
  // Inverted range — `useAgentUsageSeries` gates its query on
  // `boundaries.length >= 2`, so this must issue no request rather than one
  // the backend would reject.
  const invertedRange = {
    kind: "custom",
    startDate: "2026-03-10",
    endDate: "2026-03-01",
  };
  await withMountedHook(invertedRange, async (hook) => {
    assert.deepEqual(hook.boundaries, []);
  });
});
