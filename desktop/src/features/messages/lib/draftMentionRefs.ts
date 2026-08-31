import { hasMention } from "@/features/messages/lib/hasMention";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { isThreadReply } from "@/features/messages/lib/threading";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import type { TimelineMessage } from "@/features/messages/types";
import type { MessageComposerEditTarget } from "@/features/messages/ui/MessageComposer.types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getMentionTagPubkey,
  resolveMentionProps,
} from "@/shared/lib/resolveMentionNames";

export function resolveEditMentionRefs(
  content: string,
  tags: string[][] | undefined,
  profiles: UserProfileLookup | undefined,
  isAgentPubkey: (pubkey: string) => boolean,
): DraftMentionRef[] {
  const { mentionNames, mentionPubkeysByName } = resolveMentionProps(
    tags,
    profiles,
  );
  const refs = (mentionNames ?? [])
    .filter((displayName) => hasMention(content, displayName))
    .flatMap((displayName) => {
      const pubkey = mentionPubkeysByName?.[displayName.toLowerCase()];
      return pubkey
        ? [
            {
              displayName,
              pubkey,
              isAgent: isAgentPubkey(normalizePubkey(pubkey)),
            },
          ]
        : [];
    });
  return refs;
}

function unresolvedEditMentionPubkeys(
  content: string,
  tags: string[][] | undefined,
  refs: readonly DraftMentionRef[],
): string[] {
  if (!content.includes("@")) {
    return [];
  }

  const resolved = new Set(refs.map((ref) => normalizePubkey(ref.pubkey)));
  return [
    ...new Set(
      (tags ?? [])
        .map(getMentionTagPubkey)
        .filter((pubkey): pubkey is string => Boolean(pubkey))
        .map(normalizePubkey)
        .filter((pubkey) => pubkey && !resolved.has(pubkey)),
    ),
  ];
}

export function buildEditMentionState(
  content: string,
  tags: string[][] | undefined,
  profiles: UserProfileLookup | undefined,
  isAgentPubkey: (pubkey: string) => boolean,
): Pick<MessageComposerEditTarget, "mentionRefs" | "unresolvedMentionPubkeys"> {
  const mentionRefs = resolveEditMentionRefs(
    content,
    tags,
    profiles,
    isAgentPubkey,
  );
  return {
    mentionRefs,
    unresolvedMentionPubkeys: unresolvedEditMentionPubkeys(
      content,
      tags,
      mentionRefs,
    ),
  };
}

export function buildMessageComposerEditTarget(
  message: TimelineMessage,
  profiles: UserProfileLookup | undefined,
  isAgentPubkey: (pubkey: string) => boolean,
): MessageComposerEditTarget {
  const mentionState = buildEditMentionState(
    message.body,
    message.tags,
    profiles,
    isAgentPubkey,
  );
  return {
    author: message.author,
    body: message.body,
    id: message.id,
    isThreadReply: isThreadReply(message.tags ?? []),
    imetaMedia: imetaMediaFromTags(message.tags),
    ...mentionState,
  };
}

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
