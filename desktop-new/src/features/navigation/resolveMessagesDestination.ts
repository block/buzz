import type { MessagesDestination, NavigableChannel } from "./types";

/**
 * Resolves Messages' initial destination after its available conversations are
 * known. Explicit user work always wins; an unselected workspace opens the
 * first ordinary channel rather than a transitional empty panel.
 */
export function resolveMessagesDestination(
  current: MessagesDestination,
  channels: readonly NavigableChannel[],
  sessionChannelIds: ReadonlySet<string>,
): MessagesDestination {
  if (current.type !== "empty") return current;

  const firstChannel = channels.find(
    (channel) => !sessionChannelIds.has(channel.id),
  );
  return firstChannel
    ? { type: "channel", channelId: firstChannel.id }
    : current;
}
