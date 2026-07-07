type MessageAuthorCandidate = {
  pubkey?: string | null;
};

/**
 * Max gap (seconds) between two same-author messages for the later one to still
 * render as a continuation (time-only, no avatar). Beyond this the message
 * reads as a new thought and gets the traditional avatar + header treatment,
 * even from the same author. Applied consistently across the channel timeline,
 * the threaded reply panel, and the home inbox detail view.
 */
export const MESSAGE_GROUPING_WINDOW_SECONDS = 10 * 60;

export function hasSameMessageAuthor(
  previous: MessageAuthorCandidate | null | undefined,
  current: MessageAuthorCandidate | null | undefined,
) {
  const previousPubkey = previous?.pubkey?.trim().toLowerCase();
  const currentPubkey = current?.pubkey?.trim().toLowerCase();

  return Boolean(
    previousPubkey && currentPubkey && previousPubkey === currentPubkey,
  );
}

/**
 * Whether `current` falls within {@link MESSAGE_GROUPING_WINDOW_SECONDS} of
 * `previous`. Both timestamps are Unix seconds. A missing previous timestamp
 * (or one in the future) is treated as out of window. Callers combine this
 * with {@link hasSameMessageAuthor} to decide continuation grouping.
 */
export function isWithinGroupingWindow(
  previousCreatedAt: number | null | undefined,
  currentCreatedAt: number | null | undefined,
) {
  if (
    typeof previousCreatedAt !== "number" ||
    typeof currentCreatedAt !== "number"
  ) {
    return false;
  }

  const gap = currentCreatedAt - previousCreatedAt;
  return gap >= 0 && gap <= MESSAGE_GROUPING_WINDOW_SECONDS;
}
