import type { RegisteredAgentReference } from "@/shared/api/tauriRegisteredAgents";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

type MinimalProfile = {
  displayName?: string | null;
  avatarUrl?: string | null;
};

type PubkeyRecord = { pubkey: string };

export function registeredAgentRoleSummary(roleSummary: string | null): string {
  const normalizedRole = roleSummary?.trim();
  return normalizedRole
    ? `${normalizedRole} · Externally managed`
    : "Externally managed";
}

export function registeredAgentAriaLabel(
  label: string,
  pubkey: string,
): string {
  return `${label} externally managed agent profile, public key ${truncatePubkey(
    normalizePubkey(pubkey),
  )}`;
}

export function resolveRegisteredAgentDisplay({
  reference,
  profile,
}: {
  reference: RegisteredAgentReference;
  profile: MinimalProfile | null | undefined;
}) {
  const profileLabel = profile?.displayName?.trim();
  const storedLabel = reference.label?.trim();
  return {
    label:
      profileLabel ||
      storedLabel ||
      truncatePubkey(normalizePubkey(reference.pubkey)),
    avatarUrl: profile?.avatarUrl?.trim() || null,
  };
}

export function dedupeRegisteredAgentsAgainstManaged<T extends PubkeyRecord>(
  references: readonly RegisteredAgentReference[],
  managed: readonly T[],
): RegisteredAgentReference[] {
  const managedPubkeys = new Set(
    managed.map((agent) => normalizePubkey(agent.pubkey)),
  );
  return references.filter(
    (reference) => !managedPubkeys.has(normalizePubkey(reference.pubkey)),
  );
}

export function visibleRegisteredAgentReferences<T extends PubkeyRecord>(
  references: readonly RegisteredAgentReference[],
  managed: readonly T[],
  error: Error | null,
): RegisteredAgentReference[] {
  if (error) return [];
  return dedupeRegisteredAgentsAgainstManaged(references, managed);
}
