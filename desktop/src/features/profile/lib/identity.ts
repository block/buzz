import type { Profile, UserProfileSummary } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

export type UserProfileLookup = Record<string, UserProfileSummary>;

export { truncatePubkey };

/**
 * Deep-equal two profile lookups by value. Used to stabilise the merged
 * `messageProfiles` reference at the ChannelScreen boundary: the underlying
 * `users-batch` query re-keys on the full sorted pubkey set, so typing churn
 * (a transient typing-only pubkey entering/leaving the set) produces a fresh
 * lookup object identity even when no profile value actually changed. That new
 * reference fails MessageRow's `prev.profiles === next.profiles` memo check and
 * re-renders the entire timeline on every keystroke-adjacent typing event.
 * Returning the previous reference when this reports equal keeps the memo
 * intact. Consumers read profiles by pubkey value only, never treating identity
 * as a change signal, so returning the stale-but-value-identical reference is
 * safe.
 */
export function profileLookupsEqual(
  a: UserProfileLookup,
  b: UserProfileLookup,
): boolean {
  if (a === b) {
    return true;
  }

  const aKeys = Object.keys(a);
  if (aKeys.length !== Object.keys(b).length) {
    return false;
  }

  for (const key of aKeys) {
    const prev = a[key];
    const next = b[key];
    if (
      next === undefined ||
      prev.displayName !== next.displayName ||
      prev.name !== next.name ||
      prev.avatarUrl !== next.avatarUrl ||
      prev.nip05Handle !== next.nip05Handle ||
      prev.ownerPubkey !== next.ownerPubkey ||
      prev.isAgent !== next.isAgent
    ) {
      return false;
    }
  }

  return true;
}

function getResolvedProfile(
  pubkey: string,
  profiles: UserProfileLookup | undefined,
) {
  if (!profiles) {
    return null;
  }

  return profiles[normalizePubkey(pubkey)] ?? null;
}

export function mergeCurrentProfileIntoLookup(
  profiles: UserProfileLookup | undefined,
  currentProfile:
    | Pick<Profile, "pubkey" | "displayName" | "avatarUrl" | "nip05Handle">
    | null
    | undefined,
) {
  if (!currentProfile) {
    return profiles;
  }

  return {
    ...(profiles ?? {}),
    [normalizePubkey(currentProfile.pubkey)]: {
      displayName: currentProfile.displayName,
      // `Profile` does not carry the kind-0 `name`; keep whatever the batch
      // lookup already resolved so mention aliases survive the merge.
      name: profiles?.[normalizePubkey(currentProfile.pubkey)]?.name ?? null,
      avatarUrl: currentProfile.avatarUrl,
      nip05Handle: currentProfile.nip05Handle,
      isAgent: profiles?.[normalizePubkey(currentProfile.pubkey)]?.isAgent,
      ownerPubkey:
        profiles?.[normalizePubkey(currentProfile.pubkey)]?.ownerPubkey ?? null,
    },
  };
}

export function resolveUserLabel(input: {
  pubkey: string;
  currentPubkey?: string;
  fallbackName?: string | null;
  profiles?: UserProfileLookup;
  preferResolvedSelfLabel?: boolean;
}) {
  const {
    currentPubkey,
    fallbackName,
    preferResolvedSelfLabel = false,
    profiles,
    pubkey,
  } = input;

  if (
    typeof currentPubkey === "string" &&
    normalizePubkey(currentPubkey) === normalizePubkey(pubkey)
  ) {
    if (!preferResolvedSelfLabel) {
      return "You";
    }
  }

  const profile = getResolvedProfile(pubkey, profiles);
  const displayName = profile?.displayName?.trim();
  if (displayName) {
    return displayName;
  }

  const nip05Handle = profile?.nip05Handle?.trim();
  if (nip05Handle) {
    return nip05Handle;
  }

  const safeFallback = fallbackName?.trim();
  if (safeFallback) {
    return safeFallback;
  }

  return truncatePubkey(pubkey);
}

/**
 * Returns true when the current user owns the agent that authored a message.
 * Mirrors the relay's `is_agent_owner` gate: ownership is determined by the
 * NIP-OA `ownerPubkey` field on the author's profile, NOT by the local
 * managed-agents list (which can diverge from server-side ownership).
 */
export function ownsAuthorAgent(
  profile: { ownerPubkey: string | null } | undefined,
  currentPubkey: string | undefined,
): boolean {
  return (
    !!currentPubkey &&
    !!profile?.ownerPubkey &&
    normalizePubkey(profile.ownerPubkey) === normalizePubkey(currentPubkey)
  );
}

export function resolveUserSecondaryLabel(input: {
  pubkey: string;
  profiles?: UserProfileLookup;
}) {
  const profile = getResolvedProfile(input.pubkey, input.profiles);
  const displayName = profile?.displayName?.trim();
  const nip05Handle = profile?.nip05Handle?.trim();

  if (displayName && nip05Handle) {
    return nip05Handle;
  }

  return null;
}

/**
 * Label for an agent's owner: "you" when the current user owns it, otherwise
 * the owner's display name, NIP-05 handle, or truncated pubkey.
 */
export function formatOwnerLabel(
  ownerPubkey: string | null | undefined,
  currentPubkey: string | null | undefined,
  ownerProfiles?: UserProfileLookup,
) {
  if (!ownerPubkey) {
    return null;
  }

  const normalizedOwnerPubkey = normalizePubkey(ownerPubkey);
  if (
    currentPubkey &&
    normalizedOwnerPubkey === normalizePubkey(currentPubkey)
  ) {
    return "you";
  }

  const owner = ownerProfiles?.[normalizedOwnerPubkey];
  return (
    owner?.displayName?.trim() ||
    owner?.nip05Handle?.trim() ||
    truncatePubkey(ownerPubkey)
  );
}

/**
 * Appends an agent's owner to a display name only when that name collides with
 * another visible result. The returned label is presentation-only; callers
 * keep the original display name for selection and mention insertion.
 */
export function formatDisambiguatedAgentName(input: {
  displayName: string;
  hasNameCollision: boolean;
  isAgent?: boolean;
  ownerLabel?: string | null;
}) {
  const { displayName, hasNameCollision, isAgent, ownerLabel } = input;
  const normalizedOwnerLabel = ownerLabel?.trim();

  if (!hasNameCollision || !isAgent || !normalizedOwnerLabel) {
    return displayName;
  }

  return `${displayName} (${normalizedOwnerLabel})`;
}

/** Case-insensitive display-name counts for a currently visible result set. */
export function countVisibleDisplayNames(
  displayNames: readonly (string | null | undefined)[],
) {
  const counts = new Map<string, number>();

  for (const displayName of displayNames) {
    const normalizedName = displayName?.trim().toLowerCase();
    if (!normalizedName) {
      continue;
    }
    counts.set(normalizedName, (counts.get(normalizedName) ?? 0) + 1);
  }

  return counts;
}

export function hasVisibleDisplayNameCollision(
  displayName: string,
  counts: ReadonlyMap<string, number>,
) {
  return (counts.get(displayName.trim().toLowerCase()) ?? 0) > 1;
}
