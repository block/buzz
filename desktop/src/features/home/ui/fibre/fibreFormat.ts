import type { Fibre, FibreArtifact, FibrePerson } from "@/features/triage/api";

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

export function fibrePeopleLabel(people: readonly FibrePerson[]) {
  return people.map((person) => person.label).join(", ");
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
