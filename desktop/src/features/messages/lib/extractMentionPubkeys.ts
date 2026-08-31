import { getMentionOffsets } from "./hasMention";

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

/** Keep a second same-name selection from rebinding text already in the draft. */
export function selectedMentionLabel(
  displayName: string,
  pubkey: string,
  selectedMentions: ReadonlyMap<string, string>,
): string {
  const bindings = new Map(
    [...selectedMentions].map(([label, key]) => [
      normalizeDisplayName(label),
      key.toLowerCase(),
    ]),
  );
  const conflicts = (label: string) => {
    const existing = bindings.get(normalizeDisplayName(label));
    return existing !== undefined && existing !== pubkey.toLowerCase();
  };
  if (!conflicts(displayName)) return displayName;
  const qualified = `${displayName} (${pubkey.toLowerCase()})`;
  let label = qualified;
  let suffix = 2;
  // A display name may itself look qualified. Never overwrite that binding.
  while (conflicts(label)) label = `${qualified} ${suffix++}`;
  return label;
}

/** Reserve each label before binding the next identity in a multi-agent selection. */
export function selectedMentionLabels<
  T extends { displayName: string; pubkey?: string },
>(
  selections: readonly T[],
  selectedMentions: ReadonlyMap<string, string>,
): T[] {
  const bindings = new Map(selectedMentions);
  return selections.map((selected) => {
    if (!selected.pubkey) return selected;
    const displayName = selectedMentionLabel(
      selected.displayName,
      selected.pubkey,
      bindings,
    );
    bindings.set(displayName, selected.pubkey);
    return { ...selected, displayName };
  });
}

/**
 * Returns explicit selected mention pubkeys and manually typed channel-member
 * mentions. At each `@` offset, only the longest valid display name wins so a
 * member whose name prefixes another member is not spuriously tagged.
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
    const winners = matches.filter(
      (match) => match.displayName.length === longestNameLength,
    );
    const identities = new Set(
      winners.flatMap((match) =>
        match.pubkey ? [match.pubkey.toLowerCase()] : [],
      ),
    );
    if (identities.size > 1) {
      throw new Error(
        `The mention @${winners[0].displayName} is ambiguous. Choose a recipient from the mention picker.`,
      );
    }
    for (const match of winners) {
      if (match.pubkey) winningPubkeys.add(match.pubkey);
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
  return pubkeys;
}
