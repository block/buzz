import {
  mentionMatchCandidates,
  type MentionPubkeyCandidate,
} from "./extractMentionPubkeys";
import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { isThreadReply } from "@/features/messages/lib/threading";
import type { DraftMentionRef } from "@/features/messages/lib/useDrafts";
import type { TimelineMessage } from "@/features/messages/types";
import type { MessageComposerEditTarget } from "@/features/messages/ui/MessageComposer.types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getMentionTagPubkey,
  mentionIdentityTags,
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
    content,
  );
  const presentNames = new Set(
    mentionOccurrences(
      content,
      (mentionNames ?? []).map((displayName) => ({ displayName })),
    ).flatMap((match) =>
      match.candidates.map((candidate) => candidate.displayName),
    ),
  );
  const refs = (mentionNames ?? [])
    .filter((displayName) => presentNames.has(displayName))
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
      mentionIdentityTags(tags)
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
): Pick<
  MessageComposerEditTarget,
  "mentionRefs" | "unresolvedMentionPubkeys" | "unresolvedMentionRefs"
> {
  const mentionRefs = resolveEditMentionRefs(
    content,
    tags,
    profiles,
    isAgentPubkey,
  );
  const unresolvedMentionPubkeys = unresolvedEditMentionPubkeys(
    content,
    tags,
    mentionRefs,
  );
  // Retain ambiguous aliases as candidates, not bindings. Matching each key in
  // isolation supplies its aliases; matching all candidates together prevents a
  // shorter alias from claiming another identity's longer literal occurrence.
  const candidates = [
    ...new Set(
      mentionIdentityTags(tags)
        .map(getMentionTagPubkey)
        .filter((key): key is string => Boolean(key)),
    ),
  ].flatMap((pubkey) =>
    (
      resolveMentionProps([["mention", pubkey]], profiles, content)
        .mentionNames ?? []
    ).map((displayName) => ({
      displayName,
      pubkey,
      isAgent: isAgentPubkey(pubkey),
    })),
  );
  const unresolved = new Set(unresolvedMentionPubkeys);
  const unresolvedMentionRefs = mentionOccurrences(content, candidates).flatMap(
    (match) =>
      match.candidates.filter((candidate) => unresolved.has(candidate.pubkey)),
  );
  return {
    mentionRefs,
    unresolvedMentionPubkeys,
    ...(unresolvedMentionRefs.length ? { unresolvedMentionRefs } : {}),
  };
}

/** Keep historical references only while their original occurrence still owns them. */
export function snapshotUnresolvedEditMentionPubkeys(
  content: string,
  originalContent: string,
  editTarget: Pick<
    MessageComposerEditTarget,
    "mentionRefs" | "unresolvedMentionPubkeys" | "unresolvedMentionRefs"
  >,
  getMentionRefs: (
    content: string,
    fallback: readonly DraftMentionRef[],
  ) => DraftMentionRef[],
): string[] {
  const candidates = editTarget.unresolvedMentionRefs ?? [];
  // Resolve all historical competitors together, retaining ties as references,
  // never choosing a binding for an ambiguous label.
  const present = getMentionRefs(content, [
    ...(editTarget.mentionRefs ?? []),
    ...candidates,
  ]);
  return (editTarget.unresolvedMentionPubkeys ?? []).filter((pubkey) => {
    const aliases = candidates.filter((ref) => ref.pubkey === pubkey);
    // Without a historical alias (e.g. unloaded profile), there is no evidence
    // tying this key to any edited text. Preserve only an unchanged body rather
    // than attaching old identities to a replacement's unrelated @mention.
    if (!aliases.length) {
      const originalRefs = editTarget.mentionRefs ?? [];
      return (
        content === originalContent &&
        getMentionRefs(content, originalRefs).every((ref) =>
          originalRefs.some(
            (original) =>
              original.pubkey === ref.pubkey &&
              original.displayName === ref.displayName,
          ),
        )
      );
    }
    return aliases.some((ref) =>
      present.some(
        (winner) =>
          winner.pubkey === pubkey && winner.displayName === ref.displayName,
      ),
    );
  });
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
  memberCandidates: readonly MentionPubkeyCandidate[] = [],
  selectedDisplayNames: Iterable<string> = [],
  fallbackRefs: readonly DraftMentionRef[] = [],
  competingDisplayNames: readonly string[] = [],
): DraftMentionRef[] {
  // Fallback references may contain ambiguous historical ties. Keep them as
  // candidates, not a name-to-key Map: collapsing ties would invent a binding.
  // Non-binding competitors also take part when snapshotting resolved refs.
  const personaLabels = [...selectedDisplayNames];
  const currentNames = new Set(
    [...mentions.keys(), ...personaLabels].map((name) =>
      name.trim().toLowerCase(),
    ),
  );
  const fallback = fallbackRefs.filter(
    (ref) => !currentNames.has(ref.displayName.trim().toLowerCase()),
  );
  const agentNames = new Set(
    selectedAgentNames.map((name) => name.trim().toLowerCase()),
  );
  const refs = [
    ...fallback,
    ...[...mentions].map(([displayName, pubkey]) => ({
      displayName,
      pubkey,
      isAgent: agentNames.has(displayName.trim().toLowerCase()),
    })),
  ];
  const presentNames = new Set(
    mentionOccurrences(content, [
      ...fallback,
      ...mentionMatchCandidates({
        selectedMentions: mentions,
        memberCandidates,
        selectedDisplayNames: [
          ...personaLabels,
          ...fallback.map((ref) => ref.displayName),
        ],
        competingDisplayNames,
      }),
    ]).flatMap((match) =>
      match.candidates.map((candidate) => candidate.displayName),
    ),
  );
  return refs
    .filter(({ displayName }) => presentNames.has(displayName))
    .map((ref) => ({ ...ref, pubkey: normalizePubkey(ref.pubkey) }));
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
