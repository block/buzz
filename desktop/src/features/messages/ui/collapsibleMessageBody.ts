/** ~12 lines of text-sm at typical line-height — matches Slack-ish clamp. */
export const COLLAPSED_MESSAGE_MAX_HEIGHT_PX = 240;

/** Expand when the row is a search/route target so the match isn't hidden. */
export function shouldForceExpandMessageBody({
  highlighted,
  searchQuery,
}: {
  highlighted?: boolean;
  searchQuery?: string;
}): boolean {
  if (highlighted) return true;
  return Boolean(searchQuery?.trim());
}

export function messageBodyNeedsClamp(
  scrollHeight: number,
  maxHeight = COLLAPSED_MESSAGE_MAX_HEIGHT_PX,
): boolean {
  return scrollHeight > maxHeight + 1;
}
