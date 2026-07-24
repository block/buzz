import { hasMention } from "./hasMention";
import type { MentionCandidate } from "./mentionCandidates";

/**
 * Resolve `@Name` tokens in `text` to pubkeys.
 *
 * Explicit composer registrations (`mentionMap`) win. Remaining channel
 * members are matched by display name, at most one pubkey per name — when
 * two identities share a friendly name (cross-community collision), prefer
 * a managed agent so a single `@Name` does not emit duplicate `p` tags.
 *
 * Names already claimed by persona mentions are skipped so identity
 * candidates cannot collide with an in-composer persona token.
 */
export function collectMentionPubkeys(
  text: string,
  mentionMap: ReadonlyMap<string, string>,
  candidates: readonly MentionCandidate[],
  personaMentionNames: ReadonlySet<string> = new Set(),
): string[] {
  const pubkeys: string[] = [];
  const resolvedNames = new Set(
    [...personaMentionNames].map((name) => name.trim().toLowerCase()),
  );

  for (const [displayName, pubkey] of mentionMap) {
    if (!hasMention(text, displayName)) {
      continue;
    }
    pubkeys.push(pubkey);
    resolvedNames.add(displayName.trim().toLowerCase());
  }

  const ranked = [...candidates].sort((left, right) => {
    const rank = (candidate: MentionCandidate) => {
      if (candidate.isManagedAgent) return 0;
      if (candidate.isAgent) return 1;
      return 2;
    };
    return rank(left) - rank(right);
  });

  for (const candidate of ranked) {
    if (!candidate.pubkey || !candidate.isMember) {
      continue;
    }
    if (pubkeys.includes(candidate.pubkey)) {
      continue;
    }
    const name = candidate.displayName;
    if (!name) {
      continue;
    }
    const key = name.trim().toLowerCase();
    if (resolvedNames.has(key)) {
      continue;
    }
    if (!hasMention(text, name)) {
      continue;
    }
    pubkeys.push(candidate.pubkey);
    resolvedNames.add(key);
  }

  return [...new Set(pubkeys)];
}
