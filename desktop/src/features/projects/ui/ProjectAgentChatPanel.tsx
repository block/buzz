import { Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import { normalizeRelayUrl } from "@/features/communities/communityStorage";
import { useCommunities } from "@/features/communities/useCommunities";
import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { projectDetailAgentContextBlock } from "@/features/projects/lib/projectDetailAgentContext";
import { pickDefaultProjectsAgent } from "@/features/projects/lib/projectAgentSelection";
import {
  projectAgentMembershipInput,
  restoreProjectsAgentConversation,
  submitProjectAgentMessage,
} from "@/features/projects/lib/projectAgentConversation";
import {
  clearStoredProjectsAgentConversation,
  type ProjectsConversationOpener,
  projectsConversationScope,
  readStoredProjectsAgentConversation,
  type StoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { addChannelMembers } from "@/shared/api/tauri";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  type AgentCandidate,
  ConversationThread,
  useAgentCandidates,
} from "./ProjectsAgentPromptPage";
import { ProjectAgentContextStrip } from "./ProjectAgentContextStrip";
import { AgentContextPayloadPreview } from "./AgentContextPayloadPreview";
import { ProjectAgentSelectionComposerBanner } from "./ProjectAgentSelectionComposerBanner";

type ProjectAgentConversation = {
  agent: AgentCandidate;
  channel: Channel;
  opener: ProjectsConversationOpener;
};

export function ProjectAgentChatPanel({
  canResetWidth = false,
  constrainToAvailableSpace = true,
  context,
  detached = false,
  homeChannel = null,
  layout = "pane",
  onClose,
  onResetWidth,
  onResizeStart,
  sharedHeaderBackdrop,
  widthPx = 0,
}: {
  canResetWidth?: boolean;
  constrainToAvailableSpace?: boolean;
  context: ProjectDetailAgentContext;
  detached?: boolean;
  homeChannel?: Channel | null;
  layout?: "pane" | "canvas";
  onClose?: () => void;
  onResetWidth?: () => void;
  onResizeStart?: (event: React.PointerEvent<HTMLButtonElement>) => void;
  sharedHeaderBackdrop?: boolean;
  widthPx?: number;
}) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  // Repository coordinates (`kind:owner:dtag`) are not globally unique — the
  // same address can exist on two relays. Scope persistence, drafts, and
  // restore to the community's relay identity so a community switch can
  // never surface the other tenant's conversation. No relay identity means
  // nothing to safely restore against, so the scope stays null (no-op reads
  // and writes).
  const relayScope = activeCommunity?.relayUrl
    ? normalizeRelayUrl(activeCommunity.relayUrl)
    : null;
  const signerScope = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : null;
  const storageScope = projectsConversationScope(
    "detail",
    relayScope,
    signerScope,
    context.repoAddress,
  );
  const [isSending, setIsSending] = React.useState(false);
  const [storedConversation, setStoredConversation] =
    React.useState<StoredProjectsAgentConversation | null>(() =>
      readStoredProjectsAgentConversation(storageScope),
    );
  const [conversation, setConversation] =
    React.useState<ProjectAgentConversation | null>(null);
  const candidates = useAgentCandidates();
  const channelsQuery = useChannelsQuery();
  const profileQuery = useProfileQuery();
  const openDmMutation = useOpenDmMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const selectedAgent =
    conversation?.agent ?? pickDefaultProjectsAgent(candidates);
  const candidateProfilesQuery = useUsersBatchQuery(
    selectedAgent ? [selectedAgent.pubkey] : [],
  );
  const selectedAgentAvatarUrl = selectedAgent
    ? (candidateProfilesQuery.data?.profiles[
        normalizePubkey(selectedAgent.pubkey)
      ]?.avatarUrl ?? null)
    : null;
  // Computed once per context so the pre-send preview and the appended
  // payload are byte-identical: the user must be able to inspect exactly
  // what will be signed under their key, not a paraphrase of it.
  const contextPayload = React.useMemo(
    () => projectDetailAgentContextBlock(context),
    [context],
  );
  const restorableConversation = React.useMemo(
    () =>
      restoreProjectsAgentConversation({
        candidates,
        channels: channelsQuery.data ?? [],
        currentPubkey: identityQuery.data?.pubkey ?? null,
        homeChannelId: homeChannel?.id ?? null,
        stored: storedConversation,
      }),
    [
      candidates,
      channelsQuery.data,
      homeChannel?.id,
      identityQuery.data?.pubkey,
      storedConversation,
    ],
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
      try {
        // The awaits below suspend across a possible community switch;
        // `submitProjectAgentMessage` binds every relay side effect to the
        // scope captured here (fail closed), and threads follow-ups onto the
        // opener so a same-second follow-up cannot be hidden by id ordering.
        if (
          homeChannel &&
          (!conversation || conversation.channel.id === homeChannel.id)
        ) {
          const alreadyMember = homeChannel.memberPubkeys.some(
            (pubkey) =>
              normalizePubkey(pubkey) === normalizePubkey(selectedAgent.pubkey),
          );
          if (!alreadyMember) {
            await addChannelMembers(
              projectAgentMembershipInput({
                channelId: homeChannel.id,
                agentPubkey: selectedAgent.pubkey,
                relayScope,
                signerScope,
              }),
            );
          }
        }
        const { channel, sent } = await submitProjectAgentMessage({
          agent: selectedAgent,
          conversation,
          content: `${trimmed}${contextPayload}`,
          homeChannel,
          mentionPubkeys: [
            ...new Set([...mentionPubkeys, selectedAgent.pubkey]),
          ],
          mediaTags,
          relayScope,
          signerScope: identityQuery.data?.pubkey ?? null,
          startAgent: (input) => startAgentMutation.mutateAsync(input),
          openDm: (input) => openDmMutation.mutateAsync(input),
          send: (request) =>
            sendChannelMessage(
              request.channelId,
              request.content,
              request.parentEventId,
              request.mediaTags,
              request.mentionPubkeys,
              undefined,
              undefined,
              undefined,
              undefined,
              undefined,
              request.expectedRelayUrl,
              request.expectedSignerPubkey,
            ),
        });
        if (!conversation) {
          // Anchor the conversation to the exact accepted opener event: a
          // bare timestamp cannot isolate it from unrelated same-second DM
          // history.
          const opener = {
            createdAt: sent.createdAt,
            eventId: sent.eventId,
          };
          const nextConversation = {
            agent: selectedAgent,
            channel,
            opener,
          };
          const stored = {
            agentPubkey: selectedAgent.pubkey,
            channelId: channel.id,
            opener,
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
      contextPayload,
      conversation,
      homeChannel,
      identityQuery.data?.pubkey,
      isSending,
      openDmMutation,
      relayScope,
      selectedAgent,
      signerScope,
      startAgentMutation,
      storageScope,
    ],
  );

  const handleClear = React.useCallback(() => {
    clearStoredProjectsAgentConversation(storageScope);
    setStoredConversation(null);
    setConversation(null);
  }, [storageScope]);

  const conversationBody = (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 flex-col",
        (detached || layout === "canvas") && "bg-background",
      )}
    >
      {layout === "pane" ? (
        <ProjectAgentContextStrip
          context={context}
          onClose={onClose}
          sharedBackdrop={sharedHeaderBackdrop}
        />
      ) : null}
      <div className="flex min-h-0 flex-1 flex-col">
        <div
          className={cn(
            "flex min-h-0 flex-1 flex-col overflow-x-hidden overflow-y-auto pb-4",
            layout === "pane" ? "pt-[4.25rem]" : "pt-6",
          )}
          data-testid="project-agent-conversation-scroll"
        >
          {conversation ? (
            <ConversationThread
              agent={conversation.agent}
              agentAvatarUrl={selectedAgentAvatarUrl}
              channel={conversation.channel}
              currentPubkey={identityQuery.data?.pubkey ?? null}
              selfAvatarUrl={profileQuery.data?.avatarUrl ?? null}
              opener={conversation.opener}
            />
          ) : (
            <div className="flex min-h-40 flex-1 flex-col items-center justify-center gap-2 text-center">
              <p className="text-sm font-medium text-foreground">
                {homeChannel
                  ? "Explain what this project should be"
                  : "Ask about this page"}
              </p>
              <p className="max-w-72 text-xs text-muted-foreground">
                {homeChannel
                  ? "The project agent will build it out from this channel."
                  : "Start a conversation with the project agent."}
              </p>
            </div>
          )}
        </div>
        {context.selection?.length ? (
          <ProjectAgentSelectionComposerBanner items={context.selection} />
        ) : null}
        <MessageComposer
          channelId={conversation?.channel.id ?? homeChannel?.id ?? null}
          channelName={selectedAgent?.name ?? "project agent"}
          channelType={homeChannel?.channelType ?? "dm"}
          containerClassName="px-3 pb-3"
          disabled={!selectedAgent || isSending}
          draftKey={`project-agent:${storageScope ?? "unscoped"}`}
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
            <>
              <AgentContextPayloadPreview
                iconOnly
                payload={contextPayload}
                triggerLabel="Preview message context"
              />
              {conversation ? (
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
              ) : null}
            </>
          }
        />
      </div>
    </div>
  );

  if (layout === "canvas") {
    return (
      <div
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        data-testid="project-agent-chat-panel"
      >
        {conversationBody}
      </div>
    );
  }

  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      constrainToAvailableSpace={constrainToAvailableSpace}
      detached={detached}
      onResetWidth={onResetWidth ?? (() => {})}
      onResizeStart={onResizeStart ?? (() => {})}
      testId="project-agent-chat-panel"
      widthPx={widthPx}
    >
      {conversationBody}
    </RightAuxiliaryPane>
  );
}
