import * as React from "react";
import { applyReusableAgentAccessPolicy } from "@/features/agents/channelAgents";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  enqueueAgentWake,
  getErrorMessage,
  isManagedAgentRunning,
  isProviderBackedAgent,
  type QueuedAgentWake,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";

/** What the send path learned while making the mentioned agents ready. */
export type EnsureAgentMentionsReadyResult = {
  errors: string[];
  pubkeys: string[];
  /**
   * Whether an awaited relay write ran. Informational only: the publish
   * boundary revalidates mention authorization unconditionally, so nothing
   * consumes this to decide anything — it stays because the signal is
   * truthful by construction and unit-pinned.
   */
  wroteRelayState: boolean;
  /**
   * Detached wakes this pass queued instead of firing. The caller flushes
   * them only after the relay accepts the publish, so a wake — and its
   * failure toast claiming "your message was sent" — can never precede the
   * publish outcome, and an aborted send simply drops the queue. Each entry's
   * replay floor was stamped at enqueue time (see `QueuedAgentWake`).
   */
  agentsToWake: QueuedAgentWake[];
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
};

/**
 * Reconcile every mentioned managed agent into a state where it will see the
 * message about to be published: access policy applied, channel membership
 * written, and — for an agent that is not already up — a detached wake
 * queued on the result for the caller to flush once the publish succeeds.
 *
 * The membership write is awaited because the harness only subscribes to
 * channels it belongs to; only the start itself is detached, and it is
 * queued rather than fired so it cannot outrun the publish it exists for.
 */
export function useEnsureAgentMentionsReady({
  attachAgentToChannel,
  getManagedAgentsByPubkey,
  getPersonas,
  memberPubkeys,
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
          errors: [] as string[],
          pubkeys: [] as string[],
          wroteRelayState: false,
          agentsToWake: [] as QueuedAgentWake[],
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
      const agentsToWake: QueuedAgentWake[] = [];
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
            // policy reports `wrote: false`.
            wroteRelayState = true;
          }
          if (participants.has(pubkey)) {
            if (
              (isProviderBackedAgent(readyAgent) &&
                readyAgent.status !== "deployed") ||
              (!isProviderBackedAgent(readyAgent) &&
                !isManagedAgentRunning(readyAgent))
            ) {
              enqueueAgentWake(agentsToWake, readyAgent);
            }
          } else {
            await attachAgentToChannel({
              channelId: capturedChannelId,
              agent: readyAgent,
              role: "bot",
              detachedStart: (agentToWake) =>
                enqueueAgentWake(agentsToWake, agentToWake),
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
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
        wroteRelayState,
        agentsToWake,
      };
    },
    [
      attachAgentToChannel,
      getManagedAgentsByPubkey,
      getPersonas,
      memberPubkeys,
    ],
  );
}
