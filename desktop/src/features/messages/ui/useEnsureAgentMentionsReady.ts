import * as React from "react";
import { applyReusableAgentAccessPolicy } from "@/features/agents/channelAgents";
import { useRelayAgentsQuery } from "@/features/agents/hooks";
import type {
  AgentPersona,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
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
  /**
   * Agents that live on a different computer and so cannot answer from here.
   * Informational only: the message still sends, the toast just names the
   * device that would have to reply.
   */
  notices: string[];
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
  // Deduped by React Query against the identical `relayAgentsQueryKey`
  // already in flight from `useMentions`, so this costs no extra fetch. It is
  // the only place a mention resolved to another computer's keypair can be
  // named.
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
    ) => {
      if (!capturedChannelId || mentionPubkeys.length === 0) {
        return {
          errors: [] as string[],
          notices: [] as string[],
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
      const notices: string[] = [];
      const pubkeys: string[] = [];
      let wroteRelayState = false;
      const agentsToWake: QueuedAgentWake[] = [];
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
        notices,
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
      relayAgentsByPubkey,
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
