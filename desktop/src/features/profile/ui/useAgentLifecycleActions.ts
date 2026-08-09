import * as React from "react";
import { toast } from "sonner";

import {
  isManagedAgentActive,
  respawnManagedAgentWithRules,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { clearActiveTurnsForAgentOnStop } from "@/features/agents/managedAgentRuntimeHooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { placeLookupFromLocationProof } from "@/features/presence/lib/presencePlace";
import { loadRemoteHostConnection } from "@/features/remote-agents/remoteHostSettings";
import { hostAgentdLocationProof } from "@/features/remote-agents/hostAgentdClient";
import type { Channel, ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

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
  const presencePubkeys = React.useMemo(
    () => (managedAgent ? [normalizePubkey(managedAgent.pubkey)] : []),
    [managedAgent],
  );
  const presenceQuery = usePresenceQuery(presencePubkeys, {
    enabled: presencePubkeys.length > 0,
  });

  const resolvePlaceHint = React.useCallback(async () => {
    if (!managedAgent) return null;
    const conn = loadRemoteHostConnection();
    if (!conn?.baseUrl || !conn.token) return null;
    try {
      const proof = await hostAgentdLocationProof(
        conn.baseUrl,
        conn.token,
        "public",
      );
      const map = placeLookupFromLocationProof(proof);
      return map[normalizePubkey(managedAgent.pubkey)] ?? null;
    } catch {
      return null;
    }
  }, [managedAgent]);

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

      const placeHint = await resolvePlaceHint();
      await startManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
        presenceLookup: presenceQuery.data,
        placeHint,
      });
      toast.success(
        managedAgent.backend.type === "provider"
          ? `Deploying ${managedAgent.name}.`
          : `Started ${managedAgent.name} on this computer.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent action failed.",
      );
    }
  }, [
    channels,
    managedAgent,
    presenceQuery.data,
    relayAgents,
    resolvePlaceHint,
    startManagedAgent,
    stopManagedAgent,
  ]);

  const handleAgentRestart = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      const placeHint = await resolvePlaceHint();
      await respawnManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
        stopManagedAgent,
        presenceLookup: presenceQuery.data,
        placeHint,
        onStopped: () => clearActiveTurnsForAgentOnStop(managedAgent.pubkey),
      });
      toast.success(`Restarted ${managedAgent.name} on this computer.`);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent restart failed.",
      );
    }
  }, [
    managedAgent,
    presenceQuery.data,
    resolvePlaceHint,
    startManagedAgent,
    stopManagedAgent,
  ]);

  return { handleAgentPrimaryAction, handleAgentRestart };
}
