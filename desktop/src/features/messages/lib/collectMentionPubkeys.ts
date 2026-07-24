import { hasMention } from "@/features/messages/lib/hasMention";
import { extractNip27MentionPubkeys } from "@/features/messages/lib/nip27Mentions";
import { rewriteMentionsToNip27 } from "@/features/messages/lib/rewriteMentionsToNip27";
import { normalizePubkey } from "@/shared/lib/pubkey";

type MentionCandidateLike = {
  displayName?: string;
  isMember?: boolean;
  pubkey?: string;
};

/**
 * Resolve mention pubkeys from `@name` maps/candidates plus NIP-27
 * `nostr:npub1…` URIs already present in the text.
 */
export function collectMentionPubkeys(
  text: string,
  mentionMap: Iterable<readonly [string, string]>,
  selectedDisplayNames: Iterable<string>,
  mentionCandidates: readonly MentionCandidateLike[],
): string[] {
  const pubkeys: string[] = [];
  const selected = new Set(
    [...selectedDisplayNames].map((name) => name.trim().toLowerCase()),
  );

  for (const [displayName, pubkey] of mentionMap) {
    if (hasMention(text, displayName)) {
      pubkeys.push(pubkey);
    }
  }

  for (const candidate of mentionCandidates) {
    if (!candidate.pubkey || !candidate.isMember) {
      continue;
    }
    if (pubkeys.includes(candidate.pubkey)) {
      continue;
    }
    const name = candidate.displayName;
    if (name && selected.has(name.trim().toLowerCase())) {
      continue;
    }
    if (name && hasMention(text, name)) {
      pubkeys.push(candidate.pubkey);
    }
  }

  for (const pubkey of extractNip27MentionPubkeys(text)) {
    if (
      !pubkeys.some(
        (existing) => normalizePubkey(existing) === normalizePubkey(pubkey),
      )
    ) {
      pubkeys.push(pubkey);
    }
  }

  return [...new Set(pubkeys)];
}

/** Rewrite resolved `@name` mentions in `text` to NIP-27 wire URIs. */
export function buildNip27WireBody(
  text: string,
  mentionPubkeys: readonly string[],
  getDisplayName: (pubkey: string) => string | null,
): string {
  const pairs: Array<readonly [string, string]> = [];
  for (const pubkey of mentionPubkeys) {
    const displayName = getDisplayName(pubkey);
    if (displayName) {
      pairs.push([displayName, pubkey]);
    }
  }
  return rewriteMentionsToNip27(text, pairs);
}
