/**
 * Channel-wide mention marker tag (NIP-CM).
 *
 * A message carries at most one `["notify", "channel" | "here"]` tag. There is
 * no per-member `p`-tag expansion: the single marker is what the relay
 * validates and what clients key notification behavior off.
 *
 * Keep in sync with `buzz_core::channel_mentions` (Rust). Mode strings are
 * lowercase on the wire — the relay parse is case-sensitive.
 */
export const NOTIFY_TAG = "notify";

/** Wire values for the notify tag, in precedence order (broadest first). */
export const NOTIFY_MODES = ["channel", "here"] as const;

export type NotifyMode = (typeof NOTIFY_MODES)[number];

/** Narrow an arbitrary string to a notify mode. */
export function isNotifyMode(value: string): value is NotifyMode {
  return (NOTIFY_MODES as readonly string[]).includes(value);
}

/**
 * Read the notify mode carried by an already-split `["notify", mode]` tag set.
 *
 * Only the first tag is honored (one marker per event) and an unrecognized mode
 * reads as no mention, so a malformed tag degrades to a plain message instead
 * of being rejected by the relay.
 */
export function notifyModeFromTags(
  notifyTags: readonly string[][],
): NotifyMode | null {
  const mode = notifyTags[0]?.[1];
  return mode && isNotifyMode(mode) ? mode : null;
}
