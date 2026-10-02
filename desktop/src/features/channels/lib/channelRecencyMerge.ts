export type RecencyChannel = {
  id: string;
  lastMessageAt: string | null;
};

function timestamp(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * Keeps recency monotonic when a refresh settles. An authoritative absence may
 * clear an unchanged value, but never a live value added during the request.
 */
export function mergeConcurrentChannelRecency<T extends RecencyChannel>(
  refreshed: T[],
  displayed: T[] | undefined,
  atRequestStart: T[] | undefined,
): T[] {
  if (!displayed) return refreshed;
  const displayedById = new Map(displayed.map((c) => [c.id, c.lastMessageAt]));
  const startById = new Map(
    atRequestStart?.map((c) => [c.id, c.lastMessageAt]) ?? [],
  );

  return refreshed.map((channel) => {
    const displayedValue = displayedById.get(channel.id);
    const displayedAt = timestamp(displayedValue);
    const refreshedAt = timestamp(channel.lastMessageAt);
    const changed = displayedValue !== startById.get(channel.id);
    const keepDisplayed =
      displayedAt !== null &&
      (refreshedAt !== null ? displayedAt > refreshedAt : changed);
    return keepDisplayed
      ? { ...channel, lastMessageAt: displayedValue ?? null }
      : channel;
  });
}
