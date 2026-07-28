import { channelNotifyHeaderSuffix } from "@/features/notifications/lib/channelNotifyLabels";
import type { ResolvedChannelNotifyState } from "@/features/notifications/lib/resolveChannelNotifyState";
import type { Channel } from "@/shared/api/types";

/**
 * Header description line for a channel. When `notify` is supplied and the
 * channel's NIP-CN level is not the default, the level is appended so the
 * header explains why the channel is quiet.
 */
export function getChannelDescription(
  channel: Channel | null,
  notify?: ResolvedChannelNotifyState | null,
): string {
  if (!channel) {
    return "Connect to the relay to browse channels and read messages.";
  }

  const notifySuffix = notify ? channelNotifyHeaderSuffix(notify) : null;

  const prefixes = [
    channel.archivedAt ? "Archived." : null,
    !channel.isMember ? "Read-only until you join this open channel." : null,
  ].filter((value) => value && value.trim().length > 0);

  // Show only the first non-empty field to avoid duplication when
  // topic, description, and purpose contain overlapping text.
  const detail = [channel.topic, channel.description, channel.purpose].find(
    (value) => value && value.trim().length > 0,
  );

  const parts = [...prefixes, detail ?? null].filter(Boolean);
  const body =
    parts.length > 0 ? parts.join(" ") : "Channel details and activity.";

  return notifySuffix ? `${body} ${notifySuffix}` : body;
}
