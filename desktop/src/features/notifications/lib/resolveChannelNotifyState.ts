import type { ChannelMuteStore } from "@/features/sidebar/lib/channelMutesStorage";
import type {
  ChannelNotifyLevel,
  ChannelNotifyPrefsStore,
} from "@/features/sidebar/lib/channelNotifyPrefsStorage";

/**
 * The single resolved notification state for a channel. Every consumer (unread
 * aggregation, the notify ladder, delivery sites, the sidebar, the
 * cross-community rail observer) reads this instead of re-deriving level or
 * expiry logic. Pure and React-free so the non-React observer can use it.
 */
export type ResolvedChannelNotifyState = {
  level: ChannelNotifyLevel;
  /** True when `level` is "mute" only because a timed mute is still running. */
  timedMuteActive: boolean;
  /** Expiry of the running timed mute (Unix seconds), or null when none runs. */
  muteUntil: number | null;
  desktop: boolean;
  followAllThreads: boolean;
  broadcasts: boolean;
  /** Hide the channel from sidebar lists ("Mute and hide"). */
  hidden: boolean;
};

export const DEFAULT_CHANNEL_NOTIFY_STATE: ResolvedChannelNotifyState =
  Object.freeze({
    level: "all" as ChannelNotifyLevel,
    timedMuteActive: false,
    muteUntil: null,
    desktop: true,
    followAllThreads: false,
    broadcasts: true,
    hidden: false,
  });

/**
 * Resolve a channel's effective notification state from the prefs blob and the
 * legacy `channel-mutes` blob.
 *
 * Interop rule (NIP-CN): when both stores have an entry for the channel, the
 * newer `updatedAt` wins **for the mute dimension only** — an unmute performed
 * on an old client (or mobile) must beat a stale prefs "mute", and prefs wins
 * ties. A legacy-only mute resolves to level "mute" without `hidden`, so
 * channels muted under the old UI never disappear unexpectedly.
 *
 * A running `muteUntil` is a lazy overlay: it forces level "mute" without
 * touching the stored level, so expiry restores the prior level automatically
 * and never hides the channel.
 */
export function resolveChannelNotifyState(
  channelId: string,
  prefs: ChannelNotifyPrefsStore,
  legacyMutes: ChannelMuteStore,
  nowSeconds: number,
): ResolvedChannelNotifyState {
  const entry = prefs.channels[channelId];
  const legacy = legacyMutes.channels[channelId];
  if (!entry && !legacy) return DEFAULT_CHANNEL_NOTIFY_STATE;

  const storedLevel: ChannelNotifyLevel = entry?.level ?? "all";
  let level = storedLevel;
  let hidden = Boolean(entry) && storedLevel === "mute";

  if (legacy) {
    const legacyWins = !entry || legacy.updatedAt > entry.updatedAt;
    if (legacyWins) {
      if (legacy.muted) {
        level = "mute";
        hidden = false;
      } else if (storedLevel === "mute") {
        // Old clients can only express muted/unmuted; a newer unmute clears the
        // mute dimension and leaves the channel at the default level.
        level = "all";
        hidden = false;
      }
    }
  }

  const timedMuteActive =
    entry?.muteUntil !== undefined && entry.muteUntil > nowSeconds;
  if (timedMuteActive) level = "mute";

  return {
    level,
    timedMuteActive,
    muteUntil: timedMuteActive ? (entry?.muteUntil ?? null) : null,
    desktop: entry?.desktop ?? true,
    followAllThreads: entry?.followAllThreads ?? false,
    broadcasts: entry?.broadcasts ?? true,
    hidden,
  };
}

/**
 * Earliest still-running `muteUntil` in the store, or null when no timed mute is
 * active. Drives the single UI-refresh timer that re-resolves state on expiry.
 */
export function nextTimedMuteExpiry(
  prefs: ChannelNotifyPrefsStore,
  nowSeconds: number,
): number | null {
  let earliest: number | null = null;
  for (const entry of Object.values(prefs.channels)) {
    const until = entry.muteUntil;
    if (until === undefined || until <= nowSeconds) continue;
    if (earliest === null || until < earliest) earliest = until;
  }
  return earliest;
}
