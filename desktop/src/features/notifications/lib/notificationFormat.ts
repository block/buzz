import { i18n } from "@/i18n";

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
 * characters, appending the locale's truncation marker when truncated.  Returns
 * `fallback` when `content` is blank after trimming.
 */
export function truncateNotificationBody(
  content: string,
  fallback: string,
): string {
  const trimmed = content.trim();
  if (trimmed.length === 0) return fallback;
  if (trimmed.length <= NOTIFICATION_BODY_MAX_LENGTH) return trimmed;
  // Resolved per call: `i18n` boots after this module is imported, and the
  // marker's own length is the room the budget has to leave for it.
  const marker = i18n.t("notifications.toast.body-truncation-marker");
  return `${trimmed.slice(0, NOTIFICATION_BODY_MAX_LENGTH - marker.length).trimEnd()}${marker}`;
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
    ? i18n.t("notifications.toast.title-in-channel", {
        prefix: opts.prefix,
        channelLabel: opts.channelLabel,
      })
    : opts.prefix;
}

export type MessageNotificationSource =
  | "mention"
  | "approval"
  | "needs_action"
  | "dm"
  | "thread_reply";

/**
 * Neutral body copy for a blank message, per notification source. Resolved at
 * call time: this module is imported before the catalogs boot.
 */
function messageBodyFallback(source: MessageNotificationSource): string {
  if (source === "approval") {
    return i18n.t("notifications.toast.body-approval");
  }
  if (source === "dm") {
    return i18n.t("notifications.toast.body-new-message");
  }
  if (source === "thread_reply") {
    return i18n.t("notifications.toast.body-new-reply");
  }
  return i18n.t("notifications.toast.body-needs-attention");
}

/**
 * Title lead-in for a non-DM notification. The sender's display name leads
 * whenever their profile resolved; each source degrades to neutral copy
 * otherwise. Mentions and needs-action rows never invent a name.
 */
function messageTitlePrefix(
  source: MessageNotificationSource,
  senderName: string | null,
): string {
  if (source === "mention") {
    return senderName
      ? i18n.t("notifications.toast.title-mention-sender", { senderName })
      : i18n.t("notifications.toast.title-mention");
  }
  if (source === "approval") {
    return senderName
      ? i18n.t("notifications.toast.title-approval-sender", { senderName })
      : i18n.t("notifications.toast.title-approval");
  }
  if (source === "thread_reply") {
    return senderName
      ? i18n.t("notifications.toast.title-reply-sender", { senderName })
      : i18n.t("notifications.toast.title-reply");
  }
  return senderName ?? i18n.t("notifications.toast.title-needs-action");
}

/**
 * Canonical copy for every message-shaped desktop notification (home-feed
 * mentions and needs-action items, live DMs, live thread replies). All paths
 * format through here so sender attribution and fallbacks stay consistent:
 * the sender leads the title whenever their profile has resolved, and each
 * source degrades to neutral copy — never a raw pubkey — when it has not.
 *
 * `senderName` must already be a real human label (see
 * `senderNameFromSummary`); `channelName` is the raw channel name without
 * a `#` prefix.
 */
export function formatMessageNotification(opts: {
  source: MessageNotificationSource;
  senderName?: string | null;
  channelName?: string | null;
  content: string;
}): { title: string; body: string } {
  const { source, content } = opts;
  const senderName = opts.senderName?.trim() || null;
  const channelName = opts.channelName?.trim() || null;
  const body = truncateNotificationBody(content, messageBodyFallback(source));

  if (source === "dm") {
    return {
      title:
        senderName ??
        channelName ??
        i18n.t("notifications.toast.title-direct-message"),
      body,
    };
  }

  const channelLabel = channelName ? `#${channelName}` : null;
  const prefix = messageTitlePrefix(source, senderName);

  return { title: formatNotificationTitle({ prefix, channelLabel }), body };
}
