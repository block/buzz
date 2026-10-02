import assert from "node:assert/strict";
import { after, afterEach, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

import {
  clearActiveTurnsForAgent,
  getActiveTurnsForAgent,
  resetActiveAgentTurnsStore,
  syncAgentTurnsFromEvents,
  useActiveAgentTurns,
} from "@/features/agents/activeAgentTurnsStore.ts";
import {
  getAgentObserverSnapshot,
  getAgentTranscript,
  resetAgentObserverStore,
  syncAgentObserverEvents,
} from "@/features/agents/observerRelayStore.ts";
import {
  useAgentTranscript,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents.ts";
import { resolveProfileActivityAgent } from "./profileActivityAgent.ts";
import { useProfileActivityFeedScope } from "./profileActivityFeedScope.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
const { act, cleanup, renderHook } = await import("@testing-library/react");
const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);
const nativeCalls = [];
window.__TAURI_INTERNALS__ = {
  invoke: async (...args) => {
    nativeCalls.push(args);
    throw new Error(
      "Reading stored profile history must not invoke native APIs",
    );
  },
};

beforeEach(() => {
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  nativeCalls.length = 0;
});
afterEach(() => {
  cleanup();
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  assert.deepEqual(
    nativeCalls,
    [],
    "history reads must not start subscriptions",
  );
});
after(() => dom.window.close());

function resolveAgent(relayStatus, overrides = {}) {
  return resolveProfileActivityAgent({
    effectivePubkey: AGENT,
    isBot: true,
    managedAgent: undefined,
    profile: { displayName: "Scout" },
    relayAgent: relayStatus
      ? { name: "Scout", status: relayStatus }
      : undefined,
    viewerIsOwner: true,
    ...overrides,
  });
}

function event(seq, kind, channelId = "general", payload = {}) {
  return {
    seq,
    timestamp: new Date(Date.UTC(2026, 8, 2) + seq * 1000).toISOString(),
    kind,
    agentIndex: 0,
    channelId,
    sessionId: "session",
    turnId: "turn",
    payload,
  };
}

function message(seq, channelId) {
  return event(seq, "acp_read", channelId, {
    method: "session/update",
    params: {
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "Completed the review." },
      },
    },
  });
}

function useScope(agent) {
  const turns = useActiveAgentTurns(agent?.pubkey);
  return useProfileActivityFeedScope(agent, turns);
}

const idleScope = {
  channelIds: [],
  hasFeedContent: false,
  isLive: false,
  latestActivityAtByChannel: {},
  preferredChannelId: null,
};

for (const [evidence, agent] of [
  ["unknown", resolveAgent("unknown")],
  ["absent", resolveAgent(undefined)],
  ["offline", resolveAgent("offline")],
  ["online", resolveAgent("online")],
  [
    "local stopped",
    resolveAgent("unknown", {
      managedAgent: { pubkey: AGENT, name: "Local Scout", status: "stopped" },
    }),
  ],
]) {
  test(`${evidence} evidence retains completed raw history with no active turns`, () => {
    const completed = event(1, "turn_completed");
    syncAgentObserverEvents(AGENT, [completed]);
    syncAgentTurnsFromEvents(AGENT, [completed]);
    assert.deepEqual(getActiveTurnsForAgent(AGENT), []);
    assert.deepEqual(getAgentTranscript(AGENT), []);

    const { result } = renderHook(() => useScope(agent));
    assert.deepEqual(result.current, {
      channelIds: ["general"],
      hasFeedContent: true,
      isLive: false,
      latestActivityAtByChannel: { general: Date.parse(completed.timestamp) },
      preferredChannelId: "general",
    });
    if (evidence === "unknown" || evidence === "absent") {
      assert.equal(agent.status, "unknown", "history is not liveness evidence");
    }
  });

  test(`${evidence} evidence reads stored transcript as well as raw events`, () => {
    // The last raw event has no transcript row. The existing scope contract
    // prefers the last transcript channel; dropping either read is detectable.
    syncAgentObserverEvents(AGENT, [
      message(1, "general"),
      event(2, "turn_completed", "general"),
      event(3, "turn_completed", "random"),
    ]);
    const transcript = getAgentTranscript(AGENT);
    assert.equal(transcript.length, 1);
    assert.equal(transcript[0].channelId, "general");
    const { result } = renderHook(() => useScope(agent));
    assert.deepEqual(result.current.channelIds, ["general", "random"]);
    assert.equal(result.current.preferredChannelId, "general");
    assert.equal(result.current.hasFeedContent, true);
    assert.equal(result.current.isLive, false);
  });
}

for (const evidence of ["unknown", undefined]) {
  for (const finish of ["completion", "clear"]) {
    test(`${evidence ?? "absent"} evidence keeps history after active turn ${finish}`, () => {
      const agent = resolveAgent(evidence);
      const started = event(1, "turn_started");
      syncAgentObserverEvents(AGENT, [started, message(2, "general")]);
      syncAgentTurnsFromEvents(AGENT, [started]);
      const { result } = renderHook(() => useScope(agent));
      assert.equal(result.current.isLive, true);
      assert.deepEqual(result.current.channelIds, ["general"]);

      act(() => {
        const completed = event(3, "turn_completed");
        syncAgentObserverEvents(AGENT, [completed]);
        if (finish === "clear") clearActiveTurnsForAgent(AGENT);
        else syncAgentTurnsFromEvents(AGENT, [completed]);
      });
      assert.deepEqual(getActiveTurnsForAgent(AGENT), []);
      assert.equal(result.current.isLive, false);
      assert.equal(result.current.hasFeedContent, true);
      assert.deepEqual(result.current.channelIds, ["general"]);
      assert.equal(result.current.preferredChannelId, "general");
      assert.equal(getAgentObserverSnapshot(AGENT).events.length, 3);
      assert.equal(getAgentTranscript(AGENT).length, 2);
      assert.equal(agent.status, "unknown");
    });
  }
}

test("store updates, evidence loss, agent switches and reset preserve history boundaries", () => {
  const { result, rerender } = renderHook(({ agent }) => useScope(agent), {
    initialProps: { agent: resolveAgent("online") },
  });
  assert.deepEqual(result.current, idleScope);
  act(() => syncAgentObserverEvents(AGENT, [message(1, "general")]));
  assert.equal(result.current.hasFeedContent, true);

  for (const evidence of ["unknown", undefined, "offline"]) {
    rerender({ agent: resolveAgent(evidence) });
    assert.equal(result.current.hasFeedContent, true);
    assert.equal(result.current.isLive, false);
    assert.deepEqual(result.current.channelIds, ["general"]);
  }
  rerender({
    agent: resolveAgent(undefined, { effectivePubkey: OTHER_AGENT }),
  });
  assert.deepEqual(result.current, idleScope);
  rerender({ agent: resolveAgent(undefined) });
  assert.equal(result.current.hasFeedContent, true);
  act(() => resetAgentObserverStore());
  assert.deepEqual(result.current, idleScope, "community reset hides old data");
});

test("unowned, non-bot and missing identities cannot acquire stored history", () => {
  syncAgentObserverEvents(AGENT, [message(1, "general")]);
  for (const overrides of [
    { viewerIsOwner: false },
    { isBot: false },
    { effectivePubkey: null },
  ]) {
    const agent = resolveAgent("unknown", overrides);
    assert.equal(agent, null);
    const { result, unmount } = renderHook(() => useScope(agent));
    assert.deepEqual(result.current, idleScope);
    unmount();
  }
});

test("idle session readers and profile scope share history without a live subscription", () => {
  syncAgentObserverEvents(AGENT, [
    message(1, "general"),
    event(2, "turn_completed"),
  ]);
  const { result } = renderHook(() => ({
    scope: useScope(resolveAgent("unknown")),
    observer: useObserverEvents(false, AGENT),
    transcript: useAgentTranscript(false, AGENT),
  }));
  assert.equal(result.current.scope.hasFeedContent, true);
  assert.equal(result.current.scope.isLive, false);
  assert.equal(result.current.observer.events.length, 2);
  assert.equal(result.current.transcript.length, 1);
  assert.equal(result.current.observer.connectionState, "idle");
});
