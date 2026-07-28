import { hasMention } from "@/features/messages/lib/hasMention";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ForwardNoteMentions = {
  /** Normalized pubkeys to send as the note's recipients. */
  pubkeys: string[];
  /** Matched display names, for rendering mention chips in the preview. */
  names: string[];
  /** Lowercased name → pubkey, mirroring `resolveMentionProps`'s output. */
  pubkeysByName: Record<string, string>;
};

const NO_MENTIONS: ForwardNoteMentions = {
  pubkeys: [],
  names: [],
  pubkeysByName: {},
};

/**
 * Whether the note plausibly `@mentions` someone.
 *
 * Deliberately cheap and over-eager (an email address counts): its only job is
 * to decide whether sending must wait for the destination's member list, and a
 * false positive costs a few milliseconds while that query settles, whereas a
 * false negative publishes a mention with no `p` tag.
 *
 * Any non-space character counts, not `\w` — that character class is ASCII-only
 * in JavaScript, so display names like `李`, `Élodie`, or an emoji-led agent
 * name are mentions `resolveForwardNoteMentions` matches but a `\w` preflight
 * would wave through unresolved.
 */
export function noteMayMention(note: string): boolean {
  return /@\S/.test(note);
}

/**
 * Resolve `@mentions` written in a forward note against the DESTINATION
 * channel's members.
 *
 * The forward dialog's note is a plain textarea, so there is no autocomplete
 * registry of picked mentions to read back the way the composer has. Matching
 * every destination member's display name against the note reproduces the
 * registry-free half of `useMentions().extractMentionPubkeys` — same
 * `hasMention` matcher (code spans masked, markdown emphasis tolerated), same
 * member-only scope — so a note mention notifies exactly the people the
 * composer would have notified.
 *
 * `pubkeys` still goes through `messageMentionPubkeys` on the send path, which
 * drops the sender and adds DM recipients.
 */
export function resolveForwardNoteMentions(
  note: string,
  members: readonly ChannelMember[] | undefined,
): ForwardNoteMentions {
  const trimmed = note.trim();
  if (trimmed.length === 0 || !members || members.length === 0) {
    return NO_MENTIONS;
  }

  const pubkeys = new Set<string>();
  const names = new Set<string>();
  const pubkeysByName: Record<string, string> = {};

  for (const member of members) {
    const displayName = member.displayName?.trim();
    if (!displayName) continue;
    const pubkey = normalizePubkey(member.pubkey);
    if (pubkey.length === 0 || !hasMention(trimmed, displayName)) continue;

    pubkeys.add(pubkey);
    names.add(displayName);
    pubkeysByName[displayName.toLowerCase()] = pubkey;
  }

  return { pubkeys: [...pubkeys], names: [...names], pubkeysByName };
}
