import React from "react";

/**
 * The deduped set of pubkeys a channel surface needs profiles for: message
 * authors, active-DM participants, known agents, currently-typing users, and
 * people bound to imported history (so imported rows can show their avatar
 * even where they never natively authored an event). Extracted from
 * ChannelScreen to keep that file within its size budget.
 */
export function useMessageProfilePubkeys(input: {
  messageEventProfilePubkeys: Iterable<string>;
  activeDmParticipantPubkeys: Iterable<string>;
  knownAgentPubkeys: Iterable<string>;
  typingEntries: Array<{ pubkey: string }>;
  boundImportPubkeys: Iterable<string>;
}): string[] {
  const {
    messageEventProfilePubkeys,
    activeDmParticipantPubkeys,
    knownAgentPubkeys,
    typingEntries,
    boundImportPubkeys,
  } = input;
  return React.useMemo(
    () => [
      ...new Set([
        ...messageEventProfilePubkeys,
        ...activeDmParticipantPubkeys,
        ...knownAgentPubkeys,
        ...typingEntries.map((entry) => entry.pubkey),
        ...boundImportPubkeys,
      ]),
    ],
    [
      activeDmParticipantPubkeys,
      knownAgentPubkeys,
      messageEventProfilePubkeys,
      typingEntries,
      boundImportPubkeys,
    ],
  );
}
