/**
 * The tag kinds a mock `send_channel_message` must echo back on the stored
 * event.
 *
 * Each of these rides its own independently validated argument of the real
 * Tauri command, and the relay returns them on the stored event. A kind
 * missing here is a kind every E2E row silently loses, so the renderer that
 * keys on it never gets exercised. Extracted from the bridge so that contract
 * is unit-testable.
 */
export function mockSendMessageExtraTags(args: {
  /** NIP-92 imeta attachments, read by the attachment renderers. */
  mediaTags?: string[][] | null;
  /** NIP-30 custom emoji, read when resolving `:shortcode:`. */
  emojiTags?: string[][] | null;
  /** Reference-only mentions, part of the outbound event's tag set. */
  mentionTags?: string[][] | null;
  /** Sender-authored link preview snapshots. */
  linkPreviewTags?: string[][] | null;
  /** Provenance of a message sent from a thread. */
  sentFromThreadTag?: string[] | null;
  /** Provenance of a forwarded copy, rendered as "Forwarded from …". */
  forwardedFromTag?: string[] | null;
  suppressLinkPreviews?: boolean;
}): string[][] {
  return [
    ...(args.mediaTags ?? []),
    ...(args.emojiTags ?? []),
    ...(args.mentionTags ?? []),
    ...(args.linkPreviewTags ?? []),
    ...(args.sentFromThreadTag ? [args.sentFromThreadTag] : []),
    ...(args.forwardedFromTag ? [args.forwardedFromTag] : []),
    ...(args.suppressLinkPreviews ? [["link-preview", "none"]] : []),
  ];
}
