import * as React from "react";
import { applyReusableAgentAccessPolicy } from "@/features/agents/channelAgents";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  getErrorMessage,
  isManagedAgentRunning,
  isProviderBackedAgent,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";

/** What the send path learned while making the mentioned agents ready. */
export type EnsureAgentMentionsReadyResult = {
  /**
   * Agent wakes this call actually fired. A wake is detached, so a send that
   * fired one should still read as fast as one that did not; a wake already in
   * flight is suppressed and does not count.
   */
  detachedStarts: number;
  errors: string[];
  pubkeys: string[];
  /**
   * Whether an awaited relay write ran, which tells the publish boundary that
   * something separated it from the pre-side-effect authorization pass.
   */
  wroteRelayState: boolean;
};

export type EnsureAgentMentionsReady = (
  mentionPubkeys: string[],
  capturedChannelId: string,
  preparedParticipantPubkeys?: string[],
  preparedManagedAgents?: ManagedAgent[],
) => Promise<EnsureAgentMentionsReadyResult>;

type AttachAgentToChannel = (input: {
  channelId: string;
  agent: ManagedAgent;
  role: "bot";
  detachedStart: (agent: ManagedAgent) => void;
}) => Promise<unknown>;

type UseEnsureAgentMentionsReadyOptions = {
  attachAgentToChannel: AttachAgentToChannel;
  getManagedAgentsByPubkey: () => Promise<Map<string, ManagedAgent>>;
  getPersonas: () => Promise<AgentPersona[]>;
  memberPubkeys: ReadonlySet<string>;
  startAgentDetached: (agent: ManagedAgent) => boolean;
};

/**
 * Reconcile every mentioned managed agent into a state where it will see the
 * message about to be published: access policy applied, channel membership
 * written, and — for an agent that is not already up — a detached wake fired.
 *
 * The membership write is awaited because the harness only subscribes to
 * channels it belongs to; only the start itself is detached.
 */
export function useEnsureAgentMentionsReady({
  attachAgentToChannel,
  getManagedAgentsByPubkey,
  getPersonas,
  memberPubkeys,
  startAgentDetached,
}: UseEnsureAgentMentionsReadyOptions): EnsureAgentMentionsReady {
  return React.useCallback(
    async (
      mentionPubkeys: string[],
      capturedChannelId: string,
      preparedParticipantPubkeys: string[] = [],
      preparedManagedAgents: ManagedAgent[] = [],
    ) => {
      if (!capturedChannelId || mentionPubkeys.length === 0) {
        return {
          detachedStarts: 0,
          errors: [] as string[],
          pubkeys: [] as string[],
          wroteRelayState: false,
        };
      }
      const [managedAgentsByPubkey, personas] = await Promise.all([
        getManagedAgentsByPubkey(),
        getPersonas(),
      ]);
      for (const agent of preparedManagedAgents) {
        managedAgentsByPubkey.set(normalizePubkey(agent.pubkey), agent);
      }
      const existingMembers = new Set([...memberPubkeys].map(normalizePubkey));
      const participants = new Set([
        ...existingMembers,
        ...preparedParticipantPubkeys.map(normalizePubkey),
      ]);
      const errors: string[] = [];
      const pubkeys: string[] = [];
      let wroteRelayState = false;
      let detachedStarts = 0;
      const countDetachedStart = (agent: ManagedAgent) => {
        if (startAgentDetached(agent)) detachedStarts += 1;
      };
      for (const pubkey of uniqueNormalizedPubkeys(mentionPubkeys)) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) continue;
        try {
          const { agent: readyAgent, wrote } = existingMembers.has(pubkey)
            ? { agent, wrote: false }
            : await applyReusableAgentAccessPolicy(
                agent,
                {},
                personas.find((persona) => persona.id === agent.personaId),
              );
          if (wrote) {
            // The access-policy reconciliation hit the relay; a matching
            // policy reports `wrote: false` and stays on the fast path.
            wroteRelayState = true;
          }
          if (participants.has(pubkey)) {
            if (
              (isProviderBackedAgent(readyAgent) &&
                readyAgent.status !== "deployed") ||
              (!isProviderBackedAgent(readyAgent) &&
                !isManagedAgentRunning(readyAgent))
            ) {
              countDetachedStart(readyAgent);
            }
          } else {
            await attachAgentToChannel({
              channelId: capturedChannelId,
              agent: readyAgent,
              role: "bot",
              detachedStart: countDetachedStart,
            });
            wroteRelayState = true;
          }
          pubkeys.push(pubkey);
        } catch (error) {
          errors.push(
            `${agent.name}: ${getErrorMessage(error, "Could not prepare agent.")}`,
          );
        }
      }
      return {
        detachedStarts,
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
        wroteRelayState,
      };
    },
    [
      attachAgentToChannel,
      getManagedAgentsByPubkey,
      getPersonas,
      memberPubkeys,
      startAgentDetached,
    ],
  );
}
