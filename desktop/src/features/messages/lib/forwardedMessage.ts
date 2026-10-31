import { getSendToChannelSemantics } from "@/features/messages/lib/sendToChannelSemantics";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Provenance marker a forwarded copy carries so the destination can attribute
 * it: `["buzz:forwarded-from", <source event id>, <source channel id>,
 * <original author pubkey>, <thread root id>?]`.
 *
 * The Tauri `send_channel_message` command re-validates every part before
 * signing (see `events/message_tags.rs`); this module is the renderer-side
 * half of that contract.
 */
export const FORWARDED_FROM_TAG = "buzz:forwarded-from";

const HEX_64_PATTERN = /^[0-9a-f]{64}$/;
const CHANNEL_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export type ForwardedFromReference = {
  sourceEventId: string;
  sourceChannelId: string;
  /** Author of the message that was forwarded, not of the forwarded copy. */
  authorPubkey: string;
  /** Thread root of the source message; null when it was top-level. */
  threadRootId: string | null;
};

function normalizedId(value: string | undefined | null): string {
  return (value ?? "").trim().toLowerCase();
}

function isValidReference(reference: ForwardedFromReference): boolean {
  return (
    HEX_64_PATTERN.test(reference.sourceEventId) &&
    CHANNEL_ID_PATTERN.test(reference.sourceChannelId) &&
    HEX_64_PATTERN.test(reference.authorPubkey) &&
    (reference.threadRootId === null ||
      HEX_64_PATTERN.test(reference.threadRootId))
  );
}

/** Build the signed-event tag for a forward. Throws on an unusable reference. */
export function buildForwardedFromTag(
  reference: ForwardedFromReference,
): string[] {
  const normalized: ForwardedFromReference = {
    sourceEventId: normalizedId(reference.sourceEventId),
    sourceChannelId: normalizedId(reference.sourceChannelId),
    authorPubkey: normalizePubkey(reference.authorPubkey ?? ""),
    threadRootId: reference.threadRootId
      ? normalizedId(reference.threadRootId)
      : null,
  };
  if (!isValidReference(normalized)) {
    throw new Error("A forwarded message needs a complete source reference.");
  }

  const tag = [
    FORWARDED_FROM_TAG,
    normalized.sourceEventId,
    normalized.sourceChannelId,
    normalized.authorPubkey,
  ];
  return normalized.threadRootId ? [...tag, normalized.threadRootId] : tag;
}

/** Read the provenance of a received message; null when it is not a forward. */
export function getForwardedFromReference(
  tags: readonly (readonly string[])[] | null | undefined,
): ForwardedFromReference | null {
  const tag = tags?.find(
    (candidate) =>
      (candidate.length === 4 || candidate.length === 5) &&
      candidate[0] === FORWARDED_FROM_TAG,
  );
  if (!tag) return null;

  const reference: ForwardedFromReference = {
    sourceEventId: normalizedId(tag[1]),
    sourceChannelId: normalizedId(tag[2]),
    authorPubkey: normalizedId(tag[3]),
    threadRootId: tag[4] ? normalizedId(tag[4]) : null,
  };
  return isValidReference(reference) ? reference : null;
}

/**
 * Provenance to stamp on a forward of `message` taken from `sourceChannelId`.
 *
 * Forwarding an already-forwarded copy keeps pointing at the first original,
 * so attribution survives a chain instead of degrading to "forwarded from the
 * person who forwarded it". Returns null when the message cannot be
 * attributed (optimistic row, missing author, non-UUID channel).
 */
export function forwardProvenanceFor(
  message: TimelineMessage,
  sourceChannelId: string | null | undefined,
): ForwardedFromReference | null {
  // An edited forward no longer represents the original author's words. Stop
  // provenance inheritance here so a second forward attributes the edited copy
  // to its actual author rather than laundering new text as the original.
  const inherited = message.edited
    ? null
    : getForwardedFromReference(message.tags);
  if (inherited) return inherited;

  const threadRootId = normalizedId(
    getThreadReference(message.tags ?? []).rootId,
  );
  const reference: ForwardedFromReference = {
    sourceEventId: normalizedId(message.id),
    sourceChannelId: normalizedId(sourceChannelId),
    authorPubkey: normalizePubkey(message.pubkey ?? ""),
    threadRootId: threadRootId || null,
  };
  return isValidReference(reference) ? reference : null;
}

/** Whether the action bar should offer "Forward" for this row. */
export function canForwardMessage(
  message: TimelineMessage,
  sourceChannelId: string | null | undefined,
): boolean {
  return (
    !message.pending &&
    message.kind === KIND_STREAM_MESSAGE &&
    forwardProvenanceFor(message, sourceChannelId) !== null
  );
}

export function requiresValidatedForwardTransport(
  forwardedFrom: ForwardedFromReference | null | undefined,
): boolean {
  return forwardedFrom !== null && forwardedFrom !== undefined;
}

/** The source body, verbatim. */
export type ForwardPayload = {
  content: string;
  /** Content tags that give the body meaning (imeta, emoji, mention, previews). */
  contentTags: string[][];
  forwardedFrom: ForwardedFromReference;
};

/**
 * Everything a forward sends: the verbatim body, the content tags that give it
 * meaning, and the provenance marker.
 *
 * Deliberately omits mention recipients. The source `p` tags are that
 * message's notification audience; re-emitting them would page people who were
 * mentioned in another conversation. `mention` reference tags are kept so the
 * copied text still renders its chips — they identify, they do not notify.
 */
export function buildForwardPayload(
  message: TimelineMessage,
  sourceChannelId: string | null | undefined,
): ForwardPayload | null {
  const forwardedFrom = forwardProvenanceFor(message, sourceChannelId);
  if (!forwardedFrom) return null;

  return {
    content: message.body,
    // The profile lookup only feeds `mentionPubkeys`, which a forward drops,
    // so the semantic tag set is the same with or without it.
    contentTags: getSendToChannelSemantics(message).semanticTags,
    forwardedFrom,
  };
}

const PREVIEW_MAX_LENGTH = 140;
/**
 * One-line excerpt of the body a forward would copy, for the destination
 * picker's confirmation. Markdown is left as written: the point is to confirm
 * "this is the message I picked", and stripping syntax can make two adjacent
 * messages look identical.
 */
export function forwardPreview(body: string | undefined | null): string {
  const collapsed = (body ?? "").replace(/\s+/g, " ").trim();
  return collapsed.length > PREVIEW_MAX_LENGTH
    ? `${collapsed.slice(0, PREVIEW_MAX_LENGTH - 1).trimEnd()}…`
    : collapsed;
}
