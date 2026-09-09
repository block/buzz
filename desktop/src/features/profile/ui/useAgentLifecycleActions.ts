import * as React from "react";
import { toast } from "sonner";

import {
  isManagedAgentActive,
  needsProviderAttestationRecovery,
  respawnManagedAgentWithRules,
  type StartManagedAgentInput,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { agentPresenceStartBlockReason } from "@/features/agents/lib/useAgentAvailability";
import { clearActiveTurnsForAgentOnStop } from "@/features/agents/managedAgentRuntimeHooks";
import { useProviderEnrollmentScope } from "@/features/agents/lib/useProviderEnrollmentScope";
import type {
  Channel,
  ManagedAgent,
  RelayAgent,
  PresenceStatus,
} from "@/shared/api/types";

export function useAgentLifecycleActions({
  availability,
  channels,
  managedAgent,
  relayAgents,
  startManagedAgent,
  stopManagedAgent,
}: {
  availability: PresenceStatus | undefined;
  channels: readonly Channel[] | undefined;
  managedAgent: ManagedAgent | undefined;
  relayAgents: readonly RelayAgent[] | undefined;
  startManagedAgent: (input: StartManagedAgentInput) => Promise<unknown>;
  stopManagedAgent: (pubkey: string) => Promise<unknown>;
}) {
  const { expectedRelayUrl, expectedSignerPubkey } =
    useProviderEnrollmentScope();
  const handleAgentPrimaryAction = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      if (isManagedAgentActive(managedAgent)) {
        const result = await stopManagedAgentWithRules({
          agent: managedAgent,
          channels: channels ?? [],
          relayAgents: relayAgents ?? [],
          stopManagedAgent,
        });
        if (managedAgent.backend.type === "local") {
          clearActiveTurnsForAgentOnStop(managedAgent.pubkey);
        }
        toast.success(result.noticeMessage ?? `Stopped ${managedAgent.name}.`);
        return;
      }

      const isAttestationRecovery =
        needsProviderAttestationRecovery(managedAgent);
      if (!isAttestationRecovery) {
        const blockReason = agentPresenceStartBlockReason(false, availability);
        if (blockReason) throw new Error(blockReason);
      }
      if (
        isAttestationRecovery &&
        (!expectedRelayUrl || !expectedSignerPubkey)
      ) {
        throw new Error("Community enrollment scope is unavailable.");
      }
      await startManagedAgentWithRules({
        agent: managedAgent,
        expectedRelayUrl: isAttestationRecovery ? expectedRelayUrl : undefined,
        expectedSignerPubkey: isAttestationRecovery
          ? expectedSignerPubkey
          : undefined,
        startManagedAgent,
      });
      toast.success(
        isAttestationRecovery
          ? `Retrying enrollment for ${managedAgent.name}.`
          : managedAgent.backend.type === "provider"
            ? `Deploying ${managedAgent.name}.`
            : `Started ${managedAgent.name}.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent action failed.",
      );
    }
  }, [
    availability,
    channels,
    expectedRelayUrl,
    expectedSignerPubkey,
    managedAgent,
    relayAgents,
    startManagedAgent,
    stopManagedAgent,
  ]);

  const handleAgentRestart = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      const blockReason = agentPresenceStartBlockReason(
        isManagedAgentActive(managedAgent),
        availability,
      );
      if (blockReason) throw new Error(blockReason);
      await respawnManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
        stopManagedAgent,
        onStopped: () => clearActiveTurnsForAgentOnStop(managedAgent.pubkey),
      });
      toast.success(`Restarted ${managedAgent.name}.`);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent restart failed.",
      );
    }
  }, [availability, managedAgent, startManagedAgent, stopManagedAgent]);

  return { handleAgentPrimaryAction, handleAgentRestart };
}
