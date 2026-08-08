import * as React from "react";
import { toast } from "sonner";

import {
  isManagedAgentLive,
  managedAgentPresence,
  respawnManagedAgentWithRules,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { clearActiveTurnsForAgentOnStop } from "@/features/agents/managedAgentRuntimeHooks";
import { usePresenceQuery } from "@/features/presence/hooks";
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
  // The live axis for remote agents (I3). Owned here rather than passed in, so every caller of
  // this hook gets the presence-aware branch; react-query dedupes the subscription.
  const presenceQuery = usePresenceQuery(
    managedAgent ? [managedAgent.pubkey] : [],
  );
  const presence = managedAgent
    ? managedAgentPresence(managedAgent, presenceQuery.data)
    : { status: undefined, loaded: false };

  const handleAgentPrimaryAction = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      // Remote agents: a shut-down agent must fall through to the deploy arm. Keying this on
      // the deployment record instead of presence made that arm unreachable, so a remote agent
      // could never be brought back from this surface.
      if (isManagedAgentLive(managedAgent, presence)) {
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
    presence,
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
        onStopped: () => clearActiveTurnsForAgentOnStop(managedAgent.pubkey),
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
