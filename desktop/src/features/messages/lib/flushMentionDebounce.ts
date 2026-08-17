/**
 * Synchronously flush a pending mention debounce and resolve the correct
 * top-ranked suggestion. Used by handleMentionKeyDown to close the race
 * window where Tab/Enter fires before the debounce catches up to typed text.
 */
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
import {
  type MentionCandidateForRanking,
  rankMentionCandidates,
} from "./mentionRanking";
import {
  mapMentionCandidateToSuggestion,
  type MentionSuggestionCandidate,
} from "./mentionSuggestionMapping";

type MentionCandidateWithUI = MentionCandidateForRanking &
  MentionSuggestionCandidate;

export type FlushMentionDebounceResult =
  | { type: "match"; suggestion: MentionSuggestion; startIndex: number }
  | { type: "no-match" };

/**
 * Cancel the pending debounce timer, re-detect the prefix query from the
 * latest editor state, rank candidates, and return the top suggestion — or
 * null if no valid match is found.
 */
export function flushMentionDebounce<T extends MentionCandidateWithUI>(opts: {
  debounceTimerRef: React.MutableRefObject<ReturnType<
    typeof setTimeout
  > | null>;
  latestValueRef: React.RefObject<string>;
  latestCursorRef: React.RefObject<number>;
  searchableNamesLowerRef: React.RefObject<string[]>;
  candidates: readonly T[];
  activePersonaIds: ReadonlySet<string>;
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
}): FlushMentionDebounceResult | null {
  if (opts.debounceTimerRef.current !== null) {
    clearTimeout(opts.debounceTimerRef.current);
  }
  opts.debounceTimerRef.current = null;

  const mention = detectPrefixQuery(
    "@",
    opts.latestValueRef.current,
    opts.latestCursorRef.current,
    opts.searchableNamesLowerRef.current,
  );

  if (!mention || mention.query.length === 0) {
    return null;
  }

  const ranked = rankMentionCandidates(
    opts.candidates,
    mention.query,
    opts.activePersonaIds,
  );

  if (ranked.length === 0) {
    return { type: "no-match" };
  }

  const { candidate, label } = ranked[0];
  return {
    type: "match",
    suggestion: mapMentionCandidateToSuggestion({
      candidate,
      label,
      channelType: opts.channelType,
      currentPubkey: opts.currentPubkey,
      ownerProfiles: opts.ownerProfiles,
      profiles: opts.profiles,
    }),
    startIndex: mention.startIndex,
  };
}
