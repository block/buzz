import {
  type ChannelSearchable,
  scoreChannelMatch,
} from "./channelSearchScore";

/** The subset of `Channel` the picker's ordering logic reads. */
export type PickerChannel = ChannelSearchable & {
  isMember: boolean;
};

/**
 * Default ordering for the channel picker: channels the user has joined
 * first, then alphabetical — so deploy dialogs lead with the channels a
 * user is most likely to target.
 */
export function sortChannelsMembersFirst<T extends PickerChannel>(
  channels: T[],
): T[] {
  return [...channels].sort(
    (a, b) =>
      Number(b.isMember) - Number(a.isMember) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
  );
}

/**
 * Channels matching `query`, best match first. Uses the channel browser's
 * fuzzy scorer; ties keep the incoming (members-first) order.
 */
export function filterChannelsByQuery<T extends PickerChannel>(
  channels: T[],
  query: string,
): T[] {
  const lowerQuery = query.trim().toLowerCase();
  if (lowerQuery.length === 0) {
    return channels;
  }

  const scored: { channel: T; score: number }[] = [];
  for (const channel of channels) {
    const score = scoreChannelMatch(channel, lowerQuery);
    if (score !== null) {
      scored.push({ channel, score });
    }
  }
  scored.sort((a, b) => a.score - b.score);
  return scored.map((entry) => entry.channel);
}
