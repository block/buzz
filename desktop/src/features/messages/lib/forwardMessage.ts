import type { Channel, RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE_FORWARD } from "@/shared/constants/kinds";

/**
 * Forward-message (kind 40009) tag composition and parsing.
 *
 * A forward is a snapshot: the complete signed original event rides in a
 * single `fwd` tag, with `k` (original kind), `fwd-src` (source channel +
 * visibility label) and — for open sources only — a NIP-18-style `q` tag.
 * The relay re-verifies the embedded event, so everything built here must
 * stay byte-faithful to the original.
 */

export type ForwardSourceType = "channel" | "private" | "dm";

/** Kinds a forward may embed (relay allowlist). Kind 40009 itself is not
 *  embeddable — forwarding a forward flattens to ITS embedded original. */
export const FORWARDABLE_SOURCE_KINDS: ReadonlySet<number> = new Set([
  9, 40002, 45001, 45003,
]);

/** Relay ceiling for the serialized embedded event (64 KiB). */
export const MAX_FWD_TAG_BYTES = 64 * 1024;

/** Whether the "Forward message…" action applies to a message of this kind. */
export function canForwardMessageKind(kind: number | undefined): boolean {
  if (kind === undefined) return false;
  return (
    FORWARDABLE_SOURCE_KINDS.has(kind) || kind === KIND_STREAM_MESSAGE_FORWARD
  );
}

/** Map a channel row to the `fwd-src` visibility label the relay validates. */
export function forwardSourceTypeForChannel(
  channel: Pick<Channel, "channelType" | "visibility">,
): ForwardSourceType {
  if (channel.channelType === "dm") return "dm";
  return channel.visibility === "open" ? "channel" : "private";
}

/** NIP-29 group scope (`h` tag) of an event, or null when untagged. */
export function getEventChannelId(
  event: Pick<RelayEvent, "tags">,
): string | null {
  const value = event.tags.find((tag) => tag[0] === "h")?.[1];
  return typeof value === "string" && value.length > 0 ? value : null;
}

export type ForwardEnvelope = {
  /** The complete signed original event embedded in the `fwd` tag. */
  original: RelayEvent;
  /** Source channel uuid from the `fwd-src` tag. */
  sourceChannelId: string;
  /** Source visibility label from the `fwd-src` tag. */
  sourceType: ForwardSourceType;
};

function isForwardSourceType(value: unknown): value is ForwardSourceType {
  return value === "channel" || value === "private" || value === "dm";
}

function parseEmbeddedEvent(raw: string): RelayEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const candidate = parsed as Record<string, unknown>;
  if (
    typeof candidate.id !== "string" ||
    typeof candidate.pubkey !== "string" ||
    typeof candidate.created_at !== "number" ||
    typeof candidate.kind !== "number" ||
    typeof candidate.content !== "string" ||
    typeof candidate.sig !== "string" ||
    !Array.isArray(candidate.tags) ||
    !candidate.tags.every(
      (tag) =>
        Array.isArray(tag) && tag.every((part) => typeof part === "string"),
    )
  ) {
    return null;
  }
  return {
    id: candidate.id,
    pubkey: candidate.pubkey,
    created_at: candidate.created_at,
    kind: candidate.kind,
    tags: candidate.tags as string[][],
    content: candidate.content,
    sig: candidate.sig,
  };
}

/**
 * Parse a kind-40009 tag set into its forward envelope. Returns null when the
 * shape is malformed (missing/duplicated `fwd`, unparseable embedded event,
 * or missing `fwd-src`) so callers can render a fallback instead of throwing.
 */
export function parseForwardEnvelope(
  tags: ReadonlyArray<ReadonlyArray<string>>,
): ForwardEnvelope | null {
  const fwdTags = tags.filter((tag) => tag[0] === "fwd");
  if (fwdTags.length !== 1) return null;
  const raw = fwdTags[0][1];
  if (typeof raw !== "string" || raw.length === 0) return null;

  const original = parseEmbeddedEvent(raw);
  if (!original) return null;

  const source = tags.find((tag) => tag[0] === "fwd-src");
  const sourceChannelId = source?.[1];
  const sourceType = source?.[2];
  if (
    typeof sourceChannelId !== "string" ||
    sourceChannelId.length === 0 ||
    !isForwardSourceType(sourceType)
  ) {
    return null;
  }

  return { original, sourceChannelId, sourceType };
}

/**
 * Resolve the event a forward of `event` must embed. Forward depth is always
 * 1: forwarding a kind-40009 message forwards ITS embedded original
 * (flatten); any other allowlisted kind embeds itself. Returns null when the
 * event cannot be forwarded.
 */
export function resolveForwardOriginal(event: RelayEvent): RelayEvent | null {
  if (event.kind === KIND_STREAM_MESSAGE_FORWARD) {
    return parseForwardEnvelope(event.tags)?.original ?? null;
  }
  return FORWARDABLE_SOURCE_KINDS.has(event.kind) ? event : null;
}

/**
 * Build the forward metadata tags (`fwd`, `k`, `fwd-src`, `q`, `imeta`) for a
 * kind-40009 event. The destination `h` tag and any note-mention `p` tags are
 * appended by the send path, not here.
 *
 * - `fwd-src` uuid is derived from the original's own `h` tag (the relay
 *   rejects any mismatch, so it is never caller-supplied).
 * - `q` is emitted only for open (`"channel"`) sources.
 * - `imeta` tags are copied verbatim so attachments survive the hop.
 *
 * Throws on inputs that can never produce a relay-valid event.
 */
export function buildForwardTags(input: {
  original: RelayEvent;
  sourceType: ForwardSourceType;
}): string[][] {
  const { original, sourceType } = input;

  if (!FORWARDABLE_SOURCE_KINDS.has(original.kind)) {
    throw new Error(`Messages of kind ${original.kind} cannot be forwarded.`);
  }
  if (!original.sig) {
    throw new Error("The original message is missing its signature.");
  }
  const sourceChannelId = getEventChannelId(original);
  if (!sourceChannelId) {
    throw new Error("The original message has no source channel.");
  }

  // Serialize exactly the signed NIP-01 fields — local-only fields
  // (localKey, pending) must never leak into the embedded snapshot.
  const embedded = JSON.stringify({
    id: original.id,
    pubkey: original.pubkey,
    created_at: original.created_at,
    kind: original.kind,
    tags: original.tags,
    content: original.content,
    sig: original.sig,
  });
  if (new TextEncoder().encode(embedded).length > MAX_FWD_TAG_BYTES) {
    throw new Error("This message is too large to forward.");
  }

  const tags: string[][] = [
    ["fwd", embedded],
    ["k", String(original.kind)],
    ["fwd-src", sourceChannelId, sourceType],
  ];
  if (sourceType === "channel") {
    tags.push(["q", original.id, "", original.pubkey]);
  }
  for (const tag of original.tags) {
    if (tag[0] === "imeta") {
      tags.push([...tag]);
    }
  }
  return tags;
}
