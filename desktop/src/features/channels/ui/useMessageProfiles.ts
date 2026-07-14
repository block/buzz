import * as React from "react";

import {
  mergeCurrentProfileIntoLookup,
  profileLookupsEqual,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  ChannelMember,
  ManagedAgent,
  Profile,
  RelayAgent,
} from "@/shared/api/types";
import {
  mergeAgentNamesIntoProfiles,
  mergeMemberAgentFlagsIntoProfiles,
} from "./useChannelActivityTyping";

/**
 * The channel screen's message-row profile lookup: the `users-batch` query
 * profiles overlaid with the current profile, managed/relay agent names, and
 * channel-member agent flags.
 *
 * Member agent flags (`role === "bot"` or `isAgent`) ride along in the lookup
 * so a member-only bot — known through channel membership alone, with no
 * profile `isAgent` flag and no managed/relay/feed presence — still passes
 * MessageRow's per-pubkey `profiles[pk]?.isAgent` check; rows no longer see
 * a member-derived agent set.
 *
 * The returned reference is stabilised across renders when no profile value
 * changed: the raw merge gets a fresh identity whenever the `users-batch`
 * query re-keys — which typing churn triggers constantly — and that identity
 * flows to MessageRow's `prev.profiles === next.profiles` memo check, so an
 * unstable reference re-renders the whole timeline per keystroke. Consumers
 * read profiles by pubkey value only, never treating identity as a change
 * signal, so returning the stale-but-value-identical reference is safe.
 */
export function useMessageProfiles({
  channelMembers,
  currentProfile,
  currentPubkey,
  managedAgents,
  profiles,
  relayAgents,
}: {
  channelMembers: ChannelMember[] | undefined;
  currentProfile: Profile | undefined;
  currentPubkey: string | undefined;
  managedAgents: ManagedAgent[];
  profiles: UserProfileLookup | undefined;
  relayAgents: RelayAgent[];
}): UserProfileLookup {
  const raw = React.useMemo(() => {
    const base = mergeCurrentProfileIntoLookup(profiles, currentProfile) ?? {};
    return mergeMemberAgentFlagsIntoProfiles(
      mergeAgentNamesIntoProfiles(
        base,
        managedAgents,
        relayAgents,
        currentPubkey,
      ),
      channelMembers,
    );
  }, [
    channelMembers,
    currentProfile,
    currentPubkey,
    managedAgents,
    profiles,
    relayAgents,
  ]);

  const ref = React.useRef(raw);
  if (!profileLookupsEqual(ref.current, raw)) {
    ref.current = raw;
  }
  return ref.current;
}
