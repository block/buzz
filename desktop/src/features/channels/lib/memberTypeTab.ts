import type { ChannelMember, UserSearchResult } from "@/shared/api/types";

export type MemberTypeTab = "all" | "people" | "agents";

export function filterMembersForTypeTab<T extends ChannelMember>(
  members: readonly T[],
  tab: MemberTypeTab,
  isBot: (member: T) => boolean,
): T[] {
  if (tab === "people") {
    return members.filter((member) => !isBot(member));
  }
  if (tab === "agents") {
    return members.filter((member) => isBot(member));
  }
  return [...members];
}

export function filterAddSearchResultsForTypeTab<T extends UserSearchResult>(
  results: readonly T[],
  tab: MemberTypeTab,
): T[] {
  if (tab === "people") {
    return results.filter((result) => !result.isAgent);
  }
  if (tab === "agents") {
    return results.filter((result) => result.isAgent);
  }
  return [...results];
}
