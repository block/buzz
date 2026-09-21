import {
  agentPresenceStartBlockReason,
  type AgentAvailabilityReader,
} from "@/features/agents/lib/useAgentAvailability";
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
} from "@/features/agents/hooks";
import {
  respawnManagedAgentWithRules,
  isManagedAgentActive,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import {
  clearActiveTurnsForAgentOnStop,
  useManagedAgentRuntimeAction,
} from "@/features/agents/managedAgentRuntimeHooks";
import { managedAgentPairAction } from "@/features/agents/managedAgentRuntimeStatus";
import {
  channelsQueryKey,
  useRemoveChannelMemberMutation,
} from "@/features/channels/hooks";
import { removeChannelMember } from "@/shared/api/tauri";
import type {
  ChannelMember,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";
import { i18n } from "@/i18n";

type UseMembersSidebarActionsOptions = {
  channelId: string | null;
  getAvailability: AgentAvailabilityReader;
  controllableManagedBots: readonly ManagedAgent[];
  removableManagedBots: readonly ManagedAgent[];
  currentPubkey?: string;
  onOpenChange: (open: boolean) => void;
  /** Active community relay. When set, local-agent lifecycle actions are
   * scoped to this agent+community pair instead of the whole agent. */
  relayUrl?: string;
};

type BulkAgentActionResult = {
  cancelled?: boolean;
};

const EMPTY_AGENT_CONTEXT = {
  channels: [],
  relayAgents: [],
} as const;

export function useMembersSidebarActions({
  channelId,
  getAvailability,
  controllableManagedBots,
  removableManagedBots,
  currentPubkey,
  onOpenChange,
  relayUrl,
}: UseMembersSidebarActionsOptions) {
  const queryClient = useQueryClient();
  function assertStartNotBlockedByPresence(
    agent: ManagedAgent,
    lifecycleActive: boolean,
  ) {
    const reason = agentPresenceStartBlockReason(
      lifecycleActive,
      getAvailability(agent.pubkey),
    );
    if (reason) throw new Error(reason);
  }
  const removeMemberMutation = useRemoveChannelMemberMutation(channelId);
  const startManagedAgentMutation = useStartManagedAgentMutation();
  const stopManagedAgentMutation = useStopManagedAgentMutation();
  const runtimeActionMutation = useManagedAgentRuntimeAction();
  const [actionNoticeMessage, setActionNoticeMessage] = React.useState<
    string | null
  >(null);
  const [actionErrorMessage, setActionErrorMessage] = React.useState<
    string | null
  >(null);
  const [activeActionKey, setActiveActionKey] = React.useState<string | null>(
    null,
  );

  const stoppableManagedBots = React.useMemo(
    () =>
      controllableManagedBots.filter((agent) => isManagedAgentActive(agent)),
    [controllableManagedBots],
  );

  const isActionPending =
    activeActionKey !== null ||
    removeMemberMutation.isPending ||
    startManagedAgentMutation.isPending ||
    stopManagedAgentMutation.isPending ||
    runtimeActionMutation.isPending;

  const clearActionFeedback = React.useCallback(() => {
    setActionNoticeMessage(null);
    setActionErrorMessage(null);
  }, []);

  async function runBulkAgentAction({
    action,
    actionKey,
    agents,
    failureMessage,
    onSettled,
    successMessage,
  }: {
    action: (agent: ManagedAgent) => Promise<BulkAgentActionResult | undefined>;
    actionKey: string;
    agents: readonly ManagedAgent[];
    failureMessage: string;
    onSettled?: () => Promise<void>;
    successMessage: (count: number) => string;
  }) {
    clearActionFeedback();
    setActiveActionKey(actionKey);
    const failures: Array<{ error: string; name: string }> = [];
    let successCount = 0;

    try {
      for (const agent of agents) {
        try {
          const result = await action(agent);
          if (result?.cancelled) {
            break;
          }

          successCount += 1;
        } catch (error) {
          failures.push({
            error: error instanceof Error ? error.message : failureMessage,
            name: agent.name,
          });
        }
      }

      if (successCount > 0) {
        setActionNoticeMessage(successMessage(successCount));
      }

      const failureSummary = formatFailureSummary(failures);
      if (failureSummary) {
        setActionErrorMessage(failureSummary);
      }
    } finally {
      if (onSettled) {
        await onSettled();
      }
      setActiveActionKey(null);
    }
  }

  async function handleLifecycleAction(
    agent: ManagedAgent,
    runtime?: ManagedAgentRuntimeStatus,
  ) {
    clearActionFeedback();
    setActiveActionKey(`agent:${agent.pubkey}`);

    try {
      // Local agents run one harness per agent+community pair. Scope the
      // action to the active community so stopping the agent here never
      // touches its runtimes in other communities. Provider agents keep the
      // agent-wide deploy/!shutdown flow below.
      if (agent.backend.type === "local" && relayUrl) {
        const action = managedAgentPairAction(runtime);
        assertStartNotBlockedByPresence(agent, action === "stop");
        await runtimeActionMutation.mutateAsync({
          action,
          pubkey: agent.pubkey,
          relayUrl,
        });
        setActionNoticeMessage(
          action === "stop"
            ? i18n.t("channels.members.stopped-in-community", {
                name: agent.name,
              })
            : action === "restart"
              ? i18n.t("channels.members.restarted-in-community", {
                  name: agent.name,
                })
              : i18n.t("channels.members.started-in-community", {
                  name: agent.name,
                }),
        );
        return;
      }

      if (isManagedAgentActive(agent)) {
        await stopManagedAgentWithRules({
          agent,
          ...EMPTY_AGENT_CONTEXT,
          preferredChannelId: channelId,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
        });
        if (agent.backend.type === "local") {
          clearActiveTurnsForAgentOnStop(agent.pubkey);
        }
        setActionNoticeMessage(
          agent.backend.type === "provider"
            ? i18n.t("channels.members.shutdown-sent", { name: agent.name })
            : i18n.t("channels.members.stopped-agent", { name: agent.name }),
        );
        return;
      }

      assertStartNotBlockedByPresence(agent, false);
      await startManagedAgentWithRules({
        agent,
        startManagedAgent: startManagedAgentMutation.mutateAsync,
      });
      setActionNoticeMessage(getLifecycleSuccessMessage(agent));
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error
          ? error.message
          : i18n.t("channels.members.control-failed"),
      );
    } finally {
      setActiveActionKey(null);
    }
  }

  async function handleRespawnAll() {
    await runBulkAgentAction({
      action: async (agent) => {
        assertStartNotBlockedByPresence(agent, isManagedAgentActive(agent));
        await respawnManagedAgentWithRules({
          agent,
          startManagedAgent: startManagedAgentMutation.mutateAsync,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
          onStopped: () => clearActiveTurnsForAgentOnStop(agent.pubkey),
        });
        return undefined;
      },
      actionKey: "bulk-respawn",
      agents: controllableManagedBots,
      failureMessage: i18n.t("channels.members.respawn-failed"),
      successMessage: (count) =>
        i18n.t("channels.members.spawned-agents", { count }),
    });
  }

  async function handleStopAll() {
    await runBulkAgentAction({
      action: async (agent) => {
        const result = await stopManagedAgentWithRules({
          agent,
          ...EMPTY_AGENT_CONTEXT,
          preferredChannelId: channelId,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
        });
        if (agent.backend.type === "local") {
          clearActiveTurnsForAgentOnStop(agent.pubkey);
        }
        return result;
      },
      actionKey: "bulk-stop",
      agents: stoppableManagedBots,
      failureMessage: i18n.t("channels.members.stop-failed"),
      successMessage: (count) =>
        i18n.t("channels.members.stopped-agents", { count }),
    });
  }

  async function handleRemoveAll() {
    await runBulkAgentAction({
      action: async (agent) => {
        await removeManagedBotMembership(agent.pubkey);
        return undefined;
      },
      actionKey: "bulk-remove",
      agents: removableManagedBots,
      failureMessage: i18n.t("channels.members.remove-bot-failed"),
      onSettled: invalidateSidebarQueries,
      successMessage: (count) =>
        i18n.t("channels.members.removed-bots", { count }),
    });
  }

  const handleRemoveMember = React.useCallback(
    (member: ChannelMember) => {
      clearActionFeedback();
      setActiveActionKey(`remove:${member.pubkey}`);
      void removeMemberMutation
        .mutateAsync(member.pubkey)
        .then(() => {
          if (member.pubkey === currentPubkey) {
            onOpenChange(false);
          }
        })
        .catch((error: unknown) => {
          setActionErrorMessage(
            error instanceof Error
              ? error.message
              : i18n.t("channels.members.remove-member-failed"),
          );
        })
        .finally(() => {
          setActiveActionKey(null);
        });
    },
    [clearActionFeedback, currentPubkey, onOpenChange, removeMemberMutation],
  );

  async function removeManagedBotMembership(pubkey: string) {
    if (!channelId) {
      throw new Error("No channel selected.");
    }

    await removeChannelMember(channelId, pubkey);
  }

  async function invalidateSidebarQueries() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
      channelId
        ? queryClient.invalidateQueries({ queryKey: ["channels", channelId] })
        : Promise.resolve(),
      queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
      queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
    ]);
  }

  return {
    actionErrorMessage,
    actionNoticeMessage,
    handleLifecycleAction,
    handleRemoveAll,
    handleRemoveMember,
    handleRespawnAll,
    handleStopAll,
    isActionPending,
    hasControllableManagedBots: controllableManagedBots.length > 0,
    hasRemovableManagedBots: removableManagedBots.length > 0,
    hasStoppableManagedBots: stoppableManagedBots.length > 0,
  };
}

function getLifecycleSuccessMessage(agent: ManagedAgent) {
  if (agent.backend.type === "provider") {
    return i18n.t("channels.members.deployed", { name: agent.name });
  }

  return agent.status === "stopped"
    ? i18n.t("channels.members.respawned", { name: agent.name })
    : i18n.t("channels.members.spawned", { name: agent.name });
}

function formatFailureSummary(
  failures: Array<{
    error: string;
    name: string;
  }>,
) {
  if (failures.length === 0) {
    return null;
  }

  if (failures.length === 1) {
    const [failure] = failures;
    return `${failure.name}: ${failure.error}`;
  }

  return failures
    .map((failure) => `${failure.name}: ${failure.error}`)
    .join("; ");
}
