/**
 * Resolve the `p`-tag recipients an outgoing message body mentions.
 *
 * Two sources, in order: names the author picked from autocomplete (which
 * carry an exact pubkey), then channel members whose display name still
 * matches literally. Reserved tokens (`@channel`, `@here`) are excluded from
 * both — a channel-wide mention never expands into per-member pubkeys, and a
 * member who happens to be named "here" is not the target of `@here`.
 *
 * Extracted from `useMentions` so the precedence rules stay unit-testable.
 */

import { isReservedMentionName } from "./channelNotify";
import { hasMention } from "./hasMention";

export type MentionPubkeyCandidate = {
  displayName: string | null;
  isMember: boolean;
  pubkey?: string;
};

export function resolveMentionPubkeys(
  text: string,
  mentionMap: ReadonlyMap<string, string>,
  personaMentionNames: Iterable<string>,
  candidates: readonly MentionPubkeyCandidate[],
): string[] {
  const pubkeys: string[] = [];
  const selectedDisplayNames = new Set(
    [...mentionMap.keys(), ...personaMentionNames].map((name) =>
      name.trim().toLowerCase(),
    ),
  );

  for (const [displayName, pubkey] of mentionMap) {
    if (isReservedMentionName(displayName)) continue;
    if (hasMention(text, displayName)) {
      pubkeys.push(pubkey);
    }
  }

  for (const candidate of candidates) {
    if (!candidate.pubkey) continue;
    if (!candidate.isMember) continue;
    if (pubkeys.includes(candidate.pubkey)) continue;
    const name = candidate.displayName;
    if (!name || isReservedMentionName(name)) continue;
    if (selectedDisplayNames.has(name.trim().toLowerCase())) continue;
    if (hasMention(text, name)) {
      pubkeys.push(candidate.pubkey);
    }
  }

  return [...new Set(pubkeys)];
}
