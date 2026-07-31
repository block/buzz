import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

import { buildHuddleTtsLiveFilter } from "@/shared/api/relayChannelFilters";
import { relayClient } from "@/shared/api/relayClient";
import {
  createInitialMembershipGate,
  createLatestStateGate,
  createOrderedSpeaker,
  routeLiveAgentText,
} from "./ttsLiveMessages";

const AGENT_PUBKEY_REFRESH_INTERVAL_MS = 30_000;
let nextTtsRouteId = 1;

function allocateTtsRouteId(): number {
  const routeId = nextTtsRouteId;
  nextTtsRouteId += 1;
  return routeId;
}

/**
 * Subscribe to agent TTS messages on the ephemeral huddle channel.
 * Pipes new agent message events to `speak_agent_message` on the Rust backend.
 *
 * Extracted from HuddleContext to keep file sizes manageable.
 */
export function useTtsSubscription(
  ephemeralChannelId: string | null,
  selfPubkeyRef: React.RefObject<string | null>,
) {
  React.useEffect(() => {
    if (!ephemeralChannelId) return;

    let disposed = false;
    let cleanup: (() => void) | null = null;
    let unlistenHuddleState: (() => void) | null = null;
    let ttsStateKnown = false;

    // ── Agent identity (authoritative, fail-closed) ───────────────────────
    //
    // Fetch the ephemeral channel's member list from the relay REST API and
    // identify agents by their "bot" role. This is authoritative — it works
    // for both creators and joiners, and reflects mid-huddle agent additions.
    //
    // FAIL-CLOSED: agentsLoaded starts false. Until the fetch succeeds and
    // populates agentPubkeys, NO messages are spoken. An empty set after a
    // successful fetch means "no agents in the huddle" → still mute.
    let agentsLoaded = false;
    const agentPubkeys = new Set<string>();

    const speakInOrder = createOrderedSpeaker(
      async (text, routeId) => {
        if (!disposed) {
          console.debug(
            `[huddle] tts stage=invoke status=attempted route_id=${routeId}`,
          );
          try {
            await invoke("speak_agent_message", { text, routeId });
            console.debug(
              `[huddle] tts stage=invoke status=accepted route_id=${routeId}`,
            );
          } catch (error) {
            console.warn(
              `[huddle] tts stage=invoke status=failed reason=native_error route_id=${routeId}`,
            );
            throw error;
          }
        }
      },
      () => {},
      false,
      (routeId, reason) => {
        console.debug(
          `[huddle] tts stage=queue status=dropped reason=${reason} route_id=${routeId}`,
        );
      },
    );

    const deliver = ({
      event,
      routeId,
    }: {
      event: Parameters<typeof routeLiveAgentText>[0];
      routeId: number;
    }) => {
      if (disposed) return;
      if (!agentsLoaded) {
        console.debug(
          `[huddle] tts stage=eligibility status=rejected reason=membership_unavailable route_id=${routeId}`,
        );
        return;
      }
      const result = routeLiveAgentText(
        event,
        agentPubkeys,
        selfPubkeyRef.current,
        ephemeralChannelId,
        routeId,
        speakInOrder.enqueue,
      );
      if (result === "queued") {
        console.debug(
          `[huddle] tts stage=eligibility status=accepted route_id=${routeId}`,
        );
      } else {
        const reason =
          result === "disabled" && !ttsStateKnown
            ? "tts_state_unknown"
            : result;
        console.debug(
          `[huddle] tts stage=eligibility status=rejected reason=${reason} route_id=${routeId}`,
        );
      }
    };
    const initialMembershipGate = createInitialMembershipGate(
      deliver,
      ({ routeId }) => {
        console.debug(
          `[huddle] tts stage=eligibility status=rejected reason=membership_unavailable route_id=${routeId}`,
        );
      },
    );

    async function loadAgentPubkeys(initial = false) {
      try {
        const pubkeys = await invoke<string[]>("get_huddle_agent_pubkeys");
        if (disposed) return;
        agentPubkeys.clear();
        for (const pk of pubkeys) agentPubkeys.add(pk);
        agentsLoaded = true;
        if (initial) {
          initialMembershipGate.succeed();
        }
      } catch (e) {
        // Fail-closed on ALL failures, including refresh after prior success.
        // Clear the set and mark as not loaded — TTS goes mute until the
        // next successful refresh. Stale membership must never authorize speech.
        agentPubkeys.clear();
        agentsLoaded = false;
        if (initial) {
          initialMembershipGate.fail();
        }
        console.error("[huddle] Failed to load agent pubkeys:", e);
      }
    }

    // Initial load + periodic refresh (catches mid-huddle agent additions).
    void loadAgentPubkeys(true);
    const agentRefreshId = window.setInterval(() => {
      void loadAgentPubkeys();
    }, AGENT_PUBKEY_REFRESH_INTERVAL_MS);

    // Install the state listener before requesting a snapshot. If a newer
    // event arrives while IPC is pending, it supersedes the stale snapshot.
    const ttsStateGate = createLatestStateGate<{ tts_enabled: boolean }>(
      (state) => {
        if (!disposed) {
          ttsStateKnown = true;
          speakInOrder.setEnabled(state.tts_enabled);
        }
      },
    );
    void listen<{ tts_enabled: boolean }>("huddle-state-changed", (event) => {
      if (!disposed) ttsStateGate.applyEvent(event.payload);
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        unlistenHuddleState = unlisten;
        const applyBootstrap = ttsStateGate.beginSnapshot();
        void invoke<{ tts_enabled: boolean }>("get_huddle_state")
          .then((state) => {
            if (!disposed) applyBootstrap(state);
          })
          .catch((err) => {
            console.warn("[huddle] Failed to load TTS state:", err);
          });
      })
      .catch((err) => {
        speakInOrder.setEnabled(false);
        console.warn("[huddle] Failed to listen for TTS state:", err);
      });

    // ── Live-only subscription ───────────────────────────────────────────
    // A limit:0 subscription receives future message fan-out while the relay
    // returns no stored rows, including pre-join rows from the current second.
    // Event-ID dedup handles reconnect replay (same event arriving twice).
    const seenEventIds = new Set<string>();
    const seenOrder: string[] = [];
    const MAX_SEEN_EVENTS = 5000;
    relayClient
      .subscribeLive(buildHuddleTtsLiveFilter(ephemeralChannelId), (event) => {
        if (disposed) return;
        // Dedup by event ID if a relay repeats live fan-out.
        if (seenEventIds.has(event.id)) return;
        seenEventIds.add(event.id);
        seenOrder.push(event.id);
        if (seenOrder.length > MAX_SEEN_EVENTS) {
          const oldest = seenOrder.shift();
          if (oldest !== undefined) seenEventIds.delete(oldest);
        }

        // Preserve arrival order while the initial authoritative membership
        // lookup is pending. A failed lookup clears this buffer fail-closed.
        const routeId = allocateTtsRouteId();
        if (!agentsLoaded) {
          console.debug(
            `[huddle] tts stage=eligibility status=deferred reason=membership_unavailable route_id=${routeId}`,
          );
        }
        initialMembershipGate.push({ event, routeId });
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((err) => {
        console.error("[huddle] TTS subscription failed:", err);
      });

    return () => {
      disposed = true;
      speakInOrder.setEnabled(false);
      cleanup?.();
      unlistenHuddleState?.();
      window.clearInterval(agentRefreshId);
    };
  }, [ephemeralChannelId, selfPubkeyRef]);
}
