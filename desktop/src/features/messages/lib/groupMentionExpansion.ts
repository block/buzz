import type { UserGroup } from "@/shared/api/relayGroups";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type GroupMentionExpansion = {
  mentionPubkeys: string[];
  markerTags: string[][];
};

export function expandGroupMentions(input: {
  channelMemberPubkeys: Iterable<string>;
  groups: readonly UserGroup[];
  individualMentionPubkeys: Iterable<string>;
}): GroupMentionExpansion {
  const channelMembers = new Set(
    [...input.channelMemberPubkeys].map(normalizePubkey),
  );
  const mentionPubkeys = new Set(
    [...input.individualMentionPubkeys].map(normalizePubkey),
  );
  const markerTags: string[][] = [];
  const seenGroups = new Set<string>();

  for (const group of input.groups) {
    if (seenGroups.has(group.id)) continue;
    seenGroups.add(group.id);
    markerTags.push(["group", group.id, group.handle]);
    for (const pubkey of group.memberPubkeys) {
      const normalized = normalizePubkey(pubkey);
      if (channelMembers.has(normalized)) {
        mentionPubkeys.add(normalized);
      }
    }
  }

  return {
    mentionPubkeys: [...mentionPubkeys],
    markerTags,
  };
}
