/**
 * Mounted-hook lifecycle test for the observer-agent viewer pin.
 *
 * The tiered live-event window (observerRelayStore) keeps an agent's full
 * MAX_OBSERVER_EVENTS window only while a session viewer is pinned; an unpinned
 * agent is bounded to UNPINNED_AGENT_EVENT_TAIL. The pin/unpin lifecycle is
 * owned by the viewer hooks: useObserverEvents (session panel) and
 * useAgentTranscript (activity bar). This mounts the REAL hooks in a React tree
 * and asserts the wiring end to end:
 *
 *   - mounting a viewer pins the agent, so its window accumulates the full
 *     window (well past the unpinned tail);
 *   - unmounting the viewer unpins and immediately bounds the window back to
 *     the tail — reclaiming the deep scroll-back the panel accumulated.
 *
 * This is a mutation test for the useObserverAgentPin wiring: remove the pin
 * from the hook and the mounted-window assertion fails (window would be the
 * tail); remove the unpin/truncate and the post-unmount assertion fails
 * (window would stay full).
 *
 * The hooks are mounted with `enabled = false`: the pin is keyed on pubkey and
 * is independent of `enabled` (which gates only the live relay subscription),
 * so the pin still fires while the relay/Tauri subscription side effect stays
 * dormant — no IPC mock needed.
 *
 * DOM shim: react-dom/client needs a minimal DOM; installDOMShim (from the
 * shared observed-unread harness) provides it, matching the convention used by
 * the other mounted-hook suites in this package.
 */

import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { installDOMShim } from "@/features/channels/observedUnreadTestHarness.mjs";

installDOMShim();

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";

import {
  getAgentObserverSnapshot,
  injectObserverEventsForE2E,
  resetAgentObserverStore,
} from "@/features/agents/observerRelayStore.ts";
import {
  useAgentTranscript,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents.ts";

const TAIL = 100; // UNPINNED_AGENT_EVENT_TAIL (private constant, mirrored here)
const OVER_TAIL = TAIL + 50; // forces truncation when unpinned; far under 3000

const AGENT = "a".repeat(64);

function makeEvent(seq) {
  return {
    seq,
    timestamp: new Date(1_000_000 + seq * 1000).toISOString(),
    kind: "acp_write",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: {},
  };
}

// Injection notifies store listeners, which re-renders the mounted viewer's
// useSyncExternalStore. Wrap in act() so those commits are flushed inside the
// React act scope rather than warning about unbatched updates.
async function injectSequential(agent, count) {
  await act(async () => {
    for (let seq = 1; seq <= count; seq++) {
      injectObserverEventsForE2E(agent, [makeEvent(seq)]);
    }
  });
}

function windowLength(agent) {
  return getAgentObserverSnapshot(agent, true).events.length;
}

/**
 * Mount a viewer hook (enabled = false so only the pin fires) and return an
 * unmount fn. `useHook` is the viewer hook under test.
 */
async function mountViewer(useHook) {
  function Harness() {
    useHook(false, AGENT);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(React.createElement(Harness));
  });
  return async () => {
    await act(async () => {
      root.unmount();
    });
  };
}

describe("useObserverAgentPin — mounted viewer lifecycle", () => {
  beforeEach(() => {
    resetAgentObserverStore();
  });

  it("test_mounting_useObserverEvents_pins_agent_then_unmount_truncates", async () => {
    const unmount = await mountViewer(useObserverEvents);

    await injectSequential(AGENT, OVER_TAIL);
    assert.equal(
      windowLength(AGENT),
      OVER_TAIL,
      "a mounted session-panel viewer pins the agent — the full window accumulates",
    );

    await unmount();
    assert.equal(
      windowLength(AGENT),
      TAIL,
      "unmounting the viewer unpins and bounds the window back to the tail",
    );
  });

  it("test_mounting_useAgentTranscript_pins_agent_then_unmount_truncates", async () => {
    // useAgentTranscript is the activity-bar entry point (BotActivityBar), so
    // it must pin too — otherwise a visible activity bar would show a
    // tail-truncated transcript.
    const unmount = await mountViewer(useAgentTranscript);

    await injectSequential(AGENT, OVER_TAIL);
    assert.equal(
      windowLength(AGENT),
      OVER_TAIL,
      "a mounted activity-bar viewer pins the agent — the full window accumulates",
    );

    await unmount();
    assert.equal(
      windowLength(AGENT),
      TAIL,
      "unmounting the activity-bar viewer bounds the window back to the tail",
    );
  });
});
