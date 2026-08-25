export type BusinessNavigationSource = "explicit" | "automatic";

export function canNavigateBusinessResource({
  followConversation,
  pinned,
  source,
}: {
  followConversation: boolean;
  pinned: boolean;
  source: BusinessNavigationSource;
}): boolean {
  if (source === "explicit") return true;
  return followConversation && !pinned;
}

export function shouldQueuePendingBusinessNavigation(
  bridgeVersion: 1 | 2 | null,
): boolean {
  return bridgeVersion === null;
}

export function keepLatestBusinessNavigation<T>(
  _current: T | null,
  next: T,
): T {
  return next;
}
