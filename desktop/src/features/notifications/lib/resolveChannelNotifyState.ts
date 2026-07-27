import type {
  ChannelMuteEntry,
  ChannelMuteStore,
} from "@/features/sidebar/lib/channelMutesStorage";
import type {
  ChannelNotifyEntry,
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
 * The NIP-CN legacy interop rule (N2) for the **mute dimension only**: a
 * `channel-mutes` write that is newer than the prefs entry owns the channel's
 * durable mute state, so an unmute performed on an old client (or mobile) beats
 * a stale prefs "mute" and a newer legacy mute beats a stale prefs level. Prefs
 * wins ties.
 *
 * Exported because writers need it too: a mutation that reseeds an entry has to
 * fold this decision in before stamping a fresh `updatedAt`, otherwise the new
 * timestamp flips the tie-break and silently resurrects the level the legacy
 * blob had already overruled.
 */
export function foldLegacyMuteDecision(
  entry: ChannelNotifyEntry | undefined,
  legacy: ChannelMuteEntry | undefined,
): ChannelNotifyLevel {
  const stored = entry?.level ?? "all";
  if (!legacy) return stored;
  if (entry && legacy.updatedAt <= entry.updatedAt) return stored;
  if (legacy.muted) return "mute";
  // Old clients can only express muted/unmuted; a newer unmute clears the mute
  // dimension and leaves the channel at the default level.
  return stored === "mute" ? "all" : stored;
}

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
 * `hidden` needs both terms: the prefs entry must explicitly say "mute" AND the
 * interop decision must still resolve to "mute". A newer legacy unmute clears
 * hiding along with the mute; a newer legacy *mute* leaves hiding in place
 * instead of silently downgrading "Mute and hide" to plain mute.
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
  const durableLevel = foldLegacyMuteDecision(entry, legacy);
  // Derived from the durable level, before the timed-mute overlay: a timed mute
  // must never hide, and it must not resurrect hiding a legacy unmute cleared.
  const hidden =
    Boolean(entry) && storedLevel === "mute" && durableLevel === "mute";

  let level = durableLevel;
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
