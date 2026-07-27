import type { UserProfileSummary } from "@/shared/api/types";
import { NOTIFY_TAG, isNotifyMode } from "@/shared/constants/notify";

export const MENTION_REFERENCE_TAG = "mention";

/** Notify mode carried by a `["notify", mode]` tag, or null for other tags. */
function getNotifyTagMode(tag: string[]): string | null {
  const mode = tag[0] === NOTIFY_TAG ? tag[1] : undefined;
  return mode !== undefined && isNotifyMode(mode) ? mode : null;
}

export function getMentionTagPubkey(tag: string[]): string | null {
  if ((tag[0] !== "p" && tag[0] !== MENTION_REFERENCE_TAG) || !tag[1]) {
    return null;
  }

  return tag[1].toLowerCase();
}

/**
 * All names a profile can be @mentioned by. Message text is matched against
 * the sender's view of the profile at send time (agents and the CLI resolve
 * mentions against `display_name` *or* `name`, and renames happen after the
 * fact), so a single-alias match leaves chips that render but never resolve
 * to a pubkey. Emitting every known alias — display name, kind-0 `name`, and
 * the NIP-05 local part — keeps rendered chips and pubkey resolution in sync.
 */
function collectProfileAliases(
  profile: UserProfileSummary | undefined,
): string[] {
  if (!profile) {
    return [];
  }

  const aliases: string[] = [];
  const displayName = profile.displayName?.trim();
  if (displayName) {
    aliases.push(displayName);
  }

  const name = profile.name?.trim();
  if (name) {
    aliases.push(name);
  }

  // "_" is the NIP-05 root identifier, not a mentionable handle.
  const nip05Local = profile.nip05Handle?.trim().split("@")[0]?.trim();
  if (nip05Local && nip05Local !== "_") {
    aliases.push(nip05Local);
  }

  // `@channel`/`@here` are reserved (NIP-CM): they must never resolve to an
  // identity, so a member whose display name collides with one loses that
  // alias rather than hijacking the channel-wide token.
  return aliases.filter((alias) => !isNotifyMode(alias.toLowerCase()));
}

export type ResolvedMentionProps = {
  mentionNames: string[] | undefined;
  mentionPubkeysByName: Record<string, string> | undefined;
};

/**
 * Resolves mention render names and the name→pubkey map for mentioned users
 * from message `p` tags and non-notifying `mention` reference tags, in one
 * pass over the tags.
 *
 * `p` tags drive notification/search semantics. `mention` tags only preserve
 * render metadata for reference-only mentions.
 *
 * A `["notify", "channel" | "here"]` tag (NIP-CM) contributes its mode as a
 * render name with no pubkey, so `@channel`/`@here` chip only on events that
 * actually carry the marker and never open a profile.
 *
 * Both outputs come from the same alias set, so any `@name` chip the markdown
 * renderer matches is guaranteed to resolve to a pubkey.
 */
export function resolveMentionProps(
  tags: string[][] | undefined,
  profiles: Record<string, UserProfileSummary> | undefined,
): ResolvedMentionProps {
  if (!tags) {
    return { mentionNames: undefined, mentionPubkeysByName: undefined };
  }

  const names = new Set<string>();
  const pubkeysByName: Record<string, string> = {};

  for (const tag of tags) {
    const notifyMode = getNotifyTagMode(tag);
    if (notifyMode) {
      names.add(notifyMode);
      continue;
    }

    const pubkey = profiles ? getMentionTagPubkey(tag) : null;
    if (!pubkey) {
      continue;
    }

    for (const alias of collectProfileAliases(profiles?.[pubkey])) {
      names.add(alias);
      pubkeysByName[alias.toLowerCase()] = pubkey;
    }
  }

  return {
    mentionNames: names.size > 0 ? [...names] : undefined,
    mentionPubkeysByName:
      Object.keys(pubkeysByName).length > 0 ? pubkeysByName : undefined,
  };
}

export function resolveMentionNames(
  tags: string[][] | undefined,
  profiles: Record<string, UserProfileSummary> | undefined,
): string[] | undefined {
  return resolveMentionProps(tags, profiles).mentionNames;
}

export function resolveMentionPubkeysByName(
  tags: string[][] | undefined,
  profiles: Record<string, UserProfileSummary> | undefined,
): Record<string, string> | undefined {
  return resolveMentionProps(tags, profiles).mentionPubkeysByName;
}
