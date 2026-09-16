import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";

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
 * Build shared candidates for selected bindings, personas and typed members.
 * Selected labels suppress same-name member fallback; unbound selected labels
 * still compete with shorter keyed labels for exact occurrence ownership.
 */
export function mentionMatchCandidates({
  selectedMentions,
  selectedDisplayNames,
  memberCandidates,
  competingDisplayNames,
}: {
  selectedMentions: ReadonlyMap<string, string>;
  selectedDisplayNames?: Iterable<string>;
  competingDisplayNames?: Iterable<string>;
  memberCandidates: readonly MentionPubkeyCandidate[];
}): MentionMatch[] {
  const selectedLabels = [...(selectedDisplayNames ?? [])];
  const selectedNames = new Set(
    [...selectedMentions.keys(), ...selectedLabels].map(normalizeDisplayName),
  );
  const candidates: MentionMatch[] = [];

  const addMatches = (displayName: string, pubkey?: string) => {
    const trimmedName = displayName.trim();
    if (!trimmedName) return;

    candidates.push({ displayName: trimmedName, pubkey });
  };

  for (const [displayName, pubkey] of selectedMentions) {
    addMatches(displayName, pubkey);
  }
  for (const displayName of selectedLabels) {
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

  // Historical unresolved labels own ranges without binding or suppressing
  // current same-name members. They must also block shorter selected labels.
  for (const displayName of competingDisplayNames ?? []) {
    addMatches(displayName);
  }
  return candidates;
}

/** Extract recipients from the same exact occurrences used by draft routing. */
export function extractMentionPubkeys(options: {
  text: string;
  selectedMentions: ReadonlyMap<string, string>;
  selectedDisplayNames?: Iterable<string>;
  competingDisplayNames?: Iterable<string>;
  memberCandidates: readonly MentionPubkeyCandidate[];
}): string[] {
  const { text, selectedMentions, memberCandidates } = options;
  const candidates = mentionMatchCandidates(options);
  const winningPubkeys = new Set<string>();
  for (const { candidates: winners } of mentionOccurrences(text, candidates)) {
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
