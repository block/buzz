/**
 * Mounted-hook lifecycle and race regression tests for
 * useLoadArchivedObserverEvents.
 *
 * These tests mount the REAL production hook (including its useEffect wiring,
 * fetchOlderArchived closure, runHydrationLoop call, and resetGeneration token
 * checks) against a mocked Tauri IPC bridge and a real QueryClientProvider.
 * They fail if any of the following is removed from the production hook:
 *   - the hydration effect
 *   - the resetGeneration checks (post-backfill, post-Tauri-read, post-ingest)
 *   - the generation-aware isFetching clear in finally
 *   - the post-backfill isFetching recheck before lock acquisition
 *
 * Four regressions:
 *   (a) exhausted-A → switch-to-B: B must read from a null cursor and ingest
 *       its rows. GREEN at dfb2d0385 (stale-closure was fixed in round 1).
 *       Fails at the pre-round-1 head where ps.hasOlderArchived was read from
 *       React state, not the ref.
 *   (b) deferred-I/O race (A→B): A is in flight (decrypt deferred), switch to B,
 *       resolve A's 1-row short ingest — A must NOT mark B exhausted or steal B's
 *       fetch lock, and B's eager loop must continue past page 1. Fails at
 *       dfb2d0385 (post-ingest token missing). Lock theft is asserted by calling
 *       fetchOlderArchived concurrently while B holds the lock: with correct
 *       protection the concurrent call is rejected (read count doesn't jump);
 *       without it, A's stale finally clears the lock and the concurrent call
 *       would start a duplicate read.
 *   (c) concurrent fetches during pending backfill: two callers both suspend on
 *       the backfill promise, both resume after it resolves — only one must
 *       acquire the lock and issue the Tauri read. Fails at 4c92a018d (no
 *       post-backfill isFetching recheck).
 *   (d) A→B→A: old-A's in-flight decrypt completes after the user returns to A.
 *       Old-A's generation no longer matches (each reset increments the counter),
 *       so it must not mark fresh-A exhausted or steal fresh-A's lock. Fails
 *       when resetGeneration is replaced with a channel-string equality check.
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
 * pre-seeded with the identity. Returns { unmount, render(channelId),
 * getFetchOlderArchived() }.
 *
 * getFetchOlderArchived() returns the latest fetchOlderArchived function from
 * the hook's return value, captured on each render. Tests can call it directly
 * to probe lock behaviour without going through the hydration loop.
 */
function mountHook(_initialChannelId, queryClient) {
  // Capture the latest hook return values so tests can call fetchOlderArchived.
  const hookReturnRef = { current: null };

  function HarnessComponent({ channelId }) {
    const result = useLoadArchivedObserverEvents(true, channelId);
    hookReturnRef.current = result;
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
    getFetchOlderArchived: () => hookReturnRef.current?.fetchOlderArchived,
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
   * The original stale-closure bug (pre-round-1): fetchOlderArchived captured
   * hasOlderArchived from React state (false for exhausted A). After the switch,
   * React state was still false while ps.hasOlderArchived had been reset to true.
   * The hydration loop called fetchOlderArchived up to 10 times, each returning
   * immediately at the !ps.hasOlderArchived guard — B never got a read.
   *
   * After the fix (reading ps.hasOlderArchived from the ref), B gets at least
   * one read from a null cursor and its rows are ingested.
   *
   * PROVENANCE: this test is GREEN at dfb2d0385 (the stale-closure was already
   * fixed in round 1). It would be red at the pre-round-1 head (884ed9ba2)
   * where hasOlderArchived was captured from React state in the closure.
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
   * Regression (b): deferred-I/O race — A→B: A's stale writes after ingest AND
   * lock theft via stale finally.
   *
   * Two protections are under test independently:
   *
   * 1. Post-ingest exhaustion write (post-ingest token check):
   *    The bug at dfb2d0385: fetchOlderArchived for A checked the token BEFORE
   *    ingestArchivedObserverEvents but NOT after. A's deferred decrypt resumed
   *    after the channel switch, wrote ps.hasOlderArchived=false (short-page
   *    exhaustion) for B's paging state, and B's eager loop stopped after 1 page.
   *    Removing the post-ingest token check causes bCallCount==1.
   *
   * 2. Lock theft via generation-gated finally:
   *    If the generation guard in finally is removed, stale A's finally runs
   *    ps.isFetching=false while B holds the lock. We probe this directly:
   *    after resolving stale A (while B's first read is in flight), we call
   *    fetchOlderArchived() on B ourselves. With the correct guard, B holds
   *    the lock and the concurrent call returns immediately (bCallCount stays
   *    at 1 for now). Without it (stale A stole the lock), the concurrent call
   *    acquires the lock and starts an extra Tauri read (bCallCount jumps to 2
   *    prematurely, with concurrent in-flight reads — the assertion catches this
   *    because it fires BEFORE B's deferred first read resolves).
   *
   * Precise race sequence:
   *   1. Mount on chan-a. A's Tauri read returns 1 row. Cursor set. Ingest starts.
   *   2. A's decrypt is DEFERRED (aDecryptDeferred).
   *   3. Switch to channel B.
   *   4. B's hydration loop starts. B's first Tauri read is ALSO DEFERRED.
   *   5. Resolve A's decrypt → A finishes ingest, hits post-ingest check.
   *      - At dfb2d0385 (no post-ingest check): writes ps.hasOlderArchived=false.
   *        Also, if finally is unguarded, ps.isFetching=false (lock stolen).
   *      - After fix: both writes discarded (generation mismatch).
   *   6. LOCK PROBE (while B's first read is still deferred):
   *      call fetchOlderArchived() directly. Must return without starting a new
   *      Tauri read (B holds the lock; bCallCount must still be 1).
   *   7. Resolve B's first Tauri read → B ingests 200 rows.
   *   8. B loop continues: bCallCount >= 2.
   *
   * VERIFIED: removing the post-ingest token check causes bCallCount<2 (step 8).
   * Removing just the finally generation guard causes bCallCount>=2 but the lock
   * probe at step 6 catches the theft: bCallCount jumps to 2 before B's deferred
   * first read resolves (duplicate concurrent read started while B is in flight).
   *
   * RED at dfb2d0385 (bCallCount==1). GREEN at current head.
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
    const { render, getFetchOlderArchived, unmount } = mountHook("chan-a", qc);

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

    // B's first read must have been attempted (bCallCount >= 1).
    assert.ok(
      bCallCount >= 1,
      `B must have started its first read before the lock probe — got bCallCount=${bCallCount}`,
    );
    const bCallCountBeforeAResolve = bCallCount;

    // Step 5: Resolve A's decrypt. At dfb2d0385 this writes
    // ps.hasOlderArchived=false (if no post-ingest check) and/or clears
    // ps.isFetching (if finally is unguarded).
    resolveADecrypt();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // Step 6: LOCK PROBE — B still holds the lock (B's first read is deferred).
    // Call fetchOlderArchived directly. With correct protection, B's lock is
    // intact and this call returns immediately without a Tauri read — bCallCount
    // must NOT increase. If A stole the lock, this call acquires it and starts
    // a new Tauri read before B's deferred first read resolves (bCallCount jumps).
    const fetchFn = getFetchOlderArchived();
    if (fetchFn) {
      await act(async () => {
        await fetchFn();
      });
    }

    assert.equal(
      bCallCount,
      bCallCountBeforeAResolve,
      `Lock probe must not start a new B read while B holds the lock (bCallCount=${bCallCount}, expected ${bCallCountBeforeAResolve}). If A's stale finally cleared the lock, this concurrent call would start a duplicate read.`,
    );

    // Step 7: Now resolve B's first Tauri read.
    resolveBFirstRead();

    // Let B's loop run.
    await settle(10);

    // Step 8: B must have made at least 2 Tauri reads.
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

  /**
   * Regression (c): concurrent fetches during pending backfill — only one
   * same-generation call may acquire the lock after backfill resolves.
   *
   * The gap at 4c92a018d: `ps.isFetching` was checked only at the top of
   * fetchOlderArchived, BEFORE `await ps.backfillPromise`. Two callers
   * (eager hydration loop + a concurrent scroll trigger) could both observe
   * isFetching=false, both suspend on the same pending backfill promise, then
   * both resume and proceed past the post-backfill guard (which only checked
   * generation + exhaustion). Both would set isFetching=true and issue
   * readArchivedObserverEventsForChannel from the SAME cursor — duplicating
   * the first page.
   *
   * Fix: after `await ps.backfillPromise`, recheck `ps.isFetching` immediately
   * before acquiring the lock. The second caller finds the lock already taken
   * and returns without reading.
   *
   * The test defers the ARCHIVE response (not just backfill) so we can count
   * reads while the winner's first response is still in flight. If both
   * callers acquired the lock, archiveCallCount will be 2 before the first
   * deferred response is released. With the fix, archiveCallCount is 1.
   *
   * Precise race sequence:
   *   1. Mount on chan-a. Backfill is DEFERRED (backfillDeferred).
   *      Archive responses are also deferred until resolveArchive() is called.
   *   2. Hydration loop call #1 suspends on backfill.
   *   3. Inject manual call #2 — it also sees isFetching=false and suspends on
   *      backfill.
   *   4. Resolve backfill. Both calls resume and race to acquire the lock.
   *      - Without fix: both pass the post-backfill guard, both set
   *        isFetching=true, both issue the Tauri read — archiveCallCount == 2.
   *      - With fix: one passes, sets isFetching=true; the other sees the lock
   *        taken and returns — archiveCallCount == 1.
   *   5. Assert archiveCallCount == 1 before releasing archive response.
   *      (Archive is still deferred, so loop hasn't advanced past page 1 yet —
   *      any count > 1 is purely from the concurrent race, not loop progress.)
   *
   * VERIFIED: removing the post-backfill `ps.isFetching` recheck causes
   * archiveCallCount == 2 at step 5. GREEN at current head.
   */
  it("test_two_concurrent_fetches_during_backfill_only_one_proceeds", async () => {
    let resolveBackfill;
    const backfillDeferred = new Promise((resolve) => {
      resolveBackfill = resolve;
    });

    // Defer ALL archive responses until we release them. This way, if two
    // callers both acquire the lock, archiveCallCount jumps to 2 before we
    // release the response — and we can catch it unambiguously.
    let resolveArchive;
    const archiveDeferred = new Promise((resolve) => {
      resolveArchive = resolve;
    });

    let archiveCallCount = 0;

    setIpcHandler("list_save_subscriptions", async () =>
      makeOwnerPSubResponse(),
    );
    // Defer readUnindexedObserverRows to simulate a pending backfill.
    setIpcHandler("read_unindexed_observer_rows", async () => {
      await backfillDeferred;
      return [];
    });
    setIpcHandler("index_observer_channel_id", async () => null);
    setIpcHandler("read_archived_observer_events_for_channel", async (args) => {
      if (args.channelId === "chan-a") {
        archiveCallCount++;
        // Hold this response until we explicitly release it so we can
        // count concurrent reads before any result is returned.
        await archiveDeferred;
        return Array.from({ length: 200 }, (_, i) =>
          JSON.stringify(makeArchivedRow(i, "chan-a")),
        );
      }
      return [];
    });
    setIpcHandler("decrypt_observer_event", async (args) => {
      try {
        const event = JSON.parse(args.eventJson);
        return JSON.parse(event.content);
      } catch {
        return { kind: "telemetry", channelId: null };
      }
    });

    const qc = makeQueryClient();
    const { render, getFetchOlderArchived, unmount } = mountHook("chan-a", qc);

    // Step 1-2: Mount on chan-a. Backfill is in flight (deferred).
    // Hydration loop call #1 enters fetchOlderArchived and suspends on backfill.
    await render("chan-a");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    // Step 3: Inject call #2 while backfill is still pending and call #1 is
    // suspended. Both observe isFetching=false here.
    const fetchFn = getFetchOlderArchived();
    let call2Promise;
    if (fetchFn) {
      // Do NOT await yet — let it run concurrently with call #1.
      call2Promise = fetchFn();
    }

    // Let call #2 reach its backfill await before we resolve backfill.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 5));
    });

    // Step 4: Resolve backfill. Both calls resume and race to acquire lock.
    resolveBackfill();

    // Yield to let both calls advance past the post-backfill guard and issue
    // their Tauri reads (or be blocked by the lock recheck).
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // Step 5: Archive responses are still deferred — loop cannot have advanced
    // past page 1. Any archiveCallCount > 1 here is purely from concurrent
    // reads racing through the backfill await without a lock recheck.
    assert.equal(
      archiveCallCount,
      1,
      `Exactly one archive read must start after backfill resolves — got ${archiveCallCount}. Without the post-backfill isFetching recheck, both concurrent callers acquire the lock and both issue a Tauri read (archiveCallCount == 2).`,
    );

    // Release archive responses so the lock holder can finish and the test
    // can unmount cleanly.
    resolveArchive();

    // Wait for call #2 to settle as well.
    if (call2Promise) {
      await act(async () => {
        await call2Promise;
      });
    }

    await settle(5);
    await unmount();
  });

  /**
   * Regression (d): A→B→A — old-A's stale in-flight request must not corrupt
   * fresh-A's paging state when the user returns to channel A.
   *
   * The gap in the channel-string equality check (af45fbf05): using
   * `requestChannelId === ps.activeChannelId` as the ownership test fails when
   * the user navigates A→B→A. Old-A's request sees `ps.activeChannelId === "A"`
   * after the second return to A — so every check passes. Old-A can mark fresh-A
   * exhausted and its finally releases fresh-A's lock.
   *
   * With resetGeneration each applyChannelReset() call increments the counter,
   * so A(gen=1) → B(gen=2) → A(gen=3): old-A snapshotted gen=1, which never
   * equals gen=3, so all writes are discarded regardless of channel name.
   *
   * Race sequence:
   *   1. Mount on chan-a (gen=1). A's Tauri read = 1 row (short page). Ingest
   *      starts. A's decrypt is DEFERRED.
   *   2. Switch to chan-b (gen=2). B's paging starts.
   *   3. Switch BACK to chan-a (gen=3). Fresh-A's hydration starts. Fresh-A's
   *      first Tauri read is DEFERRED (freshAReadDeferred).
   *   4. Resolve old-A's decrypt. Old-A hits short-page branch.
   *      - Without generation: old-A sees ps.activeChannelId==="chan-a", writes
   *        ps.hasOlderArchived=false and clears ps.isFetching.
   *      - With generation: gen=1 !== gen=3, writes discarded.
   *   5. LOCK PROBE: call fetchOlderArchived directly. Fresh-A holds the lock
   *      (its deferred read is in flight). With correct protection the probe
   *      returns immediately (freshACallCount unchanged). Without it (old-A's
   *      finally stole the lock), the probe starts a duplicate read.
   *   6. Resolve fresh-A's first read (full page). Fresh-A loop continues.
   *   7. Assert freshACallCount >= 2 (fresh-A ran past page 1).
   *
   * VERIFIED: this test is RED when activeChannelId string equality replaces
   * resetGeneration (old-A passes every check, marks fresh-A exhausted at step 4).
   * GREEN at current head.
   */
  it("test_A_B_A_old_request_cannot_corrupt_fresh_A_state", async () => {
    let resolveOldADecrypt;
    const oldADecryptDeferred = new Promise((resolve) => {
      resolveOldADecrypt = resolve;
    });

    let resolveFreshAFirstRead;
    const freshAFirstReadDeferred = new Promise((resolve) => {
      resolveFreshAFirstRead = resolve;
    });

    const PAGE_SIZE = 200;
    const makePage = (channelId, offset) =>
      Array.from({ length: PAGE_SIZE }, (_, i) =>
        makeArchivedRow(offset + i, channelId),
      );

    let oldADecryptStarted = false;
    // Track calls per channel / phase. We only care about chan-a reads on fresh-A.
    let freshACallCount = 0;
    // After we switch back to A (gen=3), track reads for that phase.
    let onFreshA = false;

    setIpcHandler("list_save_subscriptions", async () =>
      makeOwnerPSubResponse(),
    );
    setIpcHandler("read_unindexed_observer_rows", async () => []);
    setIpcHandler("index_observer_channel_id", async () => null);
    setIpcHandler("read_archived_observer_events_for_channel", async (args) => {
      if (args.channelId === "chan-a") {
        if (!onFreshA) {
          // Old-A's read: 1 row = short page.
          return [JSON.stringify(makeArchivedRow(1, "chan-a"))];
        }
        // Fresh-A's reads.
        freshACallCount++;
        if (freshACallCount === 1) {
          // Defer fresh-A's first read.
          await freshAFirstReadDeferred;
          return makePage("chan-a", 200).map((r) => JSON.stringify(r)); // full
        }
        if (freshACallCount <= 4)
          return makePage("chan-a", freshACallCount * 200).map((r) =>
            JSON.stringify(r),
          );
        return [JSON.stringify(makeArchivedRow(9999, "chan-a"))]; // exhaust
      }
      if (args.channelId === "chan-b") {
        // B gets one short page (we don't care about B's progress here).
        return [JSON.stringify(makeArchivedRow(50, "chan-b"))];
      }
      return [];
    });
    setIpcHandler("decrypt_observer_event", async (args) => {
      try {
        const event = JSON.parse(args.eventJson);
        const parsed = JSON.parse(event.content);
        if (parsed.channelId === "chan-a" && !oldADecryptStarted && !onFreshA) {
          oldADecryptStarted = true;
          await oldADecryptDeferred; // block OLD A's decrypt
        }
        return parsed;
      } catch {
        return { kind: "telemetry", channelId: null };
      }
    });

    const qc = makeQueryClient();
    const { render, getFetchOlderArchived, unmount } = mountHook("chan-a", qc);

    // Step 1: Mount on chan-a (gen=1). Old-A's Tauri read returns 1 row.
    // Ingest starts, decrypt blocks.
    await render("chan-a");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    // Step 2: Switch to chan-b (gen=2).
    await render("chan-b");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // Step 3: Switch back to chan-a (gen=3). Fresh-A hydration starts.
    onFreshA = true;
    await render("chan-a");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    // Fresh-A must have started its first (deferred) read.
    assert.ok(
      freshACallCount >= 1,
      `fresh-A must have started its first read — got freshACallCount=${freshACallCount}`,
    );
    const freshACountBeforeOldResolve = freshACallCount;

    // Step 4: Resolve old-A's decrypt. Without generation check, old-A's
    // post-ingest branch writes ps.hasOlderArchived=false (marks fresh-A
    // exhausted) and its finally clears ps.isFetching (steals fresh-A's lock).
    resolveOldADecrypt();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 10));
    });

    // Step 5: LOCK PROBE — fresh-A holds the lock (first read deferred).
    // A concurrent fetchOlderArchived call must be rejected (lock held).
    // If old-A stole the lock, this probe starts a duplicate read (freshACallCount
    // jumps before the deferred first read resolves).
    const fetchFn = getFetchOlderArchived();
    if (fetchFn) {
      await act(async () => {
        await fetchFn();
      });
    }

    assert.equal(
      freshACallCount,
      freshACountBeforeOldResolve,
      `Lock probe must not start a new fresh-A read while fresh-A holds the lock (freshACallCount=${freshACallCount}, expected ${freshACountBeforeOldResolve}). Old-A's stale finally stole the lock (A→B→A channel-string equality bug).`,
    );

    // Step 6: Resolve fresh-A's first read (full page). Loop continues.
    resolveFreshAFirstRead();
    await settle(10);

    // Step 7: Fresh-A must have made at least 2 reads (loop continued past page 1).
    // Without generation check, old-A wrote ps.hasOlderArchived=false before
    // fresh-A's first page resolved, causing the loop to exit — freshACallCount==1.
    assert.ok(
      freshACallCount >= 2,
      `fresh-A must read at least 2 pages — got ${freshACallCount}. Old-A's stale writes (A→B→A) would leave freshACallCount==1.`,
    );

    await unmount();
  });
});
