/**
 * Mounted-hook tests for useAgentReplyAutoOpen.
 *
 * The auto-open policy itself is a pure function covered by
 * messages/lib/agentReplyAutoOpen.test.mjs. Nothing there can reach the part
 * this file exists for: the hook does not apply the decision inline, it defers
 * it through `window.setTimeout(..., 0)` so the live-subscription callback is
 * not the thing that mutates panel state. That deferral opens a window between
 * "policy said open" and "setters actually run", and the browser can process a
 * user click inside it (input tasks are serviced ahead of timer tasks). A
 * panel the user opened in that window must win over the deferred auto-open.
 *
 * The window is only expressible with the timer under test control, so the
 * emit below runs with `setTimeout(fn, 0)` captured rather than scheduled:
 * that models "scheduled but not yet fired" exactly, with no race. Letting the
 * real timer run would make the test a coin flip on macrotask ordering and it
 * would pass for the wrong reason on a fast machine.
 *
 * ── Harness shape ────────────────────────────────────────────────────────────
 * DOM shim (shared with the observed-unread tests) → __TAURI_INTERNALS__
 * interception → production imports → createRoot/act inside a
 * QueryClientProvider. relayClient's live-subscription entry points are
 * replaced with mock.method so no socket is opened and the test owns the
 * callback the hook registers.
 */

import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach, mock } from "node:test";

import { installDOMShim } from "./observedUnreadTestHarness.mjs";

// DOM shim must run before any React import.
installDOMShim();

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd) => {
    if (cmd === "get_channel_window") return Promise.resolve([]);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback: () => 1,
};

// ── Production imports (after shims) ─────────────────────────────────────────

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { useAgentReplyAutoOpen } from "./useAgentReplyAutoOpen.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds.ts";

// ── Constants ────────────────────────────────────────────────────────────────

const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";
const AGENT_PUBKEY = "a".repeat(64);
const ROOT_ID = "1".repeat(64);
const REPLY_ID = "3".repeat(64);

const CHANNEL = { id: CHANNEL_ID, name: "agents", channelType: "channel" };

/** The user's top-level message that p-tags the agent. */
function topLevelEvent() {
  return {
    id: ROOT_ID,
    pubkey: "b".repeat(64),
    created_at: 100,
    kind: KIND_STREAM_MESSAGE,
    tags: [
      ["h", CHANNEL_ID],
      ["p", AGENT_PUBKEY],
    ],
    content: "@agent hello",
    sig: "0".repeat(128),
  };
}

/** The agent's hidden NIP-10 reply to that message. */
function agentReplyEvent() {
  return {
    id: REPLY_ID,
    pubkey: AGENT_PUBKEY,
    created_at: 101,
    kind: KIND_STREAM_MESSAGE,
    tags: [
      ["h", CHANNEL_ID],
      ["e", ROOT_ID, "", "reply"],
    ],
    content: "on it",
    sig: "0".repeat(128),
  };
}

// ── Harness ──────────────────────────────────────────────────────────────────

async function mountAutoOpen() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  /** Every setter the deferred batch is allowed to call, recorded in order. */
  const calls = [];
  /** Live-subscription callbacks the hook registered. */
  const liveCallbacks = [];

  mock.method(relayClient, "subscribeToChannelLive", async (_id, onEvent) => {
    liveCallbacks.push(onEvent);
    return async () => {
      const i = liveCallbacks.indexOf(onEvent);
      if (i >= 0) liveCallbacks.splice(i, 1);
    };
  });
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "setVisibleChannelId", () => {});

  let captured = null;

  function Inner({ hasActiveAuxiliaryPanel }) {
    const result = useAgentReplyAutoOpen({
      activeChannel: CHANNEL,
      agentPubkeys: new Set([AGENT_PUBKEY]),
      hasActiveAuxiliaryPanel,
      relaySelfPubkey: null,
      setExpandedThreadReplyIds: (v) =>
        calls.push(["setExpandedThreadReplyIds", v]),
      setOpenThreadHeadId: (v) => calls.push(["setOpenThreadHeadId", v]),
      setOptimisticOpenThreadHeadId: (v) =>
        calls.push(["setOptimisticOpenThreadHeadId", v]),
      setThreadReplyTargetId: (v) => calls.push(["setThreadReplyTargetId", v]),
      setThreadScrollTargetId: (v) =>
        calls.push(["setThreadScrollTargetId", v]),
    });
    captured = result.handleTopLevelMessageSent;
    return null;
  }

  const container = document.createElement("div");
  const root = createRoot(container);

  const render = async (hasActiveAuxiliaryPanel) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Inner, { hasActiveAuxiliaryPanel }),
        ),
      );
    });
  };

  await render(false);

  return {
    calls,
    render,
    /** Record the user's top-level agent-targeted send. */
    sendTopLevel: async () => {
      await act(async () => {
        captured(topLevelEvent());
      });
    },
    /**
     * Deliver the agent reply with `setTimeout(fn, 0)` captured instead of
     * scheduled, and return the deferred callbacks the hook queued.
     *
     * The interception is scoped to this one synchronous emit so React's own
     * scheduling is never affected.
     */
    emitReplyCapturingTimer: () => {
      const deferred = [];
      const realSetTimeout = globalThis.setTimeout;
      globalThis.setTimeout = (fn, ms, ...rest) => {
        if (ms === 0) {
          deferred.push(fn);
          return deferred.length;
        }
        return realSetTimeout(fn, ms, ...rest);
      };
      try {
        for (const fn of [...liveCallbacks]) fn(agentReplyEvent());
      } finally {
        globalThis.setTimeout = realSetTimeout;
      }
      return deferred;
    },
    runDeferred: async (deferred) => {
      await act(async () => {
        for (const fn of deferred) fn();
      });
    },
    unmount: async () => {
      await act(async () => root.unmount());
    },
  };
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("useAgentReplyAutoOpen deferred apply", () => {
  beforeEach(() => {
    mock.restoreAll();
  });

  afterEach(() => {
    mock.restoreAll();
  });

  it("opens the thread when no panel is opened during the deferred window", async () => {
    const harness = await mountAutoOpen();
    await harness.sendTopLevel();

    const deferred = harness.emitReplyCapturingTimer();
    assert.equal(deferred.length, 1, "reply should schedule one deferred open");

    await harness.runDeferred(deferred);

    assert.deepEqual(
      harness.calls.map(([name]) => name).sort(),
      [
        "setExpandedThreadReplyIds",
        "setOpenThreadHeadId",
        "setOptimisticOpenThreadHeadId",
        "setThreadReplyTargetId",
        "setThreadScrollTargetId",
      ],
      "the full open batch should run",
    );
    assert.deepEqual(
      harness.calls.find(([name]) => name === "setOpenThreadHeadId"),
      ["setOpenThreadHeadId", ROOT_ID],
      "the opened thread should be the root the user targeted",
    );

    await harness.unmount();
  });

  it("does not overwrite a panel the user opened inside the deferred window", async () => {
    const harness = await mountAutoOpen();
    await harness.sendTopLevel();

    // Policy evaluates with no panel open and schedules the deferred batch.
    const deferred = harness.emitReplyCapturingTimer();
    assert.equal(deferred.length, 1, "reply should schedule one deferred open");
    assert.deepEqual(harness.calls, [], "nothing applies before the timer");

    // The user opens an auxiliary panel before the timer fires.
    await harness.render(true);

    await harness.runDeferred(deferred);

    assert.deepEqual(
      harness.calls,
      [],
      "the deferred batch must bail rather than displace the user's panel",
    );

    await harness.unmount();
  });
});
