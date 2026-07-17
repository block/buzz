import type { DraftMentionRef } from "./useDrafts";

import { normalizePubkey } from "@/shared/lib/pubkey";
import { hasMention } from "./hasMention";

export function snapshotDraftMentionRefs(
  content: string,
  mentions: ReadonlyMap<string, string>,
  selectedAgentNames: readonly string[],
): DraftMentionRef[] {
  const agentNames = new Set(
    selectedAgentNames.map((name) => name.trim().toLowerCase()),
  );
  return [...mentions.entries()]
    .filter(([displayName]) => hasMention(content, displayName))
    .map(([displayName, pubkey]) => ({
      displayName,
      pubkey: normalizePubkey(pubkey),
      isAgent: agentNames.has(displayName.trim().toLowerCase()),
    }));
}

function normalizeDraftMentionRefs(
  refs: readonly DraftMentionRef[],
): DraftMentionRef[] {
  const normalized: DraftMentionRef[] = [];
  for (const ref of refs) {
    const displayName = ref.displayName.trim();
    const pubkey = normalizePubkey(ref.pubkey);
    if (displayName && pubkey) {
      normalized.push({ displayName, pubkey, isAgent: ref.isAgent });
    }
  }
  return normalized;
}

export function replaceWithDraftMentionRefs(
  refs: readonly DraftMentionRef[],
  mentions: Map<string, string>,
  personaMentions: Map<string, string>,
): { names: string[]; agentNames: string[] } {
  mentions.clear();
  personaMentions.clear();
  const normalized = normalizeDraftMentionRefs(refs);
  for (const ref of normalized) mentions.set(ref.displayName, ref.pubkey);
  const names = normalized.map((ref) => ref.displayName);
  const agentNames = normalized
    .filter((ref) => ref.isAgent)
    .map((ref) => ref.displayName);
  return { names, agentNames };
}
