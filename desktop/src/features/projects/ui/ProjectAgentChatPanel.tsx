import { Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import {
  projectDetailAgentContextBlock,
  stripProjectDetailAgentContext,
} from "@/features/projects/lib/projectDetailAgentContext";
import { restoreProjectsAgentConversation } from "@/features/projects/lib/projectAgentConversation";
import {
  clearStoredProjectsAgentConversation,
  readStoredProjectsAgentConversation,
  type StoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  type AgentCandidate,
  ConversationThread,
  useAgentCandidates,
} from "./ProjectsAgentPromptPage";
import { ProjectAgentContextStrip } from "./ProjectAgentContextStrip";

type ProjectAgentConversation = {
  agent: AgentCandidate;
  channel: Channel;
  visibleAfter: number;
};

export function ProjectAgentChatPanel({
  canResetWidth,
  context,
  onResetWidth,
  onResizeStart,
  sharedHeaderBackdrop,
  widthPx,
}: {
  canResetWidth: boolean;
  context: ProjectDetailAgentContext;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  sharedHeaderBackdrop?: boolean;
  widthPx: number;
}) {
  const storageScope = `detail:${context.repoAddress}`;
  const [isSending, setIsSending] = React.useState(false);
  const [storedConversation, setStoredConversation] =
    React.useState<StoredProjectsAgentConversation | null>(() =>
      readStoredProjectsAgentConversation(storageScope),
    );
  const [conversation, setConversation] =
    React.useState<ProjectAgentConversation | null>(null);
  const candidates = useAgentCandidates();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const openDmMutation = useOpenDmMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const selectedAgent = conversation?.agent ?? candidates[0] ?? null;
  const candidateProfilesQuery = useUsersBatchQuery(
    selectedAgent ? [selectedAgent.pubkey] : [],
  );
  const selectedAgentAvatarUrl = selectedAgent
    ? (candidateProfilesQuery.data?.profiles[
        normalizePubkey(selectedAgent.pubkey)
      ]?.avatarUrl ?? null)
    : null;
  const restorableConversation = React.useMemo(
    () =>
      restoreProjectsAgentConversation({
        candidates,
        channels: channelsQuery.data ?? [],
        stored: storedConversation,
      }),
    [candidates, channelsQuery.data, storedConversation],
  );

  React.useEffect(() => {
    if (conversation || !restorableConversation) return;
    setConversation(restorableConversation);
  }, [conversation, restorableConversation]);

  const handleSubmit = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const trimmed = content.trim();
      if (!trimmed || !selectedAgent || isSending) return;
      setIsSending(true);
      const visibleAfter =
        conversation?.visibleAfter ?? Math.floor(Date.now() / 1_000);
      try {
        if (selectedAgent.isManaged && !selectedAgent.isActive) {
          await startAgentMutation.mutateAsync(selectedAgent.pubkey);
        }
        const channel =
          conversation?.channel ??
          (await openDmMutation.mutateAsync({
            pubkeys: [selectedAgent.pubkey],
          }));
        await sendChannelMessage(
          channel.id,
          `${trimmed}${projectDetailAgentContextBlock(context)}`,
          undefined,
          mediaTags,
          [...new Set([...mentionPubkeys, selectedAgent.pubkey])],
        );
        if (!conversation) {
          const nextConversation = {
            agent: selectedAgent,
            channel,
            visibleAfter,
          };
          const stored = {
            agentPubkey: selectedAgent.pubkey,
            channelId: channel.id,
            visibleAfter,
          };
          setConversation(nextConversation);
          setStoredConversation(stored);
          writeStoredProjectsAgentConversation(storageScope, stored);
        }
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to reach the agent",
        );
      } finally {
        setIsSending(false);
      }
    },
    [
      context,
      conversation,
      isSending,
      openDmMutation,
      selectedAgent,
      startAgentMutation,
      storageScope,
    ],
  );

  const handleClear = React.useCallback(() => {
    clearStoredProjectsAgentConversation(storageScope);
    setStoredConversation(null);
    setConversation(null);
  }, [storageScope]);

  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId="project-agent-chat-panel"
      widthPx={widthPx}
    >
      <ProjectAgentContextStrip
        context={context}
        sharedBackdrop={sharedHeaderBackdrop}
      />
      <div className="flex min-h-0 flex-1 flex-col">
        <div
          className="min-h-0 flex-1 overflow-y-auto px-4 pb-4 pt-[4.25rem]"
          data-testid="project-agent-conversation-scroll"
        >
          {conversation ? (
            <ConversationThread
              agent={conversation.agent}
              agentAvatarUrl={selectedAgentAvatarUrl}
              channel={conversation.channel}
              currentPubkey={identityQuery.data?.pubkey ?? null}
              selfAvatarUrl={profileQuery.data?.avatarUrl ?? null}
              stripSelfContent={stripProjectDetailAgentContext}
              visibleAfter={conversation.visibleAfter}
            />
          ) : (
            <div className="flex h-full min-h-40 flex-col items-center justify-center gap-2 text-center">
              <p className="text-sm font-medium text-foreground">
                Ask about this page
              </p>
              <p className="max-w-56 text-xs text-muted-foreground">
                Start a conversation with the project agent.
              </p>
            </div>
          )}
        </div>
        <MessageComposer
          channelId={conversation?.channel.id ?? null}
          channelName={selectedAgent?.name ?? "project agent"}
          channelType="dm"
          containerClassName="px-3 pb-3"
          disabled={!selectedAgent || isSending}
          draftKey={`project-agent:${storageScope}`}
          isSending={isSending}
          layoutMode="standalone"
          onSend={handleSubmit}
          placeholder={
            selectedAgent
              ? `Message ${selectedAgent.name}`
              : "No agents available"
          }
          profiles={candidateProfilesQuery.data?.profiles}
          showBackgroundUploadProgress={false}
          showTopBorder={false}
          toolbarExtraActions={
            conversation ? (
              <Button
                aria-label="Clear project agent chat"
                className="h-7 w-7"
                onClick={handleClear}
                size="icon"
                title="Clear conversation"
                type="button"
                variant="ghost"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            ) : null
          }
        />
      </div>
    </RightAuxiliaryPane>
  );
}
