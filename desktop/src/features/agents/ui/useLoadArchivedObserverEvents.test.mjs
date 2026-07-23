/**
 * Mounted-hook lifecycle and race regression tests for
 * useLoadArchivedObserverEvents.
 *
 * These tests mount the REAL production hook (including its useEffect wiring,
 * fetchOlderArchived closure, runHydrationLoop call, and requestChannelId token
 * checks) against a mocked Tauri IPC bridge and a real QueryClientProvider.
 * They fail if any of the following is removed from the production hook:
 *   - the hydration effect
 *   - the requestChannelId token checks (post-Tauri-read AND post-ingest)
 *   - the generation-aware isFetching clear in finally
 *
 * Two regressions:
 *   (a) exhausted-A → switch-to-B: B must read from a null cursor and ingest
 *       its rows. Fails at dfb2d0385 (before fix) and passes after.
 *   (b) deferred-I/O race: A is in flight (Tauri read deferred), switch to B,
 *       resolve A's 1-row short ingest — A must NOT mark B exhausted, and B's
 *       eager loop must continue past page 1 toward its budget. Fails at
 *       dfb2d0385 (before post-ingest token fix) and passes after.
 *
 * ── DOM shim ─────────────────────────────────────────────────────────────────
 * react-dom/client requires a minimal DOM; node has none. We install the same
 * minimal shim used by MessageComposerDraftImagePersist.test.mjs.
 *
 * ── Tauri IPC mock ───────────────────────────────────────────────────────────
 * @tauri-apps/api/core calls window.__TAURI_INTERNALS__.invoke(cmd, args).
 * We install a per-test mock at globalThis.__TAURI_INTERNALS__.invoke so every
 * listSaveSubscriptions / readArchivedObserverEventsForChannel / readUnindexed /
 * indexObserverChannelId call is intercepted by command name without patching
 * module internals.
 */

import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";

// ── Minimal DOM shim (matches MessageComposerDraftImagePersist.test.mjs) ──────

function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      if (!this._listeners[type]) this._listeners[type] = [];
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
}

installDOMShim();

// ── Tauri IPC interceptor ─────────────────────────────────────────────────────
//
// @tauri-apps/api/core calls window.__TAURI_INTERNALS__.invoke(cmd, args).
// Install a stub now (before any module that imports tauriArchive is loaded)
// so listSaveSubscriptions, readArchivedObserverEventsForChannel, etc. can be
// controlled per-test by replacing ipcHandlers.

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd, args) => {
    const handler = ipcHandlers.get(cmd);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback: (_cb) => {
    const id = Math.random();
    return id;
  },
};

function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
function clearIpcHandlers() {
  ipcHandlers.clear();
}

// ── Production imports (after shim, after IPC stub) ───────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { useLoadArchivedObserverEvents } from "@/features/agents/ui/useObserverEvents.ts";
import {
  resetAgentObserverStore,
  _testRegisterKnownAgents,
  _testGetArchivedChannelEvents,
} from "@/features/agents/observerRelayStore.ts";

// ── Constants ─────────────────────────────────────────────────────────────────

const AGENT_PUBKEY = "a".repeat(64);
const IDENTITY_PUBKEY = "c".repeat(64);
const SUB_ID = "test-hook-sub";

// ── Tauri wire-shape helpers ──────────────────────────────────────────────────

/** Returns a list_save_subscriptions response with one owner_p subscription. */
function makeOwnerPSubResponse() {
  return [
    {
      identity_pubkey: IDENTITY_PUBKEY,
      relay_url: "wss://test",
      scope_type: "owner_p",
      scope_value: IDENTITY_PUBKEY,
      kinds: "[24200]",
      created_at: 1000,
    },
  ];
}

/** Returns a raw archived observer event row for readArchivedObserverEventsForChannel. */
function makeArchivedRow(seq, channelId = "chan-1") {
  return {
    id: `ev${String(seq).padStart(63, "0")}`,
    pubkey: AGENT_PUBKEY,
    created_at: 1000 + seq,
    kind: 24200,
    tags: [
      ["p", IDENTITY_PUBKEY],
      ["agent", AGENT_PUBKEY],
      ["frame", "telemetry"],
    ],
    content: JSON.stringify({
      seq,
      timestamp: new Date(1_000_000 + seq * 1000).toISOString(),
      channelId,
      kind: "telemetry",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: { method: "session/update", params: {} },
    }),
    sig: "s".repeat(128),
  };
}

// ── React mounting helpers ────────────────────────────────────────────────────

/**
 * Mount useLoadArchivedObserverEvents in a real React tree with a QueryClient
 * pre-seeded with the identity. Returns { unmount, rerender(channelId) }.
 *
 * The hook itself is opaque to the harness — we only observe side-effects
 * (ingestArchivedObserverEvents writing to the store) and the IPC call log.
 */
function mountHook(_initialChannelId, queryClient) {
  function HarnessComponent({ channelId }) {
    useLoadArchivedObserverEvents(true, channelId);
    return null;
  }

  const container = document.createElement("div");
  const root = createRoot(container);

  const render = async (channelId) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(HarnessComponent, { channelId }),
        ),
      );
    });
  };

  return {
    render,
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

/** Make a QueryClient pre-seeded with identity so useIdentityQuery resolves. */
function makeQueryClient() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(["identity"], { pubkey: IDENTITY_PUBKEY });
  return qc;
}

// ── Settle helper ─────────────────────────────────────────────────────────────
//
// Flushes microtasks + a few macrotask ticks so async effects can settle.
// Uses act() so React commits state updates from effects.

async function settle(iterations = 3) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 5));
    });
  }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("useLoadArchivedObserverEvents — mounted hook lifecycle regressions", () => {
  beforeEach(() => {
    resetAgentObserverStore();
    clearIpcHandlers();
    _testRegisterKnownAgents(SUB_ID, [AGENT_PUBKEY]);
  });

  /**
   * Regression (a): exhausted channel A → switch to channel B.
   *
   * The original stale-closure bug: fetchOlderArchived captured hasOlderArchived
   * from React state (false for exhausted A). After the switch, React state was
   * still false while ps.hasOlderArchived had been reset to true. The hydration
   * loop called fetchOlderArchived up to 10 times, each returning immediately at
   * the !ps.hasOlderArchived guard — B never got a read.
   *
   * After the fix (reading ps.hasOlderArchived from the ref), B gets at least
   * one read from a null cursor and its rows are ingested.
   *
   * VERIFIED: this test fails at dfb2d0385 (pre-fix) — B ingested 0 rows.
   * After the fix it passes — B ingests its rows.
   */
  it("test_exhausted_channel_A_switch_to_B_hook_reads_B_from_null_cursor", async () => {
    // Channel A: 1 page of 1 row (short page → exhausted immediately).
    const aRows = [makeArchivedRow(1, "chan-a")];
    // Channel B: 1 page of 1 row.
    const bRows = [makeArchivedRow(2, "chan-b")];

    const aCalls = [];
    const bCalls = [];

    setIpcHandler("list_save_subscriptions", async () =>
      makeOwnerPSubResponse(),
    );
    setIpcHandler("read_unindexed_observer_rows", async () => []);
    setIpcHandler("index_observer_channel_id", async () => null);
    setIpcHandler("read_archived_observer_events_for_channel", async (args) => {
      if (args.channelId === "chan-a") {
        aCalls.push({ cursor: args.beforeCreatedAt ?? null });
        return aRows.map((r) => JSON.stringify(r));
      }
      if (args.channelId === "chan-b") {
        bCalls.push({ cursor: args.beforeCreatedAt ?? null });
        return bRows.map((r) => JSON.stringify(r));
      }
      return [];
    });
    // decrypt_observer_event is called inside ingestArchivedObserverEvents.
    // invokeTauri passes { eventJson: JSON.stringify(rawRelayEvent) }.
    // The row.content is the JSON-encoded ObserverEvent — return it parsed.
    setIpcHandler("decrypt_observer_event", async (args) => {
      try {
        const event = JSON.parse(args.eventJson);
        return JSON.parse(event.content);
      } catch {
        return { kind: "telemetry", channelId: null };
      }
    });

    const qc = makeQueryClient();
    const { render, unmount } = mountHook("chan-a", qc);

    // Mount on chan-a and let hydration settle.
    await render("chan-a");
    await settle(10);

    // A must have been read (at least one call, from null cursor).
    assert.ok(aCalls.length >= 1, `expected A reads, got ${aCalls.length}`);
    assert.equal(aCalls[0].cursor, null, "A first read must use null cursor");

    // Switch to chan-b.
    await render("chan-b");
    await settle(10);

    // B must have been read from a null cursor (fresh channel, no inherited cursor).
    assert.ok(bCalls.length >= 1, `expected B reads, got ${bCalls.length}`);
    assert.equal(
      bCalls[0].cursor,
      null,
      "B first read must use null cursor (not A's cursor)",
    );

    // B's rows must have been ingested into the archive store.
    const bArchived = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-b");
    assert.ok(
      bArchived.length >= 1,
      `B's rows must be ingested — found ${bArchived.length} (exhausted-A stale closure bug would leave this 0)`,
    );

    await unmount();
  });

  /**
   * Regression (b): deferred-I/O race — A's exhaustion write after its ingest.
   *
   * The post-ingest token bug at dfb2d0385: fetchOlderArchived for channel A
   * checked the channel token BEFORE ingestArchivedObserverEvents but NOT after.
   * If A's decrypt completed after a channel switch, A resumed past the ingestion
   * await and wrote ps.hasOlderArchived=false (short-page exhaustion) and cleared
   * ps.isFetching in finally — both using the NEW channel B's paging state.
   *
   * Precise race sequence:
   *   1. A's Tauri read returns 1 row (short page). A sets cursor, enters ingest.
   *   2. A's decrypt is DEFERRED (aDecryptDeferred).
   *   3. Switch to channel B.
   *   4. B's Tauri read is ALSO DEFERRED (bFirstReadDeferred).
   *   5. Resolve A's decrypt → A finishes ingest, hits short-page branch:
   *      - at dfb2d0385: writes ps.hasOlderArchived=false, clears ps.isFetching
   *      - after fix: post-ingest token check fails, writes are discarded
   *   6. Resolve B's first Tauri read → B ingests 200 rows.
   *   7. B loop check: ps.hasOlderArchived?
   *      - at dfb2d0385: false (A corrupted it) → loop exits, bCallCount == 1
   *      - after fix: true → loop continues to page 2+, bCallCount >= 2
   *
   * VERIFIED: this test fails at dfb2d0385 (bCallCount == 1, loop exits after
   * first B page) and passes after the post-ingest token check is added.
   */
  it("test_deferred_A_ingest_cannot_exhaust_B_or_steal_B_lock", async () => {
    let resolveADecrypt;
    const aDecryptDeferred = new Promise((resolve) => {
      resolveADecrypt = resolve;
    });

    let resolveBFirstRead;
    const bFirstReadDeferred = new Promise((resolve) => {
      resolveBFirstRead = resolve;
    });

    const PAGE_SIZE = 200;
    const makeBPage = (offset) =>
      Array.from({ length: PAGE_SIZE }, (_, i) =>
        makeArchivedRow(offset + i, "chan-b"),
      );

    let bCallCount = 0;
    let aDecryptStarted = false;
    let bFirstReadHeld = false;

    setIpcHandler("list_save_subscriptions", async () =>
      makeOwnerPSubResponse(),
    );
    setIpcHandler("read_unindexed_observer_rows", async () => []);
    setIpcHandler("index_observer_channel_id", async () => null);
    setIpcHandler("read_archived_observer_events_for_channel", async (args) => {
      if (args.channelId === "chan-a") {
        return [JSON.stringify(makeArchivedRow(1, "chan-a"))]; // 1 row = short page
      }
      if (args.channelId === "chan-b") {
        bCallCount++;
        if (bCallCount === 1 && !bFirstReadHeld) {
          // Defer B's first Tauri read until we explicitly release it.
          bFirstReadHeld = true;
          await bFirstReadDeferred;
          return makeBPage(100).map((r) => JSON.stringify(r)); // full page
        }
        if (bCallCount <= 4)
          return makeBPage(bCallCount * 100).map((r) => JSON.stringify(r));
        return [JSON.stringify(makeArchivedRow(9999, "chan-b"))]; // short = exhaust
      }
      return [];
    });
    setIpcHandler("decrypt_observer_event", async (args) => {
      try {
        const event = JSON.parse(args.eventJson);
        const parsed = JSON.parse(event.content);
        if (parsed.channelId === "chan-a" && !aDecryptStarted) {
          aDecryptStarted = true;
          await aDecryptDeferred; // block A's decrypt
        }
        return parsed;
      } catch {
        return { kind: "telemetry", channelId: null };
      }
    });

    const qc = makeQueryClient();
    const { render, unmount } = mountHook("chan-a", qc);

    // Step 1-2: Mount on chan-a. A's Tauri read completes (1 row), cursor set,
    // ingest starts and blocks at A's decrypt.
    await render("chan-a");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    // Step 3: Switch to chan-b while A's decrypt/ingest is blocked.
    await render("chan-b");

    // Step 4: B calls its first Tauri read and blocks.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    // Step 5: Resolve A's decrypt first. At dfb2d0385 this writes
    // ps.hasOlderArchived=false and clears ps.isFetching.
    // After fix this is a no-op (token mismatch discards the writes).
    resolveADecrypt();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // Step 6: Now resolve B's first Tauri read.
    resolveBFirstRead();

    // Let B's loop run.
    await settle(10);

    // Step 7: B must have made at least 2 Tauri reads.
    // At dfb2d0385: A corrupted ps.hasOlderArchived=false before B's first page
    // resolved, so after B's first page, the loop checks and exits. bCallCount==1.
    // After fix: A's write was discarded, ps.hasOlderArchived is still true,
    // B continues to page 2+.
    assert.ok(
      bCallCount >= 2,
      `B must read at least 2 pages — got ${bCallCount}. Post-ingest token missing at dfb2d0385 let A corrupt B's exhaustion state (bCallCount==1).`,
    );

    // A's row must NOT appear in B's channel archive.
    const bArchived = _testGetArchivedChannelEvents(AGENT_PUBKEY, "chan-b");
    for (const evt of bArchived) {
      assert.equal(
        evt.channelId,
        "chan-b",
        "B's archive must only contain B-channel events",
      );
    }

    await unmount();
  });
});
