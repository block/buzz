import type { Channel } from "@/shared/api/types";

export function isHuddleBackingChannel(
  channel: Channel,
  huddleBackingChannelIds: ReadonlySet<string>,
): boolean {
  return huddleBackingChannelIds.has(channel.id);
}

export function shouldShowSidebarChannel(
  channel: Channel,
  huddleBackingChannelIds: ReadonlySet<string>,
  revealedHuddleChannelIds: ReadonlySet<string>,
): boolean {
  return (
    !isHuddleBackingChannel(channel, huddleBackingChannelIds) ||
    revealedHuddleChannelIds.has(channel.id)
  );
}
