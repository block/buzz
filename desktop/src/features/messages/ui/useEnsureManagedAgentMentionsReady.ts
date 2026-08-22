import * as React from "react";
import {
  type useAttachManagedAgentToChannelMutation,
  useRelayAgentsQuery,
  type useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getErrorMessage,
  isManagedAgentRunning,
  isProviderBackedAgent,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";

/** Outcome of preparing the managed agents named by a message's mentions. */
export type ManagedAgentMentionReadiness = {
  /** Agents that exist here but could not be started or attached. */
  errors: string[];
  /** Agents that live on a different computer and so cannot answer from here. */
  notices: string[];
  /** Agents that are now running and attached to the channel. */
  pubkeys: string[];
};

/**
 * Start or attach every locally owned agent named by a mention, and describe
 * the ones that are not locally owned.
 *
 * The same account signed in on several computers mints a *separate* keypair
 * per computer for the same agent, so a mention can resolve to an identity
 * only another machine can run. Those used to be dropped in silence, which is
 * the reported "I @-mentioned four agents and none replied" failure; they now
 * come back as `notices` naming the device that would have to answer.
 */
export function useEnsureManagedAgentMentionsReady(options: {
  attachAgentMutation: ReturnType<
    typeof useAttachManagedAgentToChannelMutation
  >;
  getManagedAgentsByPubkey: () => Promise<Map<string, ManagedAgent>>;
  memberPubkeys: ReadonlySet<string>;
  startAgentMutation: ReturnType<typeof useStartManagedAgentMutation>;
}) {
  const {
    attachAgentMutation,
    getManagedAgentsByPubkey,
    memberPubkeys,
    startAgentMutation,
  } = options;
  // Deduped by React Query against the identical `relayAgentsQueryKey` already
  // in flight from `useMentions`, so this costs no extra fetch. It is the only
  // place a mention resolved to another computer's keypair can be named.
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgentsByPubkey = React.useMemo(
    () =>
      new Map<string, RelayAgent>(
        (relayAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent,
        ]),
      ),
    [relayAgentsQuery.data],
  );
  return React.useCallback(
    async (
      mentionPubkeys: string[],
      capturedChannelId: string,
      preparedParticipantPubkeys: string[] = [],
      preparedManagedAgents: ManagedAgent[] = [],
    ): Promise<ManagedAgentMentionReadiness> => {
      if (!capturedChannelId || mentionPubkeys.length === 0) {
        return { errors: [], notices: [], pubkeys: [] };
      }
      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      for (const agent of preparedManagedAgents) {
        managedAgentsByPubkey.set(normalizePubkey(agent.pubkey), agent);
      }
      const participantPubkeys = new Set([
        ...memberPubkeys,
        ...preparedParticipantPubkeys.map(normalizePubkey),
      ]);
      const errors: string[] = [];
      const notices: string[] = [];
      const pubkeys: string[] = [];
      for (const pubkey of uniqueNormalizedPubkeys(mentionPubkeys)) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) {
          const notice = describeUnrunnableMention(
            relayAgentsByPubkey.get(pubkey),
          );
          if (notice) notices.push(notice);
          continue;
        }
        try {
          if (participantPubkeys.has(pubkey)) {
            if (isProviderBackedAgent(agent)) {
              if (agent.status !== "deployed") {
                await startAgentMutation.mutateAsync(agent.pubkey);
              }
            } else if (!isManagedAgentRunning(agent)) {
              await startAgentMutation.mutateAsync(agent.pubkey);
            }
          } else {
            await attachAgentMutation.mutateAsync({
              channelId: capturedChannelId,
              agent,
              role: "bot",
            });
          }
          pubkeys.push(pubkey);
        } catch (error) {
          errors.push(
            `${agent.name}: ${getErrorMessage(
              error,
              "Could not prepare agent.",
            )}`,
          );
        }
      }
      return { errors, notices, pubkeys: uniqueNormalizedPubkeys(pubkeys) };
    },
    [
      attachAgentMutation,
      getManagedAgentsByPubkey,
      memberPubkeys,
      relayAgentsByPubkey,
      startAgentMutation,
    ],
  );
}

/**
 * Copy for a mention that resolved to an agent identity this install does not
 * hold the secret for. Names the owning device when the agent published one,
 * and never guesses a device name when it did not.
 *
 * Returns `null` — meaning stay silent, exactly as before this feature — for
 * an agent that declares no device at all. Relay-hosted and pre-feature
 * agents are legitimately not "set up on" any computer, so pinning them to
 * one would be both wrong and noisy on every shared-agent mention.
 */
function describeUnrunnableMention(
  remote: RelayAgent | undefined,
): string | null {
  if (!remote?.deviceId) return null;
  const name = remote.name.trim() || "That agent";
  const device = remote.deviceLabel?.trim();
  return device
    ? `${name} is set up on ${device}, not on this device. Only that device can reply.`
    : `${name} is set up on another device, not on this device. Only that device can reply.`;
}
