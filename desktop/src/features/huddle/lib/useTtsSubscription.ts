import { invoke } from "@tauri-apps/api/core";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";

const AGENT_PUBKEY_REFRESH_INTERVAL_MS = 30_000;

/**
 * Subscribe to agent TTS messages on the ephemeral huddle channel.
 * Pipes agent kind:9 messages to `speak_agent_message` on the Rust backend.
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

    async function loadAgentPubkeys() {
      try {
        const pubkeys = await invoke<string[]>("get_huddle_agent_pubkeys");
        agentPubkeys.clear();
        for (const pk of pubkeys) agentPubkeys.add(pk);
        agentsLoaded = true;
      } catch (e) {
        // Fail-closed on ALL failures, including refresh after prior success.
        // Clear the set and mark as not loaded — TTS goes mute until the
        // next successful refresh. Stale membership must never authorize speech.
        agentPubkeys.clear();
        agentsLoaded = false;
        console.error("[huddle] Failed to load agent pubkeys:", e);
      }
    }

    // Initial load + periodic refresh (catches mid-huddle agent additions).
    void loadAgentPubkeys();
    const agentRefreshId = window.setInterval(() => {
      void loadAgentPubkeys();
    }, AGENT_PUBKEY_REFRESH_INTERVAL_MS);

    // ── Live-only subscription ───────────────────────────────────────────
    // subscribeToChannelLive uses `since: now` — the relay never sends
    // historical backlog. Every event delivered is a live message.
    // Event-ID dedup handles reconnect replay (same event arriving twice).
    const seenEventIds = new Set<string>();
    const seenOrder: string[] = [];
    const MAX_SEEN_EVENTS = 5000;

    relayClient
      .subscribeToChannelLive(ephemeralChannelId, (event) => {
        if (disposed) return;
        // Defense-in-depth: subscription already filters to kind:9 only.
        if (event.kind !== 9) return;

        // Dedup by event ID (covers reconnect replay).
        if (seenEventIds.has(event.id)) return;
        seenEventIds.add(event.id);
        seenOrder.push(event.id);
        if (seenOrder.length > MAX_SEEN_EVENTS) {
          const oldest = seenOrder.shift();
          if (oldest !== undefined) seenEventIds.delete(oldest);
        }

        // Fail-closed: don't speak until agent list is loaded.
        if (!agentsLoaded) return;
        // Only speak agent messages — skip human STT transcripts.
        if (!agentPubkeys.has(event.pubkey)) return;
        if (event.pubkey === selfPubkeyRef.current) return;
        if (event.content.trim().length <= 1) return;
        // Legacy: skip [System]-prefixed messages from before kind:48106.
        if (event.content.startsWith("[System]")) return;
        invoke("speak_agent_message", { text: event.content }).catch((err) => {
          console.warn(
            "[huddle] TTS speak failed (backpressure or pipeline unavailable):",
            err,
          );
        });
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
      cleanup?.();
      window.clearInterval(agentRefreshId);
    };
  }, [ephemeralChannelId, selfPubkeyRef]);
}
