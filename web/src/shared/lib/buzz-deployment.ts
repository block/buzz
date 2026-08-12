const DEFAULT_DEEP_LINK_SCHEME = "buzz";
const DEFAULT_RELEASE_REPOSITORY = "block/buzz";

function validDeepLinkScheme(value: string | undefined): string {
  const candidate = value?.trim().toLowerCase();
  return candidate && /^[a-z][a-z0-9+.-]*$/.test(candidate)
    ? candidate
    : DEFAULT_DEEP_LINK_SCHEME;
}

function validReleaseRepository(value: string | undefined): string {
  const candidate = value?.trim();
  return candidate && /^[a-z0-9_.-]+\/[a-z0-9_.-]+$/i.test(candidate)
    ? candidate
    : DEFAULT_RELEASE_REPOSITORY;
}

export const BUZZ_DEEP_LINK_SCHEME = validDeepLinkScheme(
  import.meta.env.VITE_BUZZ_DEEP_LINK_SCHEME,
);

export const BUZZ_RELEASE_REPOSITORY = validReleaseRepository(
  import.meta.env.VITE_BUZZ_RELEASE_REPOSITORY,
);

export function buildBuzzJoinDeepLink({
  relay,
  code,
  policyReceipt,
}: {
  relay: string;
  code: string;
  policyReceipt?: string;
}): string {
  const query = new URLSearchParams({ relay, code });
  if (policyReceipt) query.set("policy_receipt", policyReceipt);
  return `${BUZZ_DEEP_LINK_SCHEME}://join?${query.toString()}`;
}
