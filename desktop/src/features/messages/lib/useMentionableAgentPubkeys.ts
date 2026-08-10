import * as React from "react";
import {
  getChannelOwnedAgentPubkeys,
  getMentionableAgentPubkeys,
  isAgentMentionChannelType,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  ChannelMember,
  ChannelType,
  RelayAgent,
} from "@/shared/api/types";

const EMPTY_AGENT_PUBKEYS: ReadonlySet<string> = new Set();

/**
 * Builds the composer allow-list without turning remote agent identities into
 * local managed instances. For non-managed identities, `profiles.ownerPubkey`
 * must be the NIP-OA-verified value returned by the profile query pipeline.
 * The relay directory must also have loaded successfully so an explicit
 * heartbeat-only (`nobody`) policy cannot be missed during startup or errors.
 */
export function useMentionableAgentPubkeys({
  channelId,
  channelMembers,
  channelType,
  currentPubkey,
  managedAgentPubkeys,
  profiles,
  relayAgents,
  sharedChannelIds,
}: {
  channelId: string | null;
  channelMembers: readonly ChannelMember[] | undefined;
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  managedAgentPubkeys: ReadonlySet<string>;
  profiles: UserProfileLookup | undefined;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}) {
  const isChannelScope = Boolean(
    channelId && isAgentMentionChannelType(channelType),
  );
  const channelOwnedAgentPubkeys = React.useMemo(
    () =>
      isChannelScope && relayAgents !== undefined
        ? getChannelOwnedAgentPubkeys({
            channelMembers,
            currentPubkey,
            profiles,
          })
        : EMPTY_AGENT_PUBKEYS,
    [channelMembers, currentPubkey, isChannelScope, profiles, relayAgents],
  );

  return React.useMemo(
    () =>
      getMentionableAgentPubkeys({
        channelOwnedAgentPubkeys,
        currentPubkey,
        eligibilityScope:
          channelId && isChannelScope
            ? { type: "channel", channelId }
            : { type: "managed-only" },
        managedAgentPubkeys,
        relayAgents,
        sharedChannelIds,
      }),
    [
      channelId,
      channelOwnedAgentPubkeys,
      currentPubkey,
      isChannelScope,
      managedAgentPubkeys,
      relayAgents,
      sharedChannelIds,
    ],
  );
}
