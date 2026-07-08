import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { scopeByChannel } from "@/features/agents/ui/agentSessionPanelLayout";
import { isMeaningfulItem } from "@/features/agents/ui/agentSessionTranscriptPresentation";

/** Identity shown on merged feed rows. */
export type LiveActivityAgent = {
  pubkey: string;
  name: string;
};

export type LiveActivityEntry = {
  agent: LiveActivityAgent;
  item: TranscriptItem;
  /** Unique across agents — transcript item ids are only per-agent unique. */
  key: string;
};

/** Consecutive run of entries from the same agent in the merged stream. */
export type LiveActivityGroup = {
  agent: LiveActivityAgent;
  entries: LiveActivityEntry[];
  key: string;
};

/**
 * Cap on merged entries kept for the preview window. The composer popover is
 * a peek, not the archive — click-through opens the full activity view.
 */
const MERGED_ENTRY_LIMIT = 80;

function parseTimestamp(timestamp: string): number {
  const parsed = Date.parse(timestamp);
  return Number.isNaN(parsed) ? 0 : parsed;
}

/**
 * Merge several per-agent transcripts into one interleaved, chronological
 * stream scoped to a channel. Inputs are index-aligned: `transcripts[i]`
 * belongs to `agents[i]`. Each per-agent transcript is already chronological,
 * so this is a stable k-way merge on item timestamps (ties keep agent order).
 */
export function mergeLiveActivity(
  agents: readonly LiveActivityAgent[],
  transcripts: readonly TranscriptItem[][],
  channelId: string | null,
): LiveActivityEntry[] {
  const cursors = agents.map((agent, index) => ({
    agent,
    items: scopeByChannel(transcripts[index] ?? [], channelId).filter(
      isMeaningfulItem,
    ),
    position: 0,
  }));

  const merged: LiveActivityEntry[] = [];
  for (;;) {
    let best: (typeof cursors)[number] | null = null;
    let bestTime = Number.POSITIVE_INFINITY;
    for (const cursor of cursors) {
      const item = cursor.items[cursor.position];
      if (!item) {
        continue;
      }
      const time = parseTimestamp(item.timestamp);
      if (time < bestTime) {
        best = cursor;
        bestTime = time;
      }
    }
    if (!best) {
      break;
    }
    const item = best.items[best.position];
    if (item) {
      merged.push({
        agent: best.agent,
        item,
        key: `${best.agent.pubkey}:${item.id}`,
      });
    }
    best.position += 1;
  }

  return merged.length > MERGED_ENTRY_LIMIT
    ? merged.slice(merged.length - MERGED_ENTRY_LIMIT)
    : merged;
}

/**
 * Group consecutive same-agent entries so the feed can render one agent
 * badge per run instead of per row.
 */
export function groupLiveActivity(
  entries: readonly LiveActivityEntry[],
): LiveActivityGroup[] {
  const groups: LiveActivityGroup[] = [];
  for (const entry of entries) {
    const current = groups[groups.length - 1];
    if (current && current.agent.pubkey === entry.agent.pubkey) {
      current.entries.push(entry);
      continue;
    }
    groups.push({
      agent: entry.agent,
      entries: [entry],
      key: entry.key,
    });
  }
  return groups;
}
