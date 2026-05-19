import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Heartbeat interval (ms) for the mesh-LLM offer publisher.
 *
 * The Rust side stamps `expires_at = now + OFFER_TTL_SECS` (15 min) on
 * each publish. We re-publish at ~1/3 of that so even a single missed
 * heartbeat leaves the offer visible to consumers; only a crash that
 * misses two consecutive heartbeats actually drops us out of the UI.
 *
 * Kept here rather than computed from a Rust constant because the value
 * is fundamentally a *frontend timer* (Tauri-side has no concept of "the
 * UI mounted"); we just need it to stay strictly less than the
 * Rust-side TTL so the invariant holds.
 */
const HEARTBEAT_MS = 5 * 60 * 1000;

/**
 * While `enabled` is true, periodically re-invoke `mesh_publish_offer` so
 * the kind:31990 event's `expires_at` stays fresh. Republishes are NIP-33
 * replaces under the same `(pubkey, d_tag)` address — consumers do not see
 * a flicker; they just see the deadline advance.
 *
 * Errors are logged to the console and dropped on the floor — the user
 * is not in front of the settings panel waiting for them, and the next
 * heartbeat (or an explicit prefs change) will surface a fresh error if
 * the relay is durably down.
 */
export function useMeshOfferHeartbeat(params: {
  enabled: boolean;
  irohRelayUrl: string | null;
}): void {
  const { enabled, irohRelayUrl } = params;

  useEffect(() => {
    if (!enabled || irohRelayUrl == null || irohRelayUrl === "") {
      return;
    }
    let cancelled = false;
    const tick = async () => {
      if (cancelled) return;
      try {
        await invoke("mesh_publish_offer", { irohRelayUrl });
      } catch (e) {
        // Heartbeat failure is non-fatal — log and let the next tick try.
        console.warn("mesh-llm heartbeat failed:", e);
      }
    };
    const id = setInterval(tick, HEARTBEAT_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [enabled, irohRelayUrl]);
}
