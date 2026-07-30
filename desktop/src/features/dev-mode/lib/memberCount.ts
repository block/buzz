/**
 * The relay's kind:39002 member snapshot carries at most 1000 p-tags
 * (`LIMIT 1000` in buzz-db's channel::get_members), so every count the
 * client derives from it saturates there. A count at the cap means
 * "1000 or more", not an exact size — render it as a lower bound.
 * Client-only mitigation: the cap itself lives in the production relay.
 */
export const CHANNEL_MEMBER_SNAPSHOT_CAP = 1000;

export function formatMemberCount(count: number): string {
  return count >= CHANNEL_MEMBER_SNAPSHOT_CAP
    ? `${CHANNEL_MEMBER_SNAPSHOT_CAP}+`
    : `${count}`;
}

export function memberCountLabel(count: number): string {
  return `${formatMemberCount(count)} ${count === 1 ? "member" : "members"}`;
}
