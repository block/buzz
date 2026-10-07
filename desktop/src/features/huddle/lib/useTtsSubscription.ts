import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

import {
  isDocumentVisible,
  subscribeDocumentVisibility,
} from "@/shared/lib/useDocumentVisible";
import { buildHuddleTtsLiveFilter } from "@/shared/api/relayChannelFilters";
import { relayClient } from "@/shared/api/relayClient";
import {
  createInitialTtsReadinessGate,
  createLatestStateGate,
  createOrderedSpeaker,
  routeLiveAgentText,
} from "./ttsLiveMessages";

const AGENT_PUBKEY_REFRESH_INTERVAL_MS = 30_000;
const AGENT_VERIFICATION_RETRY_INTERVAL_MS = 400;
const MAX_AGENT_VERIFICATION_ATTEMPTS = 8;
const TTS_STARTUP_REPLAY_WINDOW_SECONDS = 5;
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
    let agentVerificationRetryId: number | null = null;

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
      async (text, routeId, speakerPubkey) => {
        if (!disposed) {
          console.debug(
            `[huddle] tts stage=invoke status=attempted route_id=${routeId}`,
          );
          try {
            await invoke("speak_agent_message", {
              text,
              routeId,
              speakerPubkey,
            });
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

    type PendingDelivery = {
      event: Parameters<typeof routeLiveAgentText>[0];
      routeId: number;
    };
    const pendingAgentVerification = new Map<
      string,
      PendingDelivery & { attempts: number }
    >();

    const scheduleAgentVerificationRetry = () => {
      if (disposed || agentVerificationRetryId !== null) return;
      agentVerificationRetryId = window.setTimeout(async () => {
        agentVerificationRetryId = null;
        await loadAgentPubkeys();
        if (disposed) return;

        for (const [eventId, pending] of pendingAgentVerification) {
          if (agentPubkeys.has(pending.event.pubkey)) {
            pendingAgentVerification.delete(eventId);
            deliver(pending, false);
            continue;
          }
          pending.attempts += 1;
          if (pending.attempts >= MAX_AGENT_VERIFICATION_ATTEMPTS) {
            pendingAgentVerification.delete(eventId);
            console.debug(
              `[huddle] tts stage=eligibility status=rejected reason=author_not_agent route_id=${pending.routeId}`,
            );
          }
        }
        if (pendingAgentVerification.size > 0) {
          scheduleAgentVerificationRetry();
        }
      }, AGENT_VERIFICATION_RETRY_INTERVAL_MS);
    };

    const deliver = (
      { event, routeId }: PendingDelivery,
      allowAgentVerificationRetry = true,
    ) => {
      if (disposed) return;
      if (!agentsLoaded) {
        console.debug(
          `[huddle] tts stage=eligibility status=deferred reason=membership_unavailable route_id=${routeId}`,
        );
        if (allowAgentVerificationRetry) {
          pendingAgentVerification.set(event.id, {
            event,
            routeId,
            attempts: 0,
          });
          scheduleAgentVerificationRetry();
        }
        return;
      }
      const result = routeLiveAgentText(
        event,
        agentPubkeys,
        selfPubkeyRef.current,
        ephemeralChannelId,
        routeId,
        (text, queuedRouteId) =>
          speakInOrder.enqueue(text, queuedRouteId, event.pubkey),
      );
      if (result === "queued") {
        console.debug(
          `[huddle] tts stage=eligibility status=accepted route_id=${routeId}`,
        );
      } else {
        if (result === "author_not_agent" && allowAgentVerificationRetry) {
          pendingAgentVerification.set(event.id, {
            event,
            routeId,
            attempts: 0,
          });
          console.debug(
            `[huddle] tts stage=eligibility status=deferred reason=agent_verification_pending route_id=${routeId}`,
          );
          scheduleAgentVerificationRetry();
          return;
        }
        const reason =
          result === "disabled" && !ttsStateKnown
            ? "tts_state_unknown"
            : result;
        console.debug(
          `[huddle] tts stage=eligibility status=rejected reason=${reason} route_id=${routeId}`,
        );
      }
    };
    const initialReadinessGate = createInitialTtsReadinessGate(
      deliver,
      ({ routeId }, reason) => {
        console.debug(
          `[huddle] tts stage=eligibility status=rejected reason=${reason} route_id=${routeId}`,
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
          initialReadinessGate.markMembershipKnown();
        }
      } catch (e) {
        // Fail-closed on ALL failures, including refresh after prior success.
        // Clear the set and mark as not loaded — TTS goes mute until the
        // next successful refresh. Stale membership must never authorize speech.
        agentPubkeys.clear();
        agentsLoaded = false;
        if (initial) {
          initialReadinessGate.fail("membership_unavailable");
        }
        console.error("[huddle] Failed to load agent pubkeys:", e);
      }
    }

    // Initial load + periodic refresh (catches mid-huddle agent additions).
    // Keep the live subscription installed while hidden, but quiesce its REST
    // membership backstop and refresh immediately when the window returns.
    let agentRefreshId: number | null = null;
    const startAgentRefresh = (refreshNow: boolean) => {
      if (agentRefreshId !== null) window.clearInterval(agentRefreshId);
      agentRefreshId = null;
      if (!isDocumentVisible()) return;
      if (refreshNow) void loadAgentPubkeys();
      agentRefreshId = window.setInterval(() => {
        void loadAgentPubkeys();
      }, AGENT_PUBKEY_REFRESH_INTERVAL_MS);
    };
    void loadAgentPubkeys(true);
    startAgentRefresh(false);
    const unsubscribeDocumentVisibility = subscribeDocumentVisibility(
      (visible) => {
        if (visible) startAgentRefresh(true);
        else if (agentRefreshId !== null) {
          window.clearInterval(agentRefreshId);
          agentRefreshId = null;
        }
      },
    );

    // Install the state listener before requesting a snapshot. If a newer
    // event arrives while IPC is pending, it supersedes the stale snapshot.
    const ttsStateGate = createLatestStateGate<{ tts_enabled: boolean }>(
      (state) => {
        if (!disposed) {
          ttsStateKnown = true;
          speakInOrder.setEnabled(state.tts_enabled);
          initialReadinessGate.markTtsStateKnown();
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
            if (!ttsStateKnown)
              initialReadinessGate.fail("tts_state_unavailable");
            console.warn("[huddle] Failed to load TTS state:", err);
          });
      })
      .catch((err) => {
        speakInOrder.setEnabled(false);
        initialReadinessGate.fail("tts_state_unavailable");
        console.warn("[huddle] Failed to listen for TTS state:", err);
      });

    // ── Live subscription with bounded startup replay ────────────────────
    // The first agent reply can be stored while the Huddle provider is still
    // loading its membership and TTS snapshots. Include only the preceding
    // few seconds so that reply is recovered without replaying chat history.
    // Event-ID dedup handles the stored/live overlap and reconnect replay.
    const replaySince =
      Math.floor(Date.now() / 1000) - TTS_STARTUP_REPLAY_WINDOW_SECONDS;
    const seenEventIds = new Set<string>();
    const seenOrder: string[] = [];
    const MAX_SEEN_EVENTS = 5000;
    relayClient
      .subscribeLive(
        buildHuddleTtsLiveFilter(ephemeralChannelId, replaySince),
        (event) => {
          if (disposed) return;
          // Dedup by event ID if a relay repeats live fan-out.
          if (seenEventIds.has(event.id)) return;
          seenEventIds.add(event.id);
          seenOrder.push(event.id);
          if (seenOrder.length > MAX_SEEN_EVENTS) {
            const oldest = seenOrder.shift();
            if (oldest !== undefined) seenEventIds.delete(oldest);
          }

          // Preserve arrival order until initial membership and TTS state are
          // both known. A failed readiness check clears this buffer fail-closed.
          const routeId = allocateTtsRouteId();
          if (!agentsLoaded) {
            console.debug(
              `[huddle] tts stage=eligibility status=deferred reason=membership_unavailable route_id=${routeId}`,
            );
          }
          initialReadinessGate.push({ event, routeId });
        },
      )
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
      unsubscribeDocumentVisibility();
      if (agentRefreshId !== null) window.clearInterval(agentRefreshId);
      if (agentVerificationRetryId !== null) {
        window.clearTimeout(agentVerificationRetryId);
      }
      pendingAgentVerification.clear();
    };
  }, [ephemeralChannelId, selfPubkeyRef]);
}
