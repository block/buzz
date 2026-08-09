import type {
  PresenceLookup,
  PresenceStatus,
  RelayAgent,
} from "@/shared/api/types";
import { isSafeDisplayText } from "@/shared/lib/safeDisplayText";

export type SharedFleetRow = {
  pubkey: string;
  name: string;
  modelLabel?: string;
  status: PresenceStatus;
  channels: string[];
  /** Informational only; mentioning remains channel-scoped elsewhere. */
  mentionHint: string;
};

function safeDisplayLabel(value: string, maximum = 256): string | null {
  const trimmed = value.trim();
  if (!trimmed || !isSafeDisplayText(trimmed, maximum)) {
    return null;
  }
  return trimmed;
}

function uniqueChannelNames(channels: readonly string[]): string[] {
  const names = new Set<string>();
  for (const channel of channels) {
    const normalized = safeDisplayLabel(channel);
    if (normalized) names.add(normalized);
  }
  return [...names].sort((left, right) => left.localeCompare(right));
}

function mentionHint(channelCount: number): string {
  if (channelCount === 0) return "No assigned channels";
  return `Mention only in its ${channelCount} assigned channel${
    channelCount === 1 ? "" : "s"
  }`;
}

/** Build a presentation-only projection. It has no remote lifecycle actions. */
export function buildSharedFleetRows(
  agents: readonly RelayAgent[],
  presence: PresenceLookup | undefined,
): SharedFleetRow[] {
  return agents
    .map((agent) => {
      // Directory status is durable metadata, not current liveness. Never use
      // it as a fallback when the separate presence read has no entry.
      const status = presence?.[agent.pubkey.toLowerCase()];
      if (status === undefined) return null;
      const channels = uniqueChannelNames(agent.channels);
      const name = safeDisplayLabel(agent.name) ?? "Unnamed remote worker";
      const modelLabel = agent.model ? safeDisplayLabel(agent.model) : null;
      return {
        pubkey: agent.pubkey,
        name,
        status,
        channels,
        mentionHint: mentionHint(channels.length),
        ...(modelLabel ? { modelLabel } : {}),
      };
    })
    .filter((row): row is SharedFleetRow => row !== null)
    .sort((left, right) => left.name.localeCompare(right.name));
}

/** Only a recognized current presence may render as a live remote worker. */
export function activeSharedFleetRows(
  rows: readonly SharedFleetRow[],
): SharedFleetRow[] {
  return rows.filter((row) => row.status === "online" || row.status === "away");
}
