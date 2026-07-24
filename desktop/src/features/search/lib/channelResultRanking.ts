import {
  scoreChannelMatch,
  scoreChannelName,
} from "@/features/channels/lib/channelSearchScore";
import type { Channel } from "@/shared/api/types";

/** Topbar search shows a short typeahead list; narrowing is done by typing. */
export const TOPBAR_CHANNEL_RESULT_LIMIT = 5;

/**
 * Rank channels for the topbar (⌘K) typeahead using the same fuzzy scorer as
 * the channel browser, so `buzz security` / `buzzsec` find `buzz-security`
 * from either surface. Relevance-ordered (not alphabetical) — with a hard cap
 * on results, alphabetical order buries good matches behind early-alphabet
 * ones (e.g. `buzz` could never surface `buzz-security` behind 48 other
 * `buzz-*` channels).
 *
 * Visibility rules match the previous behavior: archived channels only for
 * members; otherwise open channels or memberships.
 */
export function rankTopbarChannelResults({
  channels,
  channelLabels,
  lowerQuery,
  limit = TOPBAR_CHANNEL_RESULT_LIMIT,
}: {
  channels: Channel[];
  channelLabels?: Record<string, string>;
  lowerQuery: string;
  limit?: number;
}): Channel[] {
  const scored: { channel: Channel; displayName: string; score: number }[] = [];

  for (const channel of channels) {
    const visible = channel.archivedAt
      ? channel.isMember
      : channel.visibility === "open" || channel.isMember;
    if (!visible) continue;

    const displayName = channelLabels?.[channel.id]?.trim() || channel.name;

    // Score against the display name (label override wins), with description
    // as the lowest band. When a label overrides the name, the raw name must
    // stay searchable too — take the best of the two.
    let score = scoreChannelMatch(
      { name: displayName, description: channel.description },
      lowerQuery,
    );
    if (displayName !== channel.name) {
      const rawNameScore = scoreChannelName(channel.name, lowerQuery);
      if (rawNameScore !== null && (score === null || rawNameScore < score)) {
        score = rawNameScore;
      }
    }
    if (score === null) continue;

    scored.push({ channel, displayName, score });
  }

  return scored
    .sort(
      (a, b) => a.score - b.score || a.displayName.localeCompare(b.displayName),
    )
    .slice(0, limit)
    .map((entry) => entry.channel);
}
