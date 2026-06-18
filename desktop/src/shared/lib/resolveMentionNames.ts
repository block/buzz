import type { UserProfileSummary } from "@/shared/api/types";

export const MENTION_REFERENCE_TAG = "mention";

export function getMentionTagPubkey(tag: string[]): string | null {
  if ((tag[0] !== "p" && tag[0] !== MENTION_REFERENCE_TAG) || !tag[1]) {
    return null;
  }

  return tag[1].toLowerCase();
}

/**
 * Resolves display names for mentioned users from message `p` tags and
 * non-notifying `mention` reference tags.
 *
 * `p` tags drive notification/search semantics. `mention` tags only preserve
 * render metadata for reference-only mentions.
 */
export function resolveMentionNames(
  tags: string[][] | undefined,
  profiles: Record<string, UserProfileSummary> | undefined,
): string[] | undefined {
  if (!profiles || !tags) {
    return undefined;
  }

  const names = new Set<string>();

  for (const tag of tags) {
    const pubkey = getMentionTagPubkey(tag);
    if (!pubkey) {
      continue;
    }

    const profile = profiles[pubkey];
    const displayName = profile?.displayName?.trim();

    if (displayName) {
      names.add(displayName);
    }
  }

  return names.size > 0 ? [...names] : undefined;
}

export function resolveMentionPubkeysByName(
  tags: string[][] | undefined,
  profiles: Record<string, UserProfileSummary> | undefined,
): Record<string, string> | undefined {
  if (!profiles || !tags) {
    return undefined;
  }

  const pubkeysByName: Record<string, string> = {};

  for (const tag of tags) {
    const pubkey = getMentionTagPubkey(tag);
    if (!pubkey) {
      continue;
    }

    const displayName = profiles[pubkey]?.displayName?.trim();
    if (displayName) {
      pubkeysByName[displayName.toLowerCase()] = pubkey;
    }
  }

  return Object.keys(pubkeysByName).length > 0 ? pubkeysByName : undefined;
}
