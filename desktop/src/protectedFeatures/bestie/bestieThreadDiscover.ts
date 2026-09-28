/**
 * Map home-feed / search signals into Threads upserts for agent participation.
 */

import { getThreadReference, isThreadReply } from "@/features/messages/lib/threading";
import { parseMessageLink } from "@/features/messages/lib/messageLink";
import type { FeedItem } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { bestieThreadId } from "./bestieThreadProtocol";
import type { BestieThreadUpsertInput } from "./bestieThreadTypes";

export function bestieThreadRootFromTags(
  eventId: string,
  tags: string[][],
): string {
  if (isThreadReply(tags)) {
    return getThreadReference(tags).rootId ?? eventId;
  }
  return eventId;
}

/** Threads the agent authored in (excludes Assistant DM when given). */
export function bestieParticipatingThreadInputs(input: {
  agentPubkey: string;
  excludeChannelIds?: ReadonlySet<string> | readonly string[];
  items: readonly FeedItem[];
}): BestieThreadUpsertInput[] {
  const agent = normalizePubkey(input.agentPubkey);
  if (!agent) return [];
  const exclude = new Set(
    [...(input.excludeChannelIds ?? [])].map((id) => id.trim()).filter(Boolean),
  );
  const byId = new Map<string, BestieThreadUpsertInput>();

  for (const item of input.items) {
    if (normalizePubkey(item.pubkey) !== agent) continue;
    const channelId = item.channelId?.trim();
    if (!channelId || exclude.has(channelId)) continue;
    if (item.channelType === "dm") continue;
    const rootEventId = bestieThreadRootFromTags(item.id, item.tags);
    const id = bestieThreadId(channelId, rootEventId);
    const existing = byId.get(id);
    const preview = (item.content ?? "").trim().slice(0, 280);
    const activityAt =
      typeof item.createdAt === "number" && Number.isFinite(item.createdAt)
        ? Math.floor(item.createdAt)
        : undefined;
    const priorActivity = existing?.lastActiveAt;
    byId.set(id, {
      channelId,
      channelName: item.channelName?.trim() || existing?.channelName || null,
      lastActiveAt:
        activityAt != null
          ? Math.max(activityAt, priorActivity ?? 0)
          : priorActivity,
      preview: preview || existing?.preview || "",
      rootEventId,
      source: "agent",
    });
  }

  return [...byId.values()];
}

export type ParsedBestieThreadAddTarget = {
  channelId: string;
  messageId: string;
  rootEventId: string;
};

/**
 * Parse + add input: buzz://message link, or `channelId rootOrEventId`.
 */
export function parseBestieThreadAddInput(
  raw: string,
): ParsedBestieThreadAddTarget | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  const link = parseMessageLink(trimmed);
  if (link.ok) {
    return {
      channelId: link.value.channelId,
      messageId: link.value.messageId,
      rootEventId: link.value.threadRootId ?? link.value.messageId,
    };
  }

  // Also accept hulabuzz:// via parseMessageLink (already does).
  const parts = trimmed.split(/[\s,]+/).filter(Boolean);
  if (parts.length >= 2) {
    const channelId = parts[0]!;
    const eventId = parts[1]!;
    if (channelId.length >= 8 && eventId.length >= 16) {
      return {
        channelId,
        messageId: eventId,
        rootEventId: eventId,
      };
    }
  }

  return null;
}
