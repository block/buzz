import type { RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_STREAM_MESSAGE_DIFF,
} from "@/shared/constants/kinds";

const HEX_EVENT_ID = /^[0-9a-f]{64}$/;
const PICKABLE_MESSAGE_KINDS = new Set<number>([
  ...CHANNEL_MESSAGE_EVENT_KINDS,
  KIND_STREAM_MESSAGE_DIFF,
]);

export type WorkflowMessageCandidate = {
  id: string;
  pubkey: string | null;
  content: string | null;
  createdAt: number | null;
};

export type WorkflowMessageCandidateInput = {
  id: string;
  pubkey?: string | null;
  content?: string | null;
  createdAt?: number | null;
};

export type WorkflowMessageEventValidation = {
  channelId: string;
  requestedId?: string | null;
};

/** Normalize an event ID without accepting alternate encodings. */
export function normalizeMessageEventId(eventId: string): string | null {
  const normalized = eventId.trim().toLowerCase();
  return HEX_EVENT_ID.test(normalized) ? normalized : null;
}

/**
 * Merge candidate sources in priority order. The first occurrence of an event
 * owns both its position and its presentation fields.
 */
export function mergeMessageCandidateSources(
  sources: readonly (readonly WorkflowMessageCandidateInput[])[],
): WorkflowMessageCandidate[] {
  const merged: WorkflowMessageCandidate[] = [];
  const seen = new Set<string>();

  for (const source of sources) {
    for (const candidate of source) {
      const id = normalizeMessageEventId(candidate.id);
      if (!id || seen.has(id)) continue;
      seen.add(id);
      merged.push({
        id,
        pubkey: candidate.pubkey ?? null,
        content: candidate.content ?? null,
        createdAt: candidate.createdAt ?? null,
      });
    }
  }

  return merged;
}

/** Whether an event is a user-pickable message in exactly the given channel. */
export function isPickableWorkflowMessageEvent(
  event: RelayEvent,
  channelId: string,
): boolean {
  return (
    PICKABLE_MESSAGE_KINDS.has(event.kind) &&
    event.tags.filter((tag) => tag[0] === "h").length === 1 &&
    event.tags.find((tag) => tag[0] === "h")?.[1] === channelId
  );
}

/**
 * Validate any fetched event before it can enrich a deterministic message ID.
 * Transport scoping is not trusted: the event must carry exactly one matching
 * channel tag, use a pickable kind, and match a requested ID when provided.
 */
export function validatedWorkflowMessageCandidate(
  event: RelayEvent | null | undefined,
  { channelId, requestedId }: WorkflowMessageEventValidation,
): WorkflowMessageCandidate | null {
  if (!event || !isPickableWorkflowMessageEvent(event, channelId)) return null;
  const eventId = normalizeMessageEventId(event.id);
  if (!eventId) return null;
  if (requestedId !== undefined && requestedId !== null) {
    const normalizedRequestedId = normalizeMessageEventId(requestedId);
    if (!normalizedRequestedId || eventId !== normalizedRequestedId)
      return null;
  }
  return {
    id: eventId,
    pubkey: event.pubkey,
    content: event.content,
    createdAt: event.created_at,
  };
}

export type WorkflowMessageSearchResult = {
  requestedId: string;
  event: RelayEvent | null | undefined;
};

/**
 * Validate exact events resolved from search-hit IDs. Search projections are
 * discovery hints only and never supply presentation or selection fields.
 */
export function validateWorkflowMessageSearchResults(
  results: readonly WorkflowMessageSearchResult[],
  channelId: string,
): WorkflowMessageCandidate[] {
  return results.flatMap(({ requestedId, event }) => {
    const candidate = validatedWorkflowMessageCandidate(event, {
      channelId,
      requestedId,
    });
    return candidate ? [candidate] : [];
  });
}

/**
 * Enrich a deterministic event-ID fallback only when an exact lookup returned
 * that same event in the expected channel and with a pickable message kind.
 * Invalid or unrelated fetches leave the fallback untouched.
 */
export function enrichMessageCandidateFromExactLookup(
  fallback: WorkflowMessageCandidate,
  event: RelayEvent | null | undefined,
  channelId: string,
): WorkflowMessageCandidate {
  const candidate = validatedWorkflowMessageCandidate(event, {
    channelId,
    requestedId: fallback.id,
  });
  return candidate ?? fallback;
}
