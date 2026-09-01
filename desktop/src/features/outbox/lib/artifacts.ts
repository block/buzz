import {
  KIND_FORUM_COMMENT,
  KIND_FORUM_POST,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

export const OUTBOX_MESSAGE_KINDS = [
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
  KIND_FORUM_POST,
  KIND_FORUM_COMMENT,
] as const;

export type OutboxArtifactKind = "document" | "image" | "video";

export type OutboxArtifact = {
  id: string;
  eventId: string;
  eventKind: number;
  authorPubkey: string;
  channelId: string | null;
  createdAt: number;
  filename: string;
  kind: OutboxArtifactKind;
  mimeType: string;
  sha256: string;
  size: number | undefined;
  sourceContent: string;
  sourceSummary: string;
  sourceTags: string[][];
  url: string;
};

function isOutboxDelivery(tags: readonly string[][]): boolean {
  return tags.some((tag) => tag[0] === "buzz-outbox" && tag[1] === "1");
}

function getChannelId(tags: readonly string[][]): string | null {
  return tags.find((tag) => tag[0] === "h")?.[1] ?? null;
}

function fallbackFilename(url: string, index: number): string {
  try {
    const tail = new URL(url).pathname.split("/").filter(Boolean).at(-1);
    if (tail) return decodeURIComponent(tail);
  } catch {
    // Relative and malformed URLs still receive a useful stable label.
  }
  return `Artifact ${index + 1}`;
}

function classifyArtifact(mimeType: string): OutboxArtifactKind {
  const normalized = mimeType.toLowerCase();
  if (normalized.startsWith("image/")) return "image";
  if (normalized.startsWith("video/")) return "video";
  return "document";
}

function summarizeSource(content: string, attachmentUrls: ReadonlySet<string>) {
  const summary = content
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => {
      if (!line) return false;
      for (const url of attachmentUrls) {
        if (line.includes(`](${url})`)) return false;
      }
      return true;
    })
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();

  if (summary.length <= 180) return summary;
  return `${summary.slice(0, 177).trimEnd()}…`;
}

/**
 * Projects durable file attachments from known-agent messages into the
 * product's Outbox read model. An attachment is the handoff boundary: local
 * workspace paths never become discoverable or clickable by accident.
 */
export function buildOutboxArtifacts(
  events: readonly RelayEvent[],
  knownAgentPubkeys: ReadonlySet<string>,
): OutboxArtifact[] {
  const normalizedAgents = new Set(
    [...knownAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  const artifacts: OutboxArtifact[] = [];

  for (const event of events) {
    if (!normalizedAgents.has(normalizePubkey(event.pubkey))) continue;
    if (!isOutboxDelivery(event.tags)) continue;

    const imetaEntries = [...parseImetaTags(event.tags).values()];
    if (imetaEntries.length === 0) continue;

    const attachmentUrls = new Set(imetaEntries.map((entry) => entry.url));
    const sourceSummary = summarizeSource(event.content, attachmentUrls);

    imetaEntries.forEach((entry, index) => {
      const mimeType = entry.m || "application/octet-stream";
      artifacts.push({
        id: `${event.id}:${index}`,
        eventId: event.id,
        eventKind: event.kind,
        authorPubkey: normalizePubkey(event.pubkey),
        channelId: getChannelId(event.tags),
        createdAt: event.created_at,
        filename: entry.filename?.trim() || fallbackFilename(entry.url, index),
        kind: classifyArtifact(mimeType),
        mimeType,
        sha256: entry.x || "",
        size:
          Number.isFinite(entry.size) && entry.size >= 0
            ? entry.size
            : undefined,
        sourceContent: event.content,
        sourceSummary,
        sourceTags: event.tags,
        url: entry.url,
      });
    });
  }

  return artifacts.sort(
    (left, right) =>
      right.createdAt - left.createdAt || left.id.localeCompare(right.id),
  );
}
