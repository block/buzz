import {
  buildImetaTags,
  imetaMediaFromTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { DraftState } from "@/features/messages/lib/useDrafts";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";

export type DraftPayload = {
  kind: number;
  created_at: number;
  tags: string[][];
  content: string;
  pubkey: string;
};

export type DecodedDraftPayload = {
  draftKey: string;
  draft: DraftState;
};

function isStringTag(tag: unknown): tag is string[] {
  return Array.isArray(tag) && tag.every((value) => typeof value === "string");
}

function isSupportedKind(kind: unknown): kind is number {
  return kind === KIND_STREAM_MESSAGE;
}

/** Serialize an interoperable unsigned event; editor-only selection stays local. */
export function serializeDraftPayload(
  draftKey: string,
  draft: DraftState,
  pubkey: string,
  createdAt = Math.floor(Date.now() / 1_000),
): string {
  const tags = [["h", draft.channelId], ...buildImetaTags(draft.pendingImeta)];
  if (draftKey.startsWith("thread:")) {
    const rootId = draftKey.slice("thread:".length);
    if (rootId) tags.splice(1, 0, ["e", rootId, "", "reply"]);
  }
  return JSON.stringify({
    kind: KIND_STREAM_MESSAGE,
    created_at: createdAt,
    tags,
    content: draft.content,
    pubkey,
  } satisfies DraftPayload);
}

/**
 * Validate a decrypted NIP-37 payload before it reaches the local draft store.
 * The compose key is reconstructed solely from encrypted inner tags.
 */
export function parseDraftPayload(
  plaintext: string,
  expectedPubkey: string,
  expectedKind: number,
  fallbackUpdatedAt: string,
): DecodedDraftPayload | null {
  try {
    const value: unknown = JSON.parse(plaintext);
    if (typeof value !== "object" || value === null) return null;
    const payload = value as Partial<DraftPayload>;
    if (
      !isSupportedKind(payload.kind) ||
      payload.kind !== expectedKind ||
      typeof payload.content !== "string" ||
      typeof payload.pubkey !== "string" ||
      payload.pubkey.toLowerCase() !== expectedPubkey.toLowerCase() ||
      !Array.isArray(payload.tags) ||
      !payload.tags.every(isStringTag)
    ) {
      return null;
    }
    const channelId = getChannelIdFromTags(payload.tags);
    if (!channelId) return null;
    const thread = getThreadReference(payload.tags);
    const draftKey = thread.rootId ? `thread:${thread.rootId}` : channelId;
    const media = imetaMediaFromTags(payload.tags);
    const createdAt =
      typeof payload.created_at === "number" &&
      Number.isFinite(payload.created_at)
        ? new Date(payload.created_at * 1_000).toISOString()
        : fallbackUpdatedAt;
    return {
      draftKey,
      draft: {
        content: payload.content,
        selectionStart: payload.content.length,
        selectionEnd: payload.content.length,
        channelId,
        createdAt,
        updatedAt: fallbackUpdatedAt,
        pendingImeta: media,
        spoileredAttachmentUrls: [],
        status: "active",
      },
    };
  } catch {
    return null;
  }
}
