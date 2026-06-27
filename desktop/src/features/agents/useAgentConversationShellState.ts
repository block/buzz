import * as React from "react";

import type { Channel } from "@/shared/api/types";
import {
  buildAgentConversation,
  type AgentConversation,
  type AgentConversationTitleStatus,
  type OpenAgentConversationInput,
  publishAgentConversationMarker,
  readHiddenAgentConversationIds,
  readPersistedAgentConversations,
  writeHiddenAgentConversationIds,
  writePersistedAgentConversations,
} from "./agentConversations";

type GoAgents = () => Promise<boolean>;
type GoChannel = (
  channelId: string,
  options?: {
    messageId?: string;
    replace?: boolean;
    taskReplyId?: string;
    threadRootId?: string | null;
  },
) => Promise<boolean>;

type AgentConversationShellStateInput = {
  channels: readonly Channel[];
  currentPubkey?: string;
  enabled?: boolean;
  goAgents: GoAgents;
  goChannel: GoChannel;
  selectedView: string;
  workspaceScope?: string | null;
};

export function useAgentConversationShellState({
  channels,
  currentPubkey,
  enabled = true,
  goAgents,
  goChannel,
  selectedView,
  workspaceScope,
}: AgentConversationShellStateInput) {
  const [agentConversations, setAgentConversations] = React.useState<
    AgentConversation[]
  >([]);
  const [hiddenAgentConversationIds, setHiddenAgentConversationIds] =
    React.useState<Set<string>>(() => new Set());
  const [agentConversationStorageKey, setAgentConversationStorageKey] =
    React.useState<string | null>(null);
  const [selectedAgentConversationId, setSelectedAgentConversationId] =
    React.useState<string | null>(null);
  const activeStorageKey =
    currentPubkey && workspaceScope
      ? `${workspaceScope}:${currentPubkey}`
      : null;

  React.useEffect(() => {
    if (!currentPubkey || !workspaceScope) {
      setAgentConversations([]);
      setHiddenAgentConversationIds(new Set());
      setAgentConversationStorageKey(null);
      return;
    }

    setAgentConversations(
      readPersistedAgentConversations(currentPubkey, workspaceScope),
    );
    setHiddenAgentConversationIds(
      readHiddenAgentConversationIds(currentPubkey, workspaceScope),
    );
    setAgentConversationStorageKey(activeStorageKey);
  }, [activeStorageKey, currentPubkey, workspaceScope]);

  React.useEffect(() => {
    if (
      !currentPubkey ||
      !workspaceScope ||
      agentConversationStorageKey !== activeStorageKey
    ) {
      return;
    }

    writePersistedAgentConversations(
      currentPubkey,
      workspaceScope,
      agentConversations,
    );
  }, [
    activeStorageKey,
    agentConversationStorageKey,
    agentConversations,
    currentPubkey,
    workspaceScope,
  ]);

  React.useEffect(() => {
    if (!enabled) {
      setSelectedAgentConversationId(null);
    }
  }, [enabled]);

  const selectedAgentConversation =
    enabled && selectedView === "agents" && selectedAgentConversationId
      ? (agentConversations.find(
          (conversation) => conversation.id === selectedAgentConversationId,
        ) ?? null)
      : null;

  const visibleAgentConversations = React.useMemo(
    () =>
      enabled
        ? agentConversations.filter(
            (conversation) => !hiddenAgentConversationIds.has(conversation.id),
          )
        : [],
    [agentConversations, enabled, hiddenAgentConversationIds],
  );

  const selectedAgentConversationChannel = selectedAgentConversation
    ? (channels.find(
        (channel) => channel.id === selectedAgentConversation.channelId,
      ) ?? null)
    : null;

  const clearSelectedAgentConversation = React.useCallback(() => {
    setSelectedAgentConversationId(null);
  }, []);

  const openAgentConversation = React.useCallback(
    (
      input: OpenAgentConversationInput,
      options?: { publishMarker?: boolean },
    ) => {
      if (!enabled) {
        return;
      }

      const conversation = buildAgentConversation(input);
      if (options?.publishMarker !== false) {
        void publishAgentConversationMarker(input).catch((error) => {
          console.warn("[agentConversations] marker publish failed:", error);
        });
      }
      if (currentPubkey && workspaceScope) {
        setHiddenAgentConversationIds((current) => {
          if (!current.has(conversation.id)) {
            return current;
          }

          const next = new Set(current);
          next.delete(conversation.id);
          writeHiddenAgentConversationIds(currentPubkey, workspaceScope, next);
          return next;
        });
      }
      setAgentConversations((current) => {
        const existingIndex = current.findIndex(
          (item) => item.id === conversation.id,
        );
        if (existingIndex < 0) {
          return [conversation, ...current];
        }

        const next = [...current];
        next.splice(existingIndex, 1);
        return [conversation, ...next];
      });
      setSelectedAgentConversationId(conversation.id);
      void goAgents();
    },
    [currentPubkey, enabled, goAgents, workspaceScope],
  );

  const updateAgentConversationTitle = React.useCallback(
    (
      conversationId: string,
      title: string,
      titleStatus: AgentConversationTitleStatus,
    ) => {
      setAgentConversations((current) =>
        current.map((conversation) =>
          conversation.id === conversationId
            ? { ...conversation, title, titleStatus }
            : conversation,
        ),
      );
    },
    [],
  );

  const hideAgentConversation = React.useCallback(
    (conversationId: string) => {
      const conversation =
        agentConversations.find((item) => item.id === conversationId) ?? null;
      if (!currentPubkey || !workspaceScope) {
        return;
      }

      setHiddenAgentConversationIds((current) => {
        if (current.has(conversationId)) {
          return current;
        }

        const next = new Set(current);
        next.add(conversationId);
        writeHiddenAgentConversationIds(currentPubkey, workspaceScope, next);
        return next;
      });

      if (selectedAgentConversationId === conversationId) {
        setSelectedAgentConversationId(null);
        if (conversation) {
          void goChannel(conversation.channelId);
        }
      }
    },
    [
      agentConversations,
      currentPubkey,
      goChannel,
      selectedAgentConversationId,
      workspaceScope,
    ],
  );

  const selectAgentConversation = React.useCallback(
    (conversationId: string) => {
      setSelectedAgentConversationId(conversationId);
      void goAgents();
    },
    [goAgents],
  );

  const backToAgentConversationThread = React.useCallback(
    (conversation: AgentConversation) => {
      setSelectedAgentConversationId(null);
      void goChannel(conversation.channelId, {
        messageId: conversation.agentReply.id,
        threadRootId: conversation.threadRootId,
      });
    },
    [goChannel],
  );

  return {
    agentConversations: enabled ? agentConversations : [],
    backToAgentConversationThread,
    clearSelectedAgentConversation,
    hideAgentConversation,
    openAgentConversation,
    selectAgentConversation,
    selectedAgentConversation,
    selectedAgentConversationChannel,
    selectedAgentConversationId: enabled ? selectedAgentConversationId : null,
    updateAgentConversationTitle,
    visibleAgentConversations,
  };
}
