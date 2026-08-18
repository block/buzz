const NOTIFICATION_BODY_MAX_LENGTH = 140;

/**
 * Resolve a channel's display label for use in notification titles.
 *
 * Returns `"#channelName"` when the channel is found and has a non-empty name,
 * or `null` when the channelId is absent or the channel is not yet in the list
 * (e.g. channels query hasn't resolved — the caller should fall back gracefully
 * rather than blocking the toast).
 */
export function resolveNotificationChannelLabel(
  channelId: string | null | undefined,
  channels: ReadonlyArray<{ id: string; name?: string | null }>,
): string | null {
  if (!channelId) return null;
  const channel = channels.find((c) => c.id === channelId);
  const name = channel?.name?.trim();
  return name ? `#${name}` : null;
}

/**
 * Truncate notification body text to {@link NOTIFICATION_BODY_MAX_LENGTH}
 * characters, appending "..." when truncated.  Returns `fallback` when
 * `content` is blank after trimming.
 */
export function truncateNotificationBody(
  content: string,
  fallback: string,
): string {
  const trimmed = content.trim();
  if (trimmed.length === 0) return fallback;
  if (trimmed.length <= NOTIFICATION_BODY_MAX_LENGTH) return trimmed;
  return `${trimmed.slice(0, NOTIFICATION_BODY_MAX_LENGTH - 3).trimEnd()}...`;
}

/**
 * Format a notification title with optional channel context.
 *
 * - With a channel label: `"prefix in #channel"`
 * - Without: `"prefix"`
 */
export function formatNotificationTitle(opts: {
  prefix: string;
  channelLabel: string | null;
}): string {
  return opts.channelLabel
    ? `${opts.prefix} in ${opts.channelLabel}`
    : opts.prefix;
}
