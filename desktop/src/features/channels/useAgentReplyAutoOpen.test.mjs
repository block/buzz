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
/** The expanded DM a mid-send agent mention creates (usePrepareDmSendChannel). */
const EXPANDED_DM_ID = "99999999-8888-7777-6666-555555555555";
const AGENT_PUBKEY = "a".repeat(64);
const ROOT_ID = "1".repeat(64);
const OTHER_ROOT_ID = "2".repeat(64);
const REPLY_ID = "3".repeat(64);

const CHANNEL = { id: CHANNEL_ID, name: "agents", channelType: "channel" };
const EXPANDED_DM = {
  id: EXPANDED_DM_ID,
  name: "agent dm",
  channelType: "dm",
};

/** The user's top-level message that p-tags the agent. */
function topLevelEvent({ id = ROOT_ID, channelId = CHANNEL_ID } = {}) {
  return {
    id,
    pubkey: "b".repeat(64),
    created_at: 100,
    kind: KIND_STREAM_MESSAGE,
    tags: [
      ["h", channelId],
      ["p", AGENT_PUBKEY],
    ],
    content: "@agent hello",
    sig: "0".repeat(128),
  };
}

/** The agent's hidden NIP-10 reply to that message. */
function agentReplyEvent({ rootId = ROOT_ID, channelId = CHANNEL_ID } = {}) {
  return {
    id: REPLY_ID,
    pubkey: AGENT_PUBKEY,
    created_at: 101,
    kind: KIND_STREAM_MESSAGE,
    tags: [
      ["h", channelId],
      ["e", rootId, "", "reply"],
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

  function Inner({ hasActiveAuxiliaryPanel, activeChannel }) {
    const result = useAgentReplyAutoOpen({
      activeChannel,
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

  const render = async (hasActiveAuxiliaryPanel, activeChannel = CHANNEL) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(Inner, {
            hasActiveAuxiliaryPanel,
            activeChannel,
          }),
        ),
      );
    });
  };

  await render(false);

  return {
    calls,
    render,
    /** Record the user's top-level agent-targeted send. */
    sendTopLevel: async (event = topLevelEvent()) => {
      await act(async () => {
        captured(event);
      });
    },
    /** Record a send that failed — the callback receives null. */
    sendFailed: async () => {
      await act(async () => {
        captured(null);
      });
    },
    /**
     * Deliver the agent reply with `setTimeout(fn, 0)` captured instead of
     * scheduled, and return the deferred callbacks the hook queued.
     *
     * The interception is scoped to this one synchronous emit so React's own
     * scheduling is never affected.
     */
    emitReplyCapturingTimer: (reply = agentReplyEvent()) => {
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
        for (const fn of [...liveCallbacks]) fn(reply);
      } finally {
        globalThis.setTimeout = realSetTimeout;
      }
      return deferred;
    },
    /**
     * Emit a reply with the real timer, then flush the macrotask queue. Used
     * by the unmount test, where the point is that a genuinely scheduled
     * timer must be cancelled rather than merely captured.
     */
    emitReplyWithRealTimer: (reply = agentReplyEvent()) => {
      for (const fn of [...liveCallbacks]) fn(reply);
    },
    flushTimers: async () => {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 1));
      });
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

  it("cancels a scheduled open when the screen unmounts first", async () => {
    const harness = await mountAutoOpen();
    await harness.sendTopLevel();

    // Real timer this time: the point is that an actually-scheduled callback
    // is cleared on unmount, not merely withheld by the test.
    harness.emitReplyWithRealTimer();
    await harness.unmount();
    await harness.flushTimers();

    assert.deepEqual(
      harness.calls,
      [],
      "an unmounted screen must not run the deferred setters",
    );
  });
});

describe("useAgentReplyAutoOpen pending-trigger lifecycle", () => {
  beforeEach(() => {
    mock.restoreAll();
  });

  afterEach(() => {
    mock.restoreAll();
  });

  it("opens in the expanded DM the send was redirected to", async () => {
    const harness = await mountAutoOpen();

    // Mentioning an agent in a DM creates an expanded DM mid-send, so the
    // published event carries the new channel in its h tag. The pane then
    // navigates there — after the send has already resolved.
    await harness.sendTopLevel(topLevelEvent({ channelId: EXPANDED_DM_ID }));
    await harness.render(false, EXPANDED_DM);

    const deferred = harness.emitReplyCapturingTimer(
      agentReplyEvent({ channelId: EXPANDED_DM_ID }),
    );
    assert.equal(
      deferred.length,
      1,
      "the redirected send should still arm the trigger",
    );

    await harness.runDeferred(deferred);

    assert.deepEqual(
      harness.calls.find(([name]) => name === "setOpenThreadHeadId"),
      ["setOpenThreadHeadId", ROOT_ID],
      "the expanded DM's reply should open its own thread",
    );

    await harness.unmount();
  });

  it("ignores a reply that arrives on a different channel", async () => {
    const harness = await mountAutoOpen();
    await harness.sendTopLevel();

    const deferred = harness.emitReplyCapturingTimer(
      agentReplyEvent({ channelId: EXPANDED_DM_ID }),
    );

    assert.deepEqual(
      deferred,
      [],
      "a reply on another channel must not schedule an open",
    );
    assert.deepEqual(harness.calls, [], "and must not touch panel state");

    await harness.unmount();
  });

  it("clears the pending trigger when a later send fails", async () => {
    const harness = await mountAutoOpen();

    // Send A succeeds and arms the trigger.
    await harness.sendTopLevel();
    // Send B fails. The user is waiting on B, so a late reply to A must not
    // steal the panel.
    await harness.sendFailed();

    const deferred = harness.emitReplyCapturingTimer();

    assert.deepEqual(
      deferred,
      [],
      "a failed send should have cleared the earlier trigger",
    );
    assert.deepEqual(harness.calls, [], "and no panel state should change");

    await harness.unmount();
  });

  it("does not arm on a reply to an unrelated root", async () => {
    const harness = await mountAutoOpen();
    await harness.sendTopLevel();

    const deferred = harness.emitReplyCapturingTimer(
      agentReplyEvent({ rootId: OTHER_ROOT_ID }),
    );

    assert.deepEqual(
      deferred,
      [],
      "a reply rooted elsewhere must not open a thread",
    );

    await harness.unmount();
  });
});
