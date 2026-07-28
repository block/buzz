import { hasMention, hasUnambiguousMention } from "./hasMention";

type MentionCandidate = {
  displayName: string | null;
  isAgent?: boolean;
  isMember?: boolean;
  pubkey?: string;
};

export function extractMentionPubkeysFromCandidates(
  text: string,
  registeredMentions: ReadonlyMap<string, string>,
  registeredPersonaNames: Iterable<string>,
  candidates: readonly MentionCandidate[],
): string[] {
  const pubkeys: string[] = [];
  const selectedDisplayNames = new Set(
    [...registeredMentions.keys(), ...registeredPersonaNames].map((name) =>
      name.trim().toLowerCase(),
    ),
  );
  const unselectedAgentNames = candidates.flatMap((candidate) => {
    const name = candidate.displayName?.trim();
    return candidate.pubkey &&
      candidate.isAgent &&
      name &&
      !selectedDisplayNames.has(name.toLowerCase())
      ? [name]
      : [];
  });

  for (const [displayName, pubkey] of registeredMentions) {
    if (hasMention(text, displayName)) pubkeys.push(pubkey);
  }

  for (const candidate of candidates) {
    const name = candidate.displayName;
    if (!candidate.pubkey || !name) continue;
    if (
      !candidate.isMember &&
      !(
        candidate.isAgent &&
        hasUnambiguousMention(text, name, unselectedAgentNames)
      )
    ) {
      continue;
    }
    if (
      !pubkeys.includes(candidate.pubkey) &&
      !selectedDisplayNames.has(name.trim().toLowerCase()) &&
      hasMention(text, name)
    ) {
      pubkeys.push(candidate.pubkey);
    }
  }

  return [...new Set(pubkeys)];
}
