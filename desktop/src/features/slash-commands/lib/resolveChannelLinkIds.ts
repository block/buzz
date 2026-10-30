type LinkableChannel = {
  id: string;
  name: string;
  channelType: string;
  archivedAt?: string | null;
};

/** Ids of the non-DM channels written as `#name` in `text`, in order, unique. */
export function resolveChannelLinkIds(
  text: string,
  channels: readonly LinkableChannel[],
): string[] {
  const byName = new Map<string, string>();
  for (const channel of channels) {
    if (channel.channelType === "dm") continue;
    const name = channel.name.toLowerCase();
    if (!byName.has(name)) byName.set(name, channel.id);
  }
  const ids: string[] = [];
  for (const match of text.matchAll(/(?:^|[\s([{])#([\w-]+)/g)) {
    const id = byName.get(match[1].toLowerCase());
    if (id && !ids.includes(id)) ids.push(id);
  }
  return ids;
}
