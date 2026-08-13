import type { ObserverEvent } from "./ui/agentSessionTypes";
import { normalizePubkey } from "@/shared/lib/pubkey";

// Per-agent channel-activity summary: normalized pubkey → (channelId → latest
// activity ms). One number per distinct channel the agent has been seen active
// in, recorded from every live event as it arrives — so it is NOT subject to
// the UNPINNED_AGENT_EVENT_TAIL cap that bounds `eventsByAgent`. It exists to
// keep the profile activity feed's channel scope intact after an unpinned
// agent's event window is truncated to its newest tail: the tail retains the
// newest events (so the *preferred* channel is safe), but a channel whose last
// event fell out of the tail would vanish from the derived channel set without
// this durable summary. Compact by construction — bounded by the (small) count
// of distinct channels one agent works in, not by event volume.
const channelActivityByAgent = new Map<string, Map<string, number>>();

const EMPTY_CHANNEL_ACTIVITY: Record<string, number> = {};

/**
 * Record one live event into the per-agent channel-activity summary. Keeps the
 * latest activity ms per channel so the profile feed's channel scope survives
 * the tail-truncation of `eventsByAgent`. A channelId-less event (agent-general
 * frame) carries no channel scope and is skipped; a frame with an unparseable
 * timestamp cannot advance a channel's recency and is skipped. `key` is already
 * a normalized pubkey (the caller normalized it).
 */
export function recordChannelActivity(key: string, event: ObserverEvent): void {
  if (!event.channelId) {
    return;
  }
  const millis = Date.parse(event.timestamp);
  if (Number.isNaN(millis)) {
    return;
  }
  let byChannel = channelActivityByAgent.get(key);
  if (!byChannel) {
    byChannel = new Map();
    channelActivityByAgent.set(key, byChannel);
  }
  const previous = byChannel.get(event.channelId);
  if (previous === undefined || millis > previous) {
    byChannel.set(event.channelId, millis);
  }
}

/**
 * Latest observed activity ms per channel for one agent, from the durable
 * channel-activity summary. Unlike `getAgentObserverSnapshot().events`, this is
 * NOT truncated to the unpinned tail — it retains one entry per distinct
 * channel the agent has been active in for the store's lifetime. The profile
 * activity feed folds this into its channel scope so an unpinned agent's older
 * channels do not vanish from the switcher after its event window is trimmed.
 * Returns a shared empty object (stable identity) for an unknown agent.
 */
export function getAgentChannelActivity(
  agentPubkey?: string | null,
): Record<string, number> {
  if (!agentPubkey) {
    return EMPTY_CHANNEL_ACTIVITY;
  }
  const byChannel = channelActivityByAgent.get(normalizePubkey(agentPubkey));
  if (!byChannel || byChannel.size === 0) {
    return EMPTY_CHANNEL_ACTIVITY;
  }
  return Object.fromEntries(byChannel);
}

/** Clear the summary; called from resetAgentObserverStore. */
export function clearChannelActivity(): void {
  channelActivityByAgent.clear();
}
