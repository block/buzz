import { safeNpub } from "@/shared/lib/nostrUtils";

export type HostedNostrIdentity = {
  npub?: string;
  pubkey_hex?: string;
};

export type HostedIdentityComparison = {
  boundNpub: string | null;
  localNpub: string | null;
  identityMismatch: boolean;
};

/**
 * Compare the Builderlab-bound identity with this device's signing identity.
 *
 * Builderlab's canonical `npub` is authoritative when present. Older responses
 * can fall back to `pubkey_hex`, but a present identity that cannot be
 * canonicalized must fail closed so create/connect actions never run against an
 * unknown account binding.
 */
export function compareHostedCommunityIdentity(
  identity: HostedNostrIdentity | null | undefined,
  localPubkey: string | null | undefined,
): HostedIdentityComparison {
  const boundNpub = identity
    ? safeNpub(identity.npub ?? identity.pubkey_hex ?? "")
    : null;
  const localNpub = localPubkey ? safeNpub(localPubkey) : null;

  return {
    boundNpub,
    localNpub,
    identityMismatch: Boolean(
      identity && (!boundNpub || !localNpub || boundNpub !== localNpub),
    ),
  };
}
