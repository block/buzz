import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/features/agents/ui/useObserverEvents") {
      return { shortCircuit: true, url: "buzz-test-stub:agent-transcript" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-test-stub:agent-transcript") {
      return {
        format: "module",
        shortCircuit: true,
        source: "export function useAgentTranscript() { return []; }\n",
      };
    }
    return nextLoad(url, context);
  },
});

const { BotActivityComposerAction } = await import("./BotActivityBar.tsx");
const { resetActiveAgentTurnsStore, syncAgentTurnsFromEvents } = await import(
  "@/features/agents/activeAgentTurnsStore"
);

const AGENT = "a".repeat(64);
let mounted = null;

afterEach(async () => {
  if (mounted) {
    await act(async () => mounted.root.unmount());
    mounted.container.remove();
    mounted = null;
  }
  resetActiveAgentTurnsStore();
});

function turnEvent(seq, turnId, kind, root) {
  return {
    seq,
    timestamp: new Date(Date.now() + seq).toISOString(),
    kind,
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId,
    payload: { threadRootEventId: root },
  };
}

async function mountBar(sessionPolicy, showChannelThreadCount = true) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  mounted = { container, root };
  await act(async () => {
    root.render(
      React.createElement(BotActivityComposerAction, {
        agents: [{ pubkey: AGENT, name: "Caddie", sessionPolicy }],
        channelId: "channel-1",
        onOpenAgentSession: () => {},
        openAgentSessionPubkey: null,
        showChannelThreadCount,
        workingBotPubkeys: [AGENT],
        variant: "inline",
      }),
    );
  });
  return container;
}

test("thread-policy composer shows thread count only while multiple roots are active", async () => {
  resetActiveAgentTurnsStore();
  syncAgentTurnsFromEvents(AGENT, [
    turnEvent(1, "turn-a", "turn_started", "root-a"),
  ]);
  const container = await mountBar("thread");
  assert.match(container.textContent, /Caddie: Working/);

  await act(async () => {
    syncAgentTurnsFromEvents(AGENT, [
      turnEvent(2, "turn-b", "turn_started", "root-b"),
    ]);
  });
  assert.match(container.textContent, /Caddie is working on 2 threads now/);

  await act(async () => {
    syncAgentTurnsFromEvents(AGENT, [
      turnEvent(3, "turn-b", "turn_completed", "root-b"),
    ]);
  });
  assert.match(container.textContent, /Caddie: Working/);
});

test("channel-policy composer keeps its current label with multiple turns", async () => {
  resetActiveAgentTurnsStore();
  syncAgentTurnsFromEvents(AGENT, [
    turnEvent(1, "turn-a", "turn_started", null),
    turnEvent(2, "turn-b", "turn_started", null),
  ]);
  const container = await mountBar("channel");
  assert.match(container.textContent, /Caddie: Working/);
  assert.doesNotMatch(container.textContent, /threads now/);
});

test("observer thread roots work when the agent has no local policy metadata", async () => {
  resetActiveAgentTurnsStore();
  syncAgentTurnsFromEvents(AGENT, [
    turnEvent(1, "turn-a", "turn_started", "root-a"),
    turnEvent(2, "turn-b", "turn_started", "root-b"),
  ]);
  const container = await mountBar(undefined);
  assert.match(container.textContent, /Caddie is working on 2 threads now/);
});

test("thread reply composer does not show a channel-wide thread count", async () => {
  resetActiveAgentTurnsStore();
  syncAgentTurnsFromEvents(AGENT, [
    turnEvent(1, "turn-a", "turn_started", "root-a"),
    turnEvent(2, "turn-b", "turn_started", "root-b"),
  ]);
  const container = await mountBar("thread", false);
  assert.match(container.textContent, /Caddie: Working/);
  assert.doesNotMatch(container.textContent, /threads now/);
});
