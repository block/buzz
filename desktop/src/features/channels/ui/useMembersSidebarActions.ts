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
  channelsQueryKey,
  useRemoveChannelMemberMutation,
} from "@/features/channels/hooks";
import { removeChannelMember } from "@/shared/api/tauri";
import type { ChannelMember, ManagedAgent } from "@/shared/api/types";

type UseMembersSidebarActionsOptions = {
  channelId: string | null;
  controllableManagedBots: readonly ManagedAgent[];
  removableManagedBots: readonly ManagedAgent[];
  currentPubkey?: string;
  onOpenChange: (open: boolean) => void;
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
  controllableManagedBots,
  removableManagedBots,
  currentPubkey,
  onOpenChange,
}: UseMembersSidebarActionsOptions) {
  const queryClient = useQueryClient();
  const removeMemberMutation = useRemoveChannelMemberMutation(channelId);
  const startManagedAgentMutation = useStartManagedAgentMutation();
  const stopManagedAgentMutation = useStopManagedAgentMutation();
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
    stopManagedAgentMutation.isPending;

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

  async function handleLifecycleAction(agent: ManagedAgent) {
    clearActionFeedback();
    setActiveActionKey(`agent:${agent.pubkey}`);

    try {
      if (isManagedAgentActive(agent)) {
        await stopManagedAgentWithRules({
          agent,
          ...EMPTY_AGENT_CONTEXT,
          preferredChannelId: channelId,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
        });
        setActionNoticeMessage(
          agent.backend.type === "provider"
            ? `Shutdown command sent to ${agent.name}.`
            : `Stopped ${agent.name}.`,
        );
        return;
      }

      await startManagedAgentWithRules({
        agent,
        startManagedAgent: startManagedAgentMutation.mutateAsync,
      });
      setActionNoticeMessage(getLifecycleSuccessMessage(agent));
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to control agent.",
      );
    } finally {
      setActiveActionKey(null);
    }
  }

  async function handleRespawnAll() {
    await runBulkAgentAction({
      action: async (agent) => {
        await respawnManagedAgentWithRules({
          agent,
          startManagedAgent: startManagedAgentMutation.mutateAsync,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
        });
        return undefined;
      },
      actionKey: "bulk-respawn",
      agents: controllableManagedBots,
      failureMessage: "Failed to respawn agent.",
      successMessage: (count) =>
        `Spawned or respawned ${formatCountLabel(count, "agent", "agents")}.`,
    });
  }

  async function handleStopAll() {
    await runBulkAgentAction({
      action: (agent) =>
        stopManagedAgentWithRules({
          agent,
          ...EMPTY_AGENT_CONTEXT,
          preferredChannelId: channelId,
          stopManagedAgent: stopManagedAgentMutation.mutateAsync,
        }),
      actionKey: "bulk-stop",
      agents: stoppableManagedBots,
      failureMessage: "Failed to stop agent.",
      successMessage: (count) =>
        `Stopped or requested shutdown for ${formatCountLabel(
          count,
          "agent",
          "agents",
        )}.`,
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
      failureMessage: "Failed to remove bot from channel.",
      onSettled: invalidateSidebarQueries,
      successMessage: (count) =>
        `Removed ${formatCountLabel(count, "managed bot", "managed bots")} from this channel.`,
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
            error instanceof Error ? error.message : "Failed to remove member.",
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
    return `Deployed ${agent.name}.`;
  }

  return agent.status === "stopped"
    ? `Respawned ${agent.name}.`
    : `Spawned ${agent.name}.`;
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

function formatCountLabel(count: number, singular: string, plural: string) {
  return `${count} ${count === 1 ? singular : plural}`;
}
