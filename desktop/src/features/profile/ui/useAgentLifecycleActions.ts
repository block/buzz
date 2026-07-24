import * as React from "react";
import { toast } from "sonner";

import {
  isManagedAgentActive,
  respawnManagedAgentWithRules,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import type { Channel, ManagedAgent, RelayAgent } from "@/shared/api/types";

export function useAgentLifecycleActions({
  channels,
  managedAgent,
  relayAgents,
  startManagedAgent,
  stopManagedAgent,
}: {
  channels: readonly Channel[] | undefined;
  managedAgent: ManagedAgent | undefined;
  relayAgents: readonly RelayAgent[] | undefined;
  startManagedAgent: (pubkey: string) => Promise<unknown>;
  stopManagedAgent: (pubkey: string) => Promise<unknown>;
}) {
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
        toast.success(result.noticeMessage ?? `Stopped ${managedAgent.name}.`);
        return;
      }

      await startManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
      });
      toast.success(
        managedAgent.backend.type === "provider"
          ? `Deploying ${managedAgent.name}.`
          : `Started ${managedAgent.name}.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent action failed.",
      );
    }
  }, [
    channels,
    managedAgent,
    relayAgents,
    startManagedAgent,
    stopManagedAgent,
  ]);

  const handleAgentRestart = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      await respawnManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
        stopManagedAgent,
      });
      toast.success(`Restarted ${managedAgent.name}.`);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent restart failed.",
      );
    }
  }, [managedAgent, startManagedAgent, stopManagedAgent]);

  return { handleAgentPrimaryAction, handleAgentRestart };
}
