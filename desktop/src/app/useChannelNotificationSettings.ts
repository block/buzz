import * as React from "react";

import {
  DEFAULT_CHANNEL_NOTIFY_STATE,
  type ResolvedChannelNotifyState,
} from "@/features/notifications/lib/resolveChannelNotifyState";
import type { ChannelNotifyLevel } from "@/features/sidebar/lib/channelNotifyPrefsStorage";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import {
  useChannelNotifyPrefs,
  type ChannelNotifyAdvancedPatch,
} from "@/features/sidebar/lib/useChannelNotifyPrefs";
import { useStableSet } from "@/shared/hooks/useStableReference";

export type ChannelNotificationSettings = {
  /**
   * Effective boolean mute: the legacy `channel-mutes` blob unioned with the
   * channels the resolver puts at level "mute" (including a running timed
   * mute). Consumers that still speak in booleans read this.
   */
  mutedChannelIds: ReadonlySet<string>;
  resolveChannelNotify: (channelId: string) => ResolvedChannelNotifyState;
  setChannelNotifyLevel: (channelId: string, level: ChannelNotifyLevel) => void;
  muteChannelUntil: (channelId: string, untilSeconds: number) => void;
  clearChannelTimedMute: (channelId: string) => void;
  setChannelNotifyAdvanced: (
    channelId: string,
    patch: ChannelNotifyAdvancedPatch,
  ) => void;
  /** Legacy blob mutations, still wired to the old binary Mute/Unmute items. */
  muteChannel: (channelId: string) => void;
  unmuteChannel: (channelId: string) => void;
};

/**
 * Composes the two synced notification blobs into the single surface AppShell
 * threads through the app: the NIP-CN per-channel preferences (kind 30078,
 * d-tag `channel-notify-prefs`) resolved against the legacy `channel-mutes`
 * blob, plus the NIP-CN dual-write.
 */
export const DEFAULT_CHANNEL_NOTIFICATION_SETTINGS: ChannelNotificationSettings =
  Object.freeze({
    mutedChannelIds: new Set<string>(),
    resolveChannelNotify: () => DEFAULT_CHANNEL_NOTIFY_STATE,
    setChannelNotifyLevel: () => {},
    muteChannelUntil: () => {},
    clearChannelTimedMute: () => {},
    setChannelNotifyAdvanced: () => {},
    muteChannel: () => {},
    unmuteChannel: () => {},
  });

export function useChannelNotificationSettings(
  pubkey: string | undefined,
  relayUrl: string | undefined,
): ChannelNotificationSettings {
  const {
    mutedChannelIds: legacyMutedChannelIds,
    muteStore: legacyMuteStore,
    muteChannel,
    unmuteChannel,
  } = useChannelMutes(pubkey);
  const { prefsStore, resolveChannel, setChannelLevel, ...prefs } =
    useChannelNotifyPrefs(pubkey, relayUrl, legacyMuteStore);

  // Dual-write (NIP-CN N2): a level change also moves the legacy mute boolean
  // so old clients and mobile keep honoring the mute. Timed mutes are
  // deliberately excluded — old clients cannot express them.
  const setChannelNotifyLevel = React.useCallback(
    (channelId: string, level: ChannelNotifyLevel) => {
      setChannelLevel(channelId, level);
      if (level === "mute") {
        muteChannel(channelId);
      } else {
        unmuteChannel(channelId);
      }
    },
    [muteChannel, setChannelLevel, unmuteChannel],
  );

  const mutedChannelIds = useStableSet(
    React.useMemo(() => {
      const ids = new Set<string>();
      for (const channelId of [
        ...legacyMutedChannelIds,
        ...Object.keys(prefsStore.channels),
      ]) {
        if (resolveChannel(channelId).level === "mute") ids.add(channelId);
      }
      return ids;
    }, [legacyMutedChannelIds, prefsStore.channels, resolveChannel]),
  );

  return {
    mutedChannelIds,
    resolveChannelNotify: resolveChannel,
    setChannelNotifyLevel,
    muteChannelUntil: prefs.muteChannelUntil,
    clearChannelTimedMute: prefs.clearTimedMute,
    setChannelNotifyAdvanced: prefs.setChannelAdvanced,
    muteChannel,
    unmuteChannel,
  };
}
