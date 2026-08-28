const PREFIX = "buzz-unread-catch-up-discovery.v1";

function key(scope: string): string {
  return `${PREFIX}:${scope}`;
}

function read(scope: string): Record<string, number> {
  if (!scope) return {};
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(key(scope)) ?? "{}",
    );
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))
      return {};
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, number] =>
          Number.isSafeInteger(entry[1]) && entry[1] >= 0,
      ),
    );
  } catch {
    return {};
  }
}

export function readCatchUpDiscoveryAt(
  scope: string,
  channelId: string,
): number | null {
  return read(scope)[channelId] ?? null;
}

export function advanceCatchUpDiscoveryAt(
  scope: string,
  channelId: string,
  discoveryThrough: number | null | undefined,
): void {
  if (
    !scope ||
    typeof discoveryThrough !== "number" ||
    !Number.isSafeInteger(discoveryThrough) ||
    discoveryThrough < 0
  )
    return;
  const watermarks = read(scope);
  watermarks[channelId] = Math.max(
    watermarks[channelId] ?? 0,
    discoveryThrough,
  );
  try {
    window.localStorage.setItem(key(scope), JSON.stringify(watermarks));
  } catch {
    // Catch-up remains correct: the next launch repeats full discovery.
  }
}
