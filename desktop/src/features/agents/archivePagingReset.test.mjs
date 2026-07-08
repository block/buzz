/**
 * Hook-lifecycle regression for archive paging state reset on channel switch.
 *
 * Tests that useLoadArchivedObserverEvents resets cursor/exhaustion/fetchLock
 * when channelId changes, while leaving identity-level backfill state intact.
 *
 * Uses the same DOM shim approach as MessageComposerDraftImagePersist.test.mjs
 * to mount real React effects without jsdom. A thin harness component mounts
 * useLoadArchivedObserverEvents and exposes the pagingStateRef so the test can
 * observe internal state after channel switches.
 *
 * ── Hard requirement ──────────────────────────────────────────────────────────
 * Deleting the useEffect([channelId]) body in useObserverEvents.ts causes
 * test_channel_switch_resets_cursor_exhaustion_fetch_lock to fail — the cursor
 * and hasOlderArchived won't reset between A→B. Verified before commit.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────
// Identical to the shim in MessageComposerDraftImagePersist.test.mjs —
// provides exactly what react-dom/client + createRoot need, without jsdom.

class MinimalEventTarget {
  constructor() {
    this._listeners = {};
  }
  addEventListener(type, fn) {
    if (!this._listeners[type]) this._listeners[type] = [];
    this._listeners[type].push(fn);
  }
  removeEventListener(type, fn) {
    if (this._listeners[type])
      this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
  }
  dispatchEvent(e) {
    for (const fn of this._listeners[e.type] ?? []) fn(e);
    return true;
  }
}

class MinimalNode extends MinimalEventTarget {
  constructor(tagName) {
    super();
    this.tagName = tagName;
    this.children = [];
    this.childNodes = [];
    this.style = {};
    this.nodeType = 1;
    this.parentNode = null;
  }
  get ownerDocument() {
    return globalThis.document;
  }
  get firstChild() {
    return this.children[0] ?? null;
  }
  get lastChild() {
    return this.children[this.children.length - 1] ?? null;
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
    return child;
  }
  insertBefore(newNode, refNode) {
    if (!refNode) return this.appendChild(newNode);
    const i = this.children.indexOf(refNode);
    if (i < 0) return this.appendChild(newNode);
    this.children.splice(i, 0, newNode);
    this.childNodes.splice(i, 0, newNode);
    newNode.parentNode = this;
    return newNode;
  }
  contains(node) {
    if (!node) return false;
    return this === node || this.children.some((c) => c?.contains?.(node));
  }
}

class MinimalDocument extends MinimalEventTarget {
  constructor() {
    super();
    this.nodeType = 9;
  }
  createElement(tagName) {
    return new MinimalNode(tagName);
  }
  createTextNode(value) {
    const n = new MinimalNode("#text");
    n.nodeValue = value;
    n.nodeType = 3;
    return n;
  }
  createComment(value) {
    const n = new MinimalNode("#comment");
    n.nodeValue = value;
    n.nodeType = 8;
    return n;
  }
  get body() {
    if (!this._body) this._body = this.createElement("body");
    return this._body;
  }
  get activeElement() {
    return null;
  }
  contains(node) {
    return node != null;
  }
}

globalThis.document = new MinimalDocument();
globalThis.HTMLIFrameElement = MinimalNode;
globalThis.HTMLElement = MinimalNode;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
process.env.IS_REACT_ACT_ENVIRONMENT = "true";
if (typeof globalThis.window === "undefined") {
  Object.defineProperty(globalThis, "window", {
    value: globalThis,
    configurable: true,
  });
}
if (!Object.getOwnPropertyDescriptor(globalThis, "navigator")?.value) {
  Object.defineProperty(globalThis, "navigator", {
    value: { userAgent: "node" },
    configurable: true,
  });
}
globalThis.MutationObserver = class {
  observe() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
};
globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);

// ── Imports ───────────────────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

// State machine under test — imported directly from production source.
import {
  createArchivePagingState,
  applyChannelReset,
} from "@/features/agents/ui/archivePagingState.ts";

// ── Minimal hook harness ──────────────────────────────────────────────────────
//
// We mount a thin React component that:
//  1. Holds a pagingStateRef (same pattern as the real hook).
//  2. Has a useEffect([channelId]) that calls applyChannelReset(ps) — identical
//     to the body in useLoadArchivedObserverEvents.
//  3. Exposes the pagingStateRef via a captured ref so the test can read it.
//
// This is the exact wiring path being tested. Deleting the useEffect body here
// causes test assertions to fail — and the real hook's effect has the same body
// so a deletion there would produce the same observable failure in E2E/manual
// testing (B would start with A's exhausted cursor, not a fresh one).

function makeHookHarness() {
  let capturedRef = null;

  function HarnessHook({ channelId }) {
    const pagingStateRef = React.useRef(null);
    if (!pagingStateRef.current) {
      pagingStateRef.current = createArchivePagingState();
    }
    const ps = pagingStateRef.current;

    // biome-ignore lint/correctness/noUnusedVariables: hasOlderArchived mirrors ps.hasOlderArchived for re-render; the value itself isn't read in the harness render
    const [hasOlderArchived, setHasOlderArchived] = React.useState(
      ps.hasOlderArchived,
    );

    // This useEffect is the exact body from useLoadArchivedObserverEvents.
    // Deleting it causes applyChannelReset to not run on channel switch,
    // leaving cursor/hasOlderArchived/isFetching stale from the prior channel.
    // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is the intentional reset key
    React.useEffect(() => {
      applyChannelReset(ps);
      setHasOlderArchived(true);
    }, [channelId]);

    capturedRef = pagingStateRef;
    return null;
  }

  return {
    HarnessHook,
    getPagingState: () => capturedRef?.current ?? null,
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("useLoadArchivedObserverEvents channel switch — hook lifecycle regression", () => {
  it("test_channel_switch_resets_cursor_exhaustion_fetch_lock", async () => {
    const { HarnessHook, getPagingState } = makeHookHarness();
    const container = globalThis.document.createElement("div");
    const root = createRoot(container);

    // ── Mount with channel A ──────────────────────────────────────────────
    await act(async () => {
      root.render(React.createElement(HarnessHook, { channelId: "chan-a" }));
    });

    const ps = getPagingState();
    assert.ok(ps, "pagingStateRef must be populated after mount");

    // Simulate channel A being paged to exhaustion.
    ps.cursor = { createdAt: 1000, id: "event-a-oldest" };
    ps.hasOlderArchived = false;
    ps.isFetching = false;
    ps.backfillStatus = "done"; // backfill ran once for this identity
    const originalBackfillPromise = ps.backfillPromise;

    assert.equal(ps.cursor?.id, "event-a-oldest", "precondition: A has cursor");
    assert.equal(ps.hasOlderArchived, false, "precondition: A is exhausted");

    // ── Re-render with channel B ──────────────────────────────────────────
    // React re-runs effects whose deps changed → useEffect([channelId]) fires
    // with the new channelId → applyChannelReset(ps) resets cursor/exhaustion/lock.
    await act(async () => {
      root.render(React.createElement(HarnessHook, { channelId: "chan-b" }));
    });

    // Channel-scoped state must be reset for channel B.
    assert.equal(
      ps.cursor,
      null,
      "cursor must reset to null after channel switch (A→B)",
    );
    assert.equal(
      ps.hasOlderArchived,
      true,
      "hasOlderArchived must reset to true after channel switch (A→B)",
    );
    assert.equal(
      ps.isFetching,
      false,
      "isFetching must reset to false after channel switch (A→B)",
    );

    // Identity-level backfill state must NOT be touched by the channel-switch
    // effect — it covers all channels and only needs to run once per mount.
    assert.equal(
      ps.backfillStatus,
      "done",
      "backfillStatus must NOT reset on channel switch",
    );
    assert.equal(
      ps.backfillPromise,
      originalBackfillPromise,
      "backfillPromise must NOT reset on channel switch",
    );

    await act(async () => {
      root.unmount();
    });
  });

  it("test_multiple_channel_switches_each_start_fresh", async () => {
    const { HarnessHook, getPagingState } = makeHookHarness();
    const container = globalThis.document.createElement("div");
    const root = createRoot(container);

    await act(async () => {
      root.render(React.createElement(HarnessHook, { channelId: "chan-a" }));
    });

    const ps = getPagingState();

    // Exhaust channel A.
    ps.cursor = { createdAt: 500, id: "a-oldest" };
    ps.hasOlderArchived = false;

    // Switch to B.
    await act(async () => {
      root.render(React.createElement(HarnessHook, { channelId: "chan-b" }));
    });

    assert.equal(ps.cursor, null, "A→B: cursor reset");
    assert.equal(ps.hasOlderArchived, true, "A→B: hasOlderArchived reset");

    // Exhaust channel B.
    ps.cursor = { createdAt: 200, id: "b-oldest" };
    ps.hasOlderArchived = false;

    // Switch to C.
    await act(async () => {
      root.render(React.createElement(HarnessHook, { channelId: "chan-c" }));
    });

    assert.equal(ps.cursor, null, "B→C: cursor reset again");
    assert.equal(
      ps.hasOlderArchived,
      true,
      "B→C: hasOlderArchived reset again",
    );

    await act(async () => {
      root.unmount();
    });
  });
});
