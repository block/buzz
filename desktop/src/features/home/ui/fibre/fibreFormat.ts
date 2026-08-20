import type { Fibre, FibreArtifact, FibrePerson } from "@/features/triage/api";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { truncatePubkey } from "@/shared/lib/pubkey";

export function formatFibreAge(createdAt: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000) - createdAt);
  if (seconds < 60) return `${Math.max(1, seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

export function fibreSourceLabel(fibre: Pick<Fibre, "channelName" | "isDm">) {
  if (fibre.isDm) {
    return fibre.channelName ? `DM · ${fibre.channelName}` : "DM";
  }
  return fibre.channelName ? `#${fibre.channelName}` : "Unknown channel";
}

/** Drop truncated/raw pubkeys so profile lookup can replace them. */
export function usefulStoredPersonLabel(
  label: string | null | undefined,
  pubkey: string | null | undefined,
): string | null {
  const trimmed = label?.trim();
  if (!trimmed) return null;
  if (pubkey) {
    const normalized = pubkey.trim().toLowerCase();
    if (trimmed.toLowerCase() === normalized) return null;
    if (trimmed === truncatePubkey(pubkey)) return null;
  }
  if (/^[0-9a-f]{6,}[.…].+$/i.test(trimmed)) return null;
  if (/^[0-9a-f]{16,}$/i.test(trimmed)) return null;
  return trimmed;
}

export function collectFibrePubkeys(fibres: readonly Fibre[]): string[] {
  const pubkeys = new Set<string>();
  for (const fibre of fibres) {
    for (const person of fibre.people) {
      if (person.pubkey) pubkeys.add(person.pubkey);
    }
    for (const artifact of fibre.artifacts) {
      if (artifact.authorPubkey) pubkeys.add(artifact.authorPubkey);
    }
  }
  return [...pubkeys];
}

export function resolveFibrePersonLabel(
  person: { pubkey: string | null | undefined; label?: string | null },
  input: { profiles?: UserProfileLookup; currentPubkey?: string },
): string {
  const pubkey = person.pubkey?.trim();
  if (!pubkey) {
    return usefulStoredPersonLabel(person.label, null) ?? "Unknown";
  }
  return resolveUserLabel({
    pubkey,
    currentPubkey: input.currentPubkey,
    profiles: input.profiles,
    preferResolvedSelfLabel: true,
    fallbackName: usefulStoredPersonLabel(person.label, pubkey),
  });
}

export function fibrePeopleLabel(
  people: readonly FibrePerson[],
  input?: { profiles?: UserProfileLookup; currentPubkey?: string },
) {
  return people
    .map((person) => resolveFibrePersonLabel(person, input ?? {}))
    .join(", ");
}

export function fibreArtifactCountLabel(count: number) {
  return count === 1 ? "1 message" : `${count} messages`;
}

export function latestArtifact(
  artefacts: readonly FibreArtifact[],
): FibreArtifact | null {
  if (artefacts.length === 0) return null;
  return [...artefacts].sort(
    (left, right) => (right.createdAt ?? 0) - (left.createdAt ?? 0),
  )[0];
}

export function primaryThreadTarget(fibre: Fibre): {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
} | null {
  const artifact = latestArtifact(fibre.artifacts);
  const channelId = artifact?.channelId ?? fibre.channelId;
  const messageId = artifact?.eventId;
  if (!channelId || !messageId) return null;
  return {
    channelId,
    messageId,
    threadRootId: artifact?.threadRootId ?? null,
  };
}
