import * as React from "react";

import { usePresenceQuery } from "@/features/presence/hooks";
import {
  buildChannelMentionTags,
  CHANNEL_MENTION_SUGGESTIONS,
  channelMentionModes,
  getChannelMentionAudienceLimitError,
} from "./channelMentions";
import type { MentionSuggestion } from "../ui/MentionAutocomplete";

export function useChannelMentions(
  memberPubkeys: ReadonlySet<string>,
  membersResolved: boolean,
  currentPubkey: string | null,
) {
  const memberPubkeyList = React.useMemo(
    () => [...memberPubkeys],
    [memberPubkeys],
  );
  const presenceQuery = usePresenceQuery(memberPubkeyList, {
    enabled: membersResolved && memberPubkeyList.length > 0,
  });

  const suggestionsForQuery = React.useCallback(
    (query: string): MentionSuggestion[] => {
      const normalizedQuery = query.trim().toLowerCase();
      return CHANNEL_MENTION_SUGGESTIONS.filter(({ displayName }) =>
        displayName.startsWith(normalizedQuery),
      ).map(({ annotation, displayName }) => ({
        annotation,
        audience: displayName,
        displayName,
        kind: "audience",
      }));
    },
    [],
  );

  const extractChannelMentionTags = React.useCallback(
    (text: string, originalText?: string): string[][] =>
      buildChannelMentionTags({
        memberPubkeys,
        originalText,
        presence: presenceQuery.data,
        selfPubkey: currentPubkey ?? "",
        text,
      }),
    [currentPubkey, memberPubkeys, presenceQuery.data],
  );

  const getChannelMentionError = React.useCallback(
    (text: string, originalText?: string): string | null => {
      const modes = channelMentionModes(text);
      if (modes.length === 0) return null;
      const originalModes = new Set(channelMentionModes(originalText ?? ""));
      const notifyModes = modes.includes("everyone")
        ? ["everyone" as const]
        : modes;
      const newNotifyModes = notifyModes.filter(
        (mode) => !originalModes.has(mode),
      );
      if (newNotifyModes.length === 0) return null;
      if (!membersResolved || !currentPubkey) {
        return "Checking channel members. Try again in a moment.";
      }
      if (newNotifyModes.includes("here") && presenceQuery.data === undefined) {
        return "Checking who is online. Try again in a moment.";
      }
      const self = currentPubkey.toLowerCase();
      const recipientCount = newNotifyModes.includes("everyone")
        ? memberPubkeyList.filter((pubkey) => pubkey !== self).length
        : memberPubkeyList.filter(
            (pubkey) =>
              pubkey !== self && presenceQuery.data?.[pubkey] === "online",
          ).length;
      return getChannelMentionAudienceLimitError(recipientCount);
    },
    [currentPubkey, memberPubkeyList, membersResolved, presenceQuery.data],
  );

  const names = React.useMemo(
    () => CHANNEL_MENTION_SUGGESTIONS.map(({ displayName }) => displayName),
    [],
  );

  return {
    extractChannelMentionTags,
    getChannelMentionError,
    names,
    suggestionsForQuery,
  };
}
