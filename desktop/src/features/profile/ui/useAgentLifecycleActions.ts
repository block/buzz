import * as React from "react";
import { toast } from "sonner";

import {
  isManagedAgentActive,
  respawnManagedAgentWithRules,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { agentPresenceStartBlockReason } from "@/features/agents/lib/useAgentAvailability";
import { clearActiveTurnsForAgentOnStop } from "@/features/agents/managedAgentRuntimeHooks";
import { useTranslation } from "@/i18n";
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
  startManagedAgent: (pubkey: string) => Promise<unknown>;
  stopManagedAgent: (pubkey: string) => Promise<unknown>;
}) {
  const { t } = useTranslation();
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
        toast.success(
          result.noticeMessage ??
            t("profile.agent-actions.stopped", { name: managedAgent.name }),
        );
        return;
      }

      const blockReason = agentPresenceStartBlockReason(false, availability);
      if (blockReason) throw new Error(blockReason);
      await startManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent,
      });
      toast.success(
        managedAgent.backend.type === "provider"
          ? t("profile.agent-actions.deploying", { name: managedAgent.name })
          : t("profile.agent-actions.started", { name: managedAgent.name }),
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t("profile.agent-actions.action-failed"),
      );
    }
  }, [
    availability,
    channels,
    managedAgent,
    relayAgents,
    startManagedAgent,
    stopManagedAgent,
    t,
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
      toast.success(
        t("profile.agent-actions.restarted", { name: managedAgent.name }),
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t("profile.agent-actions.restart-failed"),
      );
    }
  }, [availability, managedAgent, startManagedAgent, stopManagedAgent, t]);

  return { handleAgentPrimaryAction, handleAgentRestart };
}
