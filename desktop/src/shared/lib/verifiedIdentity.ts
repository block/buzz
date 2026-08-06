export type VerifiedIdentityFields = {
  verifiedName?: string | null;
  verifiedNameExpiresAt?: number | null;
};

function verifiedIdentityExpiryMs(
  expiresAt: number | null | undefined,
): number | null {
  if (!Number.isSafeInteger(expiresAt) || (expiresAt ?? 0) <= 0) {
    return null;
  }

  const expiresAtMs = (expiresAt as number) * 1_000;
  return Number.isSafeInteger(expiresAtMs) ? expiresAtMs : null;
}

/**
 * Return a verified name only while its relay assertion is still valid.
 * Missing or malformed expirations fail closed so old cached responses cannot
 * keep a trust label alive while the relay is unreachable.
 */
export function getCurrentVerifiedName(
  verifiedName: string | null | undefined,
  expiresAt: number | null | undefined,
  nowMs = Date.now(),
): string | null {
  const name = verifiedName?.trim();
  const expiresAtMs = verifiedIdentityExpiryMs(expiresAt);
  if (!name || expiresAtMs === null || expiresAtMs <= nowMs) {
    return null;
  }

  return name;
}

export function millisecondsUntilVerifiedIdentityExpiry(
  expiresAt: number | null | undefined,
  nowMs = Date.now(),
): number | null {
  const expiresAtMs = verifiedIdentityExpiryMs(expiresAt);
  return expiresAtMs === null ? null : Math.max(0, expiresAtMs - nowMs);
}

/** Preserve object identity until the verified-name view actually changes. */
export function withCurrentVerifiedIdentity<T extends VerifiedIdentityFields>(
  identity: T,
  nowMs = Date.now(),
): T {
  const verifiedName = getCurrentVerifiedName(
    identity.verifiedName,
    identity.verifiedNameExpiresAt,
    nowMs,
  );
  return identity.verifiedName === verifiedName
    ? identity
    : { ...identity, verifiedName };
}
