import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { UserSearchResult } from "@/shared/api/types";

/** Presentation metadata remembered when a pubkey is picked from search. */
export type AllowlistMemberHint = {
  displayName: string | null;
  avatarUrl: string | null;
  nip05Handle: string | null;
  isAgent: boolean;
};

export function hintFromSearchResult(
  user: UserSearchResult,
): AllowlistMemberHint {
  return {
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    nip05Handle: user.nip05Handle,
    isAgent: user.isAgent,
  };
}

function lookupProfile(
  pubkey: string,
  profiles?: UserProfileLookup,
) {
  if (!profiles) {
    return undefined;
  }
  const normalized = pubkey.toLowerCase();
  return profiles[normalized] ?? profiles[pubkey];
}

/**
 * Human-readable chip label: display name, then NIP-05, then truncated pubkey.
 * Search hints win over batch profiles so a just-picked name is never stale.
 */
export function resolveAllowlistChipLabel(input: {
  pubkey: string;
  hint?: AllowlistMemberHint | null;
  profiles?: UserProfileLookup;
}): string {
  const { pubkey, hint, profiles } = input;
  const fromHint =
    hint?.displayName?.trim() || hint?.nip05Handle?.trim() || null;
  if (fromHint) {
    return fromHint;
  }

  return resolveUserLabel({ pubkey, profiles });
}

export function resolveAllowlistChipAvatar(input: {
  pubkey: string;
  hint?: AllowlistMemberHint | null;
  profiles?: UserProfileLookup;
}): string | null {
  const { pubkey, hint, profiles } = input;
  if (hint?.avatarUrl) {
    return hint.avatarUrl;
  }
  return lookupProfile(pubkey, profiles)?.avatarUrl ?? null;
}

export function resolveAllowlistChipIsAgent(input: {
  pubkey: string;
  hint?: AllowlistMemberHint | null;
  profiles?: UserProfileLookup;
}): boolean {
  const { pubkey, hint, profiles } = input;
  if (hint?.isAgent) {
    return true;
  }
  return lookupProfile(pubkey, profiles)?.isAgent === true;
}
