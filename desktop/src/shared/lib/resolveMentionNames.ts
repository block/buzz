import type { UserProfileSummary } from "@/shared/api/types";

/**
 * Resolves display names for mentioned users from message `p` tags.
 *
 * Extracts pubkeys from `p` tags, looks them up in the profiles map,
 * and returns a deduplicated list of display names. Returns `undefined`
 * when no names can be resolved (so the remark plugin falls back to
 * the generic `@\S+` pattern).
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
    if (tag[0] !== "p" || !tag[1]) {
      continue;
    }

    const profile = profiles[tag[1].toLowerCase()];
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
    if (tag[0] !== "p" || !tag[1]) {
      continue;
    }

    const pubkey = tag[1].toLowerCase();
    const displayName = profiles[pubkey]?.displayName?.trim();
    if (displayName) {
      pubkeysByName[displayName.toLowerCase()] = pubkey;
    }
  }

  return Object.keys(pubkeysByName).length > 0 ? pubkeysByName : undefined;
}
