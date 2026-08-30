import { getMentionOffsets } from "./hasMention";
import { extractNostrUriPubkeys } from "./nostrUriMentions";

export type MentionPubkeyCandidate = {
  displayName: string | null;
  isMember: boolean;
  pubkey?: string;
};

type MentionMatch = {
  displayName: string;
  pubkey?: string;
};

function normalizeDisplayName(name: string): string {
  return name.trim().toLowerCase();
}

/**
 * Returns explicit selected mention pubkeys, manually typed channel-member
 * mentions, and NIP-21 `nostr:` profile URIs. At each `@` offset, only the
 * longest valid display name wins so a member whose name prefixes another
 * member is not spuriously tagged.
 */
export function extractMentionPubkeys({
  text,
  selectedMentions,
  selectedDisplayNames,
  memberCandidates,
}: {
  text: string;
  selectedMentions: ReadonlyMap<string, string>;
  selectedDisplayNames?: Iterable<string>;
  memberCandidates: readonly MentionPubkeyCandidate[];
}): string[] {
  const selectedNames = new Set(
    [...selectedMentions.keys(), ...(selectedDisplayNames ?? [])].map(
      normalizeDisplayName,
    ),
  );
  const matchesByOffset = new Map<number, MentionMatch[]>();

  const addMatches = (displayName: string, pubkey?: string) => {
    const trimmedName = displayName.trim();
    if (!trimmedName) return;

    for (const offset of getMentionOffsets(text, trimmedName)) {
      const matches = matchesByOffset.get(offset) ?? [];
      matches.push({ displayName: trimmedName, pubkey });
      matchesByOffset.set(offset, matches);
    }
  };

  for (const [displayName, pubkey] of selectedMentions) {
    addMatches(displayName, pubkey);
  }
  for (const displayName of selectedDisplayNames ?? []) {
    addMatches(displayName);
  }
  for (const candidate of memberCandidates) {
    if (
      candidate.pubkey &&
      candidate.isMember &&
      candidate.displayName &&
      !selectedNames.has(normalizeDisplayName(candidate.displayName))
    ) {
      addMatches(candidate.displayName, candidate.pubkey);
    }
  }

  const winningPubkeys = new Set<string>();
  for (const matches of matchesByOffset.values()) {
    const longestNameLength = Math.max(
      ...matches.map((match) => match.displayName.length),
    );
    for (const match of matches) {
      if (match.pubkey && match.displayName.length === longestNameLength) {
        winningPubkeys.add(match.pubkey);
      }
    }
  }

  const pubkeys: string[] = [];
  for (const [, pubkey] of selectedMentions) {
    if (winningPubkeys.delete(pubkey)) pubkeys.push(pubkey);
  }
  for (const candidate of memberCandidates) {
    if (candidate.pubkey && winningPubkeys.delete(candidate.pubkey)) {
      pubkeys.push(candidate.pubkey);
    }
  }

  // A `nostr:npub…` URI addresses someone the same way `@Name` does, but it
  // carries its own pubkey, so it resolves without a display-name match.
  const taggedPubkeys = new Set(
    pubkeys.map((pubkey) => pubkey.trim().toLowerCase()),
  );
  for (const pubkey of extractNostrUriPubkeys(text)) {
    if (!taggedPubkeys.has(pubkey)) {
      taggedPubkeys.add(pubkey);
      pubkeys.push(pubkey);
    }
  }

  return pubkeys;
}
