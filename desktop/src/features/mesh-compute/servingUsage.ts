import { i18n } from "@/i18n";
import type { MeshServingUsage } from "@/shared/api/tauriMesh";

/**
 * Pure projection of host-side serving usage into a small, politely-worded
 * indicator model for the Share compute card.
 *
 * Single source of truth for "who is using the compute I'm sharing" copy, so
 * the component and its tests agree. Kept pure/total (accepts null = not yet
 * fetched) and defensive (all fields optional-safe via the Rust extractor).
 *
 * Distinctions that matter:
 * - `localAttempts` = this machine's OWN agents using the local model. Not a
 *   "someone else is here" signal — surfaced softly as activity, not as a peer.
 * - `remoteAttempts` / `endpointAttempts` = another member consuming this
 *   machine's compute. THIS is the "someone connected to what I'm sharing"
 *   signal.
 */
export type MeshServingIndicator = {
  /** Whether to show anything at all (only while actively sharing). */
  show: boolean;
  /** Someone is being served right now. */
  active: boolean;
  /** A non-local member is (or has been) consuming this machine's compute. */
  hasRemoteConsumers: boolean;
  /** One-line status suitable for the card. */
  label: string;
  /** Longer detail for a tooltip / secondary line. */
  detail: string | null;
};

/**
 * @param usage  latest snapshot from `meshServingUsage`, or null if not fetched
 * @param isSharing  whether this machine is currently in serve mode (card owns
 *                   this from the toggle model). Usage is only meaningful while
 *                   sharing.
 */
export function deriveServingIndicator(
  usage: MeshServingUsage | null,
  isSharing: boolean,
): MeshServingIndicator {
  const hidden: MeshServingIndicator = {
    show: false,
    active: false,
    hasRemoteConsumers: false,
    label: "",
    detail: null,
  };
  if (!isSharing || !usage) {
    return hidden;
  }

  const hasRemoteConsumers =
    usage.remoteAttempts > 0 || usage.endpointAttempts > 0;
  const active = usage.inflight > 0;
  const tokensPerSecond = Math.round(usage.tokensPerSecond);

  // Remote consumer present (or seen) — the headline case the user asked for.
  if (hasRemoteConsumers) {
    const remote = usage.remoteAttempts + usage.endpointAttempts;
    const label = active
      ? i18n.t("mesh-compute.serving-indicator.remote-active", {
          inflight: usage.inflight,
        })
      : i18n.t("mesh-compute.serving-indicator.remote-idle", {
          count: remote,
        });
    const detail =
      usage.peers > 0
        ? i18n.t("mesh-compute.serving-indicator.peers-detail", {
            count: usage.peers,
            tokensPerSecond,
          })
        : i18n.t("mesh-compute.serving-indicator.tokens-per-second", {
            tokensPerSecond,
          });
    return { show: true, active, hasRemoteConsumers: true, label, detail };
  }

  // Only local (this machine's own agents) — show softly as activity.
  if (active) {
    return {
      show: true,
      active: true,
      hasRemoteConsumers: false,
      label: i18n.t("mesh-compute.serving-indicator.local-active", {
        inflight: usage.inflight,
      }),
      detail: i18n.t("mesh-compute.serving-indicator.tokens-per-second", {
        tokensPerSecond,
      }),
    };
  }
  if (usage.requestsServed > 0) {
    return {
      show: true,
      active: false,
      hasRemoteConsumers: false,
      label: i18n.t("mesh-compute.serving-indicator.idle-right-now"),
      detail: i18n.t("mesh-compute.serving-indicator.served-this-session", {
        count: usage.requestsServed,
      }),
    };
  }

  // Sharing but nothing served yet.
  return {
    show: true,
    active: false,
    hasRemoteConsumers: false,
    label: i18n.t("mesh-compute.serving-indicator.idle-yet"),
    detail: null,
  };
}
