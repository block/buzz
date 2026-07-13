/**
 * Snapshot projection invariant: timeline rows and the history-exhaustion
 * proof must be inseparable through EVERY render transport stage — React
 * deferral (`useDeferredValue`), tail buffering, and the settle-gated prepend
 * hold.
 *
 * ── Bug this pins (divider tear, 2026-07-11) ────────────────────────────────
 * `historyExhausted` used to travel to TimelineMessageList as a bare prop on
 * the URGENT render path while the rows rode the deferred/gated pipeline. The
 * final older-history page (same-day rows + hasMore:false in one response)
 * therefore produced an intermediate commit pairing the NEW exhaustion proof
 * with the STALE row array — {rendered:100, exhausted:true} — which minted the
 * oldest-day divider against a partially-loaded day. When the withheld rows
 * landed one commit later they prepended BEHIND the already-minted divider
 * key, exact-suffix shift admission failed, and the full page height landed
 * as an uncompensated scroll jump. Ledgered 3/3 in
 * RESEARCH/DIVIDER_EXTERNALIZATION_KEYSPACE.md (addendum).
 *
 * The fix threads the proof INSIDE TimelineSnapshot and through the settle
 * gate as paired `meta`, so no render can observe a (rows, proof) pair that
 * was never published together.
 *
 * **Revert verification**: feeding the gate `meta` from the LIVE snapshot
 * (`liveSnapshot.historyExhausted`) instead of the deferred one — or passing
 * the raw `historyExhausted` prop straight to the list, as the old wiring
 * did — makes the forbidden-pair assertion below fail on the urgent commit.
 *
 * ── CI surface ──────────────────────────────────────────────────────────────
 * Runs under `pnpm test` (node:test with the React dev build). Not Playwright:
 * the assertion is about per-commit render pairs, which only a render recorder
 * can observe deterministically.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim (same shape as MessageComposerDraftImagePersist) ───────

function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      this._listeners[type] ??= [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      if (this._listeners[type]) {
        this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
      }
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
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 16);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
}

installDOMShim();

// ── Imports (after shim) ─────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

import { useBufferedTimelineMessages } from "./useBufferedTimelineMessages.ts";
import { useSettleGatedPrependMessages } from "./useSettleGatedPrependMessages.ts";

// ── Harness: MessageTimeline's exact snapshot transport pipeline ────────────
// liveSnapshot (rows+proof as ONE value) → useDeferredValue → tail buffering
// → settle-gated prepend hold. Records the (rowCount, firstId, exhausted)
// pair every render body — the same pair buildVirtualizedItems consumes.

const rows = (ids) => ids.map((id) => ({ id }));
const ids = (prefix, from, to) => {
  const out = [];
  for (let i = from; i < to; i += 1) out.push(`${prefix}${i}`);
  return out;
};

const EMPTY_SNAPSHOT = {
  channelId: null,
  messages: [],
  historyExhausted: false,
};

function makeFakeScroller() {
  // Constant scrollTop + no motion events: the settle gate's watcher reaches
  // quiet-window + stable-frames and admits the held page on its own.
  const listeners = {};
  return {
    scrollTop: 0,
    addEventListener(type, fn) {
      listeners[type] ??= [];
      listeners[type].push(fn);
    },
    removeEventListener(type, fn) {
      if (listeners[type]) {
        listeners[type] = listeners[type].filter((f) => f !== fn);
      }
    },
  };
}

function pairOf(snapshot) {
  return {
    count: snapshot.messages.length,
    firstId: snapshot.messages[0]?.id ?? null,
    exhausted: snapshot.historyExhausted,
  };
}

function samePair(a, b) {
  return (
    a.count === b.count &&
    a.firstId === b.firstId &&
    a.exhausted === b.exhausted
  );
}

function makeHarness(records, scroller) {
  return function Harness({ snapshot }) {
    const deferredSnapshot = React.useDeferredValue(snapshot, EMPTY_SNAPSHOT);
    const buffered = useBufferedTimelineMessages({
      channelId: deferredSnapshot.channelId,
      isAtBottom: false, // reader is scrolled up — the tear's regime
      messages: deferredSnapshot.messages,
    });
    const {
      messages: rendered,
      meta: exhausted,
      isHoldingPrepend,
    } = useSettleGatedPrependMessages({
      channelId: deferredSnapshot.channelId,
      messages: buffered.messages,
      meta: deferredSnapshot.historyExhausted,
      scrollElementRef: { current: scroller },
    });
    records.push({
      count: rendered.length,
      firstId: rendered[0]?.id ?? null,
      exhausted,
      isHoldingPrepend,
    });
    return null;
  };
}

async function mount(Comp, snapshot) {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(Comp, { snapshot }));
  });
  return {
    update: async (nextSnapshot) => {
      await act(async () => {
        root.render(React.createElement(Comp, { snapshot: nextSnapshot }));
      });
    },
    settle: async (ms) => {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, ms));
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

// ── Tests ────────────────────────────────────────────────────────────────────

test("pass-1 landing: exhaustion proof can never pair with the stale row array", async () => {
  const CHANNEL = "chan-tear";
  // Snapshot A: 100 rows loaded, more history exists (mid-pagination).
  const snapA = {
    channelId: CHANNEL,
    messages: rows(ids("m", 35, 135)),
    historyExhausted: false,
  };
  // Snapshot B: the pass-1 landing — 35 older same-day rows AND the
  // exhaustion proof arrive in ONE authoritative publish (the ledgered
  // observed order: both caches coherent in the same notification batch).
  const snapB = {
    channelId: CHANNEL,
    messages: rows(ids("m", 0, 135)),
    historyExhausted: true,
  };

  const records = [];
  const scroller = makeFakeScroller();
  const handle = await mount(makeHarness(records, scroller), snapA);

  await handle.update(snapB);

  // The settle gate must actually engage for this to exercise the held
  // window — otherwise the test is vacuous.
  assert.ok(
    records.some((r) => r.isHoldingPrepend),
    "settle gate must hold the pure history prepend at least one render",
  );
  // While held, the rendered pair stays the ADMITTED one: {100, false}.
  const heldRecords = records.filter((r) => r.isHoldingPrepend);
  for (const r of heldRecords) {
    assert.equal(r.count, 100, "held rows must remain the admitted 100");
    assert.equal(
      r.exhausted,
      false,
      "held rows must keep the admitted (false) proof — not the fresh one",
    );
  }

  // Let the settle watcher admit (quiet window 100ms + 3 stable frames).
  await handle.settle(400);

  // Final state: rows and proof advanced TOGETHER.
  const last = records[records.length - 1];
  assert.deepEqual(
    { count: last.count, firstId: last.firstId, exhausted: last.exhausted },
    { count: 135, firstId: "m0", exhausted: true },
    "after settle the full page and its proof must land together",
  );

  // THE invariant: every render observed a (rows, proof) pair that was
  // actually published. The forbidden torn state {rendered:100,
  // exhausted:true} — the divider-minting commit — must never exist.
  const published = [EMPTY_SNAPSHOT, snapA, snapB].map(pairOf);
  for (const r of records) {
    assert.ok(
      published.some((p) => samePair(p, r)),
      `torn render: {count:${r.count}, firstId:${r.firstId}, exhausted:${r.exhausted}} matches no published snapshot`,
    );
  }

  await handle.unmount();
});

test("reverse reconnect: exhaustion retraction stays paired with its replacement rows", async () => {
  const CHANNEL = "chan-retract";
  // Exhausted window fully loaded…
  const snapExhausted = {
    channelId: CHANNEL,
    messages: rows(ids("old", 0, 135)),
    historyExhausted: true,
  };
  // …then a reconnect head-refresh replaces the whole chain with page zero
  // reporting hasMore:true — exhaustion retracts, rows change wholesale.
  const snapRetracted = {
    channelId: CHANNEL,
    messages: rows(ids("new", 0, 50)),
    historyExhausted: false,
  };

  const records = [];
  const scroller = makeFakeScroller();
  const handle = await mount(makeHarness(records, scroller), snapExhausted);

  await handle.update(snapRetracted);
  await handle.settle(400);

  // A wholesale replacement is not a pure prepend: the gate passes it through
  // (no hold), and the retracted proof must arrive WITH the new rows — never
  // stale-true against the new oldest prefix, never premature-false against
  // the old rows.
  const last = records[records.length - 1];
  assert.deepEqual(
    { count: last.count, firstId: last.firstId, exhausted: last.exhausted },
    { count: 50, firstId: "new0", exhausted: false },
  );
  const published = [EMPTY_SNAPSHOT, snapExhausted, snapRetracted].map(pairOf);
  for (const r of records) {
    assert.ok(
      published.some((p) => samePair(p, r)),
      `torn render on retraction: {count:${r.count}, firstId:${r.firstId}, exhausted:${r.exhausted}}`,
    );
  }

  await handle.unmount();
});
