/**
 * Seam test for useChannelPaneHandlers' top-level send path.
 *
 * `handleSendMessage` sits where two independently-motivated changes meet: the
 * composer's `forceRest` flag, which forces HTTP publication once background
 * link-preview preparation has run (`useMentionSendFlow` passes
 * `draft.preparedLinkPreviews != null`), and the agent-reply auto-open trigger,
 * which needs the published `RelayEvent` handed back after the send resolves.
 *
 * Neither concern is visible from the other's tests, and a merge that keeps one
 * silently drops the other: losing `forceRest` reroutes prepared-preview sends
 * back to WebSocket, and losing the callback disables auto-open entirely. Both
 * failures are invisible to typechecking because every argument is optional.
 * This file pins the seam directly.
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

import { useChannelPaneHandlers } from "./useChannelPaneHandlers.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds.ts";

// ── Constants ────────────────────────────────────────────────────────────────

const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";
const AGENT_PUBKEY = "a".repeat(64);
const ROOT_ID = "1".repeat(64);
const REPLY_ID = "3".repeat(64);

const CHANNEL = { id: CHANNEL_ID, name: "agents", channelType: "channel" };

function publishedEvent() {
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

/** A mutation stub shaped like the TanStack Query results the hook consumes. */
function stubMutation(mutateAsync = async () => undefined) {
  return { mutateAsync, isPending: false };
}

async function mountHandlers({
  sendResult = publishedEvent(),
  sendError,
} = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });

  /** Every argument object `sendMessageMutation.mutateAsync` received. */
  const sendCalls = [];
  const liveCallbacks = [];
  const panelCalls = [];

  mock.method(relayClient, "subscribeToChannelLive", async (_id, onEvent) => {
    liveCallbacks.push(onEvent);
    return async () => {
      const i = liveCallbacks.indexOf(onEvent);
      if (i >= 0) liveCallbacks.splice(i, 1);
    };
  });
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "setVisibleChannelId", () => {});

  const sendMessageMutation = stubMutation(async (input) => {
    sendCalls.push(input);
    if (sendError) throw sendError;
    return sendResult;
  });

  let handlers = null;

  function Inner() {
    handlers = useChannelPaneHandlers({
      agentReplyAutoOpen: [CHANNEL, new Set([AGENT_PUBKEY]), null],
      hasActiveAuxiliaryPanel: false,
      deleteMessageMutation: stubMutation(),
      editMessageMutation: stubMutation(),
      editTargetId: null,
      expandedThreadReplyIds: new Set(),
      getFirstReplyIdForMessage: () => null,
      getReplyDescendantIdsForMessage: () => [],
      markRevealedRepliesRead: () => {},
      profiles: undefined,
      recordThreadInteraction: () => {},
      onOptimisticOpenThreadHeadIdChange: (v) =>
        panelCalls.push(["setOptimisticOpenThreadHeadId", v]),
      onRequestEmptyEditDelete: () => {},
      openThreadHeadId: null,
      sendMessageMutation,
      setExpandedThreadReplyIds: (v) =>
        panelCalls.push(["setExpandedThreadReplyIds", v]),
      setEditTargetId: () => {},
      setOpenThreadHeadId: (v) => panelCalls.push(["setOpenThreadHeadId", v]),
      setThreadReplyTargetId: (v) =>
        panelCalls.push(["setThreadReplyTargetId", v]),
      setThreadScrollTargetId: (v) =>
        panelCalls.push(["setThreadScrollTargetId", v]),
      threadReplyTargetId: null,
      toggleReactionMutation: stubMutation(),
    });
    return null;
  }

  const container = document.createElement("div");
  const root = createRoot(container);

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Inner),
      ),
    );
  });

  return {
    sendCalls,
    panelCalls,
    /** Invoke the composer's send path with the full six-argument signature. */
    send: async ({ forceRest } = {}) => {
      await act(async () => {
        await handlers.handleSendMessage(
          "@agent hello",
          [AGENT_PUBKEY],
          undefined,
          CHANNEL_ID,
          null,
          forceRest,
        );
      });
    },
    sendExpectingFailure: async ({ forceRest } = {}) => {
      let thrown = null;
      await act(async () => {
        try {
          await handlers.handleSendMessage(
            "@agent hello",
            [AGENT_PUBKEY],
            undefined,
            CHANNEL_ID,
            null,
            forceRest,
          );
        } catch (error) {
          thrown = error;
        }
      });
      return thrown;
    },
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

describe("useChannelPaneHandlers send seam", () => {
  beforeEach(() => {
    mock.restoreAll();
  });

  afterEach(() => {
    mock.restoreAll();
  });

  it("forwards forceRest to the send mutation", async () => {
    const harness = await mountHandlers();

    await harness.send({ forceRest: true });

    assert.equal(harness.sendCalls.length, 1, "one send should be issued");
    assert.equal(
      harness.sendCalls[0].forceRest,
      true,
      "forceRest must reach mutateAsync, or prepared-preview sends fall back to WebSocket",
    );
    assert.equal(harness.sendCalls[0].channelId, CHANNEL_ID);

    await harness.unmount();
  });

  it("passes forceRest through unset when the composer omits it", async () => {
    const harness = await mountHandlers();

    await harness.send();

    assert.equal(harness.sendCalls.length, 1);
    assert.equal(
      harness.sendCalls[0].forceRest,
      undefined,
      "an omitted flag must stay omitted rather than defaulting to true",
    );

    await harness.unmount();
  });

  it("still arms auto-open on a forceRest send", async () => {
    const harness = await mountHandlers();

    await harness.send({ forceRest: true });

    const deferred = harness.emitReplyCapturingTimer();
    assert.equal(
      deferred.length,
      1,
      "the returned event must reach the auto-open trigger",
    );

    await harness.runDeferred(deferred);

    assert.deepEqual(
      harness.panelCalls.find(([name]) => name === "setOpenThreadHeadId"),
      ["setOpenThreadHeadId", ROOT_ID],
      "the agent's reply should open the thread rooted at the sent message",
    );

    await harness.unmount();
  });

  it("rethrows send failures after clearing the trigger", async () => {
    const failure = new Error("relay rejected");
    const harness = await mountHandlers({ sendError: failure });

    const thrown = await harness.sendExpectingFailure({ forceRest: true });

    assert.equal(thrown, failure, "the composer must still see the error");
    assert.equal(
      harness.sendCalls[0].forceRest,
      true,
      "forceRest is forwarded on the failing path too",
    );

    const deferred = harness.emitReplyCapturingTimer();
    assert.deepEqual(deferred, [], "a failed send must leave nothing armed");

    await harness.unmount();
  });
});
