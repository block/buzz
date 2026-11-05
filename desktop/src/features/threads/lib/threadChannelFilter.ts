export type ThreadChannelOption = {
  channelId: string;
  count: number;
  label: string;
};

/**
 * One option per channel that has a named thread, alphabetical by label so the
 * menu doesn't reshuffle as threads get new activity.
 */
export function threadChannelOptions(
  entries: readonly { channelId: string }[],
  labelFor: (channelId: string) => string,
): ThreadChannelOption[] {
  const counts = new Map<string, number>();
  for (const entry of entries) {
    counts.set(entry.channelId, (counts.get(entry.channelId) ?? 0) + 1);
  }
  return [...counts]
    .map(([channelId, count]) => ({
      channelId,
      count,
      label: labelFor(channelId),
    }))
    .sort(
      (a, b) =>
        a.label.localeCompare(b.label) ||
        a.channelId.localeCompare(b.channelId),
    );
}

/**
 * A channel filter only applies while that channel still has a named thread;
 * otherwise the view would sit empty behind a filter the menu no longer lists.
 */
export function effectiveChannelFilter(
  selected: string | null,
  options: readonly ThreadChannelOption[],
): string | null {
  if (selected === null) return null;
  return options.some((option) => option.channelId === selected)
    ? selected
    : null;
}

export function filterByChannel<T extends { channelId: string }>(
  entries: readonly T[],
  channelId: string | null,
): T[] {
  return channelId === null
    ? [...entries]
    : entries.filter((entry) => entry.channelId === channelId);
}
