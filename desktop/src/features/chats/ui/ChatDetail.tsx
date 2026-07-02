import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { MessageCircle } from "lucide-react";
import { toast } from "sonner";

import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { scopeByChannel } from "@/features/agents/ui/agentSessionPanelLayout";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useUpdateChatMetadataMutation } from "@/features/chats/hooks";
import {
  buildChatActivityPlacement,
  shouldHidePersistedAgentMessage,
} from "@/features/chats/lib/chatActivity";
import { chatProjectForMetadata } from "@/features/chats/lib/chatProjects";
import {
  buildChatCanvasContent,
  buildProjectSetupContext,
  type ChatProject,
  NO_PROJECT_SELECTION_ID,
} from "@/features/chats/lib/chatSetup";
import { ChatActivityTranscript } from "@/features/chats/ui/ChatActivityTranscript";
import { isHumanFacingAssistantText } from "@/features/chats/ui/chatActivityText";
import {
  AgentActivationCard,
  ChatContextRow,
  ChatMessageRow,
  ChatScrollAnchor,
} from "@/features/chats/ui/ChatConversationRows";
import { ProjectPicker } from "@/features/chats/ui/QuickStartChat";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { setCanvas } from "@/shared/api/tauri";
import type {
  Channel,
  ChannelTemplate,
  ChatMetadata,
  ManagedAgent,
  RelayEvent,
} from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/shared/ui/message-scroller";
import { Spinner } from "@/shared/ui/spinner";

import type { UserProfileLookup } from "@/features/profile/lib/identity";

const CHAT_CONVERSATION_CLASS = "mx-auto w-full max-w-4xl px-4 sm:px-6 lg:px-8";

function eventHasTag(event: RelayEvent, name: string, value?: string) {
  return event.tags.some(
    (tag) => tag[0] === name && (value === undefined || tag[1] === value),
  );
}

type ChatDetailProps = {
  chat: Channel;
  defaultAgent: ManagedAgent | null;
  identityPubkey?: string;
  isLoadingMessages: boolean;
  isActivatingAgent: boolean;
  isSending: boolean;
  messages: RelayEvent[];
  metadata: ChatMetadata | null;
  onActivateAgent: () => void;
  onProjectCreated: (project: ChatProject) => void;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  projects: ChatProject[];
  shareAction?: React.ReactNode;
  templates: ChannelTemplate[];
};

export function ChatDetail({
  chat,
  defaultAgent,
  identityPubkey,
  isActivatingAgent,
  isLoadingMessages,
  isSending,
  messages,
  metadata,
  onActivateAgent,
  onProjectCreated,
  onSend,
  profiles,
  projects,
  shareAction,
  templates,
}: ChatDetailProps) {
  const queryClient = useQueryClient();
  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const hasObserver = defaultAgent ? isManagedAgentActive(defaultAgent) : false;
  const activeAgentTurns = useActiveAgentTurns(defaultAgent?.pubkey);
  const isChatTurnActive = React.useMemo(
    () => activeAgentTurns.some((turn) => turn.channelId === chat.id),
    [activeAgentTurns, chat.id],
  );
  const transcript = useAgentTranscript(hasObserver, defaultAgent?.pubkey);
  const scopedTranscript = React.useMemo(
    () => scopeByChannel(transcript, chat.id),
    [chat.id, transcript],
  );
  const chatActivity = React.useMemo(
    () =>
      buildChatActivityPlacement({
        agentPubkey: defaultAgent?.pubkey,
        messages,
        transcript: scopedTranscript,
      }),
    [defaultAgent?.pubkey, messages, scopedTranscript],
  );
  const selectedProject = React.useMemo(
    () => chatProjectForMetadata(metadata),
    [metadata],
  );
  const handleSelectProject = React.useCallback(
    async (projectId: string | null) => {
      const nextProject =
        projectId && projectId !== NO_PROJECT_SELECTION_ID
          ? (projects.find((project) => project.id === projectId) ?? null)
          : null;
      const nextTemplate = nextProject?.templateId
        ? (templates.find(
            (template) => template.id === nextProject.templateId,
          ) ?? null)
        : null;
      const title = metadata?.title?.trim() || chat.name;

      try {
        await updateMetadataMutation.mutateAsync({
          channelId: chat.id,
          defaultAgentPubkey:
            metadata?.defaultAgentPubkey ?? defaultAgent?.pubkey ?? undefined,
          projectId: nextProject?.id,
          projectName: nextProject?.name,
          projectPath: nextProject?.path ?? undefined,
          projectTemplateId: nextProject?.templateId ?? undefined,
          source: metadata?.sourceChannelId
            ? {
                channelId: metadata.sourceChannelId,
                eventId: metadata.sourceEventId ?? undefined,
                threadRootId: metadata.sourceThreadRootId ?? undefined,
              }
            : undefined,
          templateId: nextProject?.templateId ?? undefined,
          title,
        });

        const leadingContent = buildProjectSetupContext({
          agent: defaultAgent,
          project: nextProject,
          templateName: nextTemplate?.name ?? null,
        });
        const canvasContent = buildChatCanvasContent({
          channelName: title,
          leadingContent,
          template: nextTemplate,
        });
        await setCanvas({
          channelId: chat.id,
          content: canvasContent ?? "",
        });
        await queryClient.invalidateQueries({
          queryKey: ["channel-canvas", chat.id],
        });

        if (nextProject) {
          onProjectCreated({
            ...nextProject,
            updatedAt: Math.floor(Date.now() / 1_000),
          });
        }
        toast.success(
          nextProject
            ? `Project set to ${nextProject.name}`
            : "Project removed",
        );
      } catch (error) {
        console.error("Failed to update chat project", error);
        toast.error("Could not update project", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [
      chat.id,
      chat.name,
      defaultAgent,
      metadata,
      onProjectCreated,
      projects,
      queryClient,
      templates,
      updateMetadataMutation,
    ],
  );
  const visibleMessages = React.useMemo(
    () =>
      messages.filter((message) => {
        if (message.kind === KIND_SYSTEM_MESSAGE) {
          return false;
        }
        const isAgent =
          defaultAgent?.pubkey != null &&
          normalizePubkey(message.pubkey) ===
            normalizePubkey(defaultAgent.pubkey);
        return (
          (eventHasTag(message, "chat_context", "source") ||
            (isAgent
              ? isHumanFacingAssistantText(message.content)
              : message.content.trim().length > 0)) &&
          !shouldHidePersistedAgentMessage({
            event: message,
            hiddenAgentMessageIds: chatActivity.hiddenAgentMessageIds,
          })
        );
      }),
    [chatActivity.hiddenAgentMessageIds, defaultAgent?.pubkey, messages],
  );
  const hasTranscriptActivity = chatActivity.totalBlockCount > 0;
  const latestVisibleMessage =
    visibleMessages.length > 0
      ? visibleMessages[visibleMessages.length - 1]
      : null;
  const latestVisibleMessageIsOwn =
    latestVisibleMessage != null &&
    identityPubkey != null &&
    normalizePubkey(latestVisibleMessage.pubkey) ===
      normalizePubkey(identityPubkey);
  const latestMessageActivityBlocks =
    latestVisibleMessage != null
      ? (chatActivity.blocksByMessageId.get(latestVisibleMessage.id) ?? [])
      : [];
  const latestOwnMessageNeedsAgent =
    latestVisibleMessageIsOwn &&
    latestMessageActivityBlocks.length === 0 &&
    !isChatTurnActive;
  const activationDelayKey =
    latestVisibleMessage != null
      ? `${latestVisibleMessage.id}:${scopedTranscript.length}`
      : "";
  const [showDelayedActivationCard, setShowDelayedActivationCard] =
    React.useState(false);
  React.useEffect(() => {
    if (!activationDelayKey || !latestOwnMessageNeedsAgent || !hasObserver) {
      setShowDelayedActivationCard(false);
      return;
    }

    const timeout = window.setTimeout(() => {
      setShowDelayedActivationCard(true);
    }, 1_200);
    return () => window.clearTimeout(timeout);
  }, [activationDelayKey, hasObserver, latestOwnMessageNeedsAgent]);
  const shouldShowAgentActivationCard =
    latestOwnMessageNeedsAgent && (!hasObserver || showDelayedActivationCard);
  const scrollSignature = React.useMemo(
    () =>
      [
        visibleMessages
          .map((message) => `${message.id}:${message.content.length}`)
          .join(","),
        scopedTranscript
          .map((item) => {
            if (item.type === "message") {
              return `${item.id}:message:${item.text.length}`;
            }
            if (item.type === "tool") {
              return `${item.id}:tool:${item.status}:${item.result.length}`;
            }
            return `${item.id}:${item.type}:${item.timestamp}`;
          })
          .join(","),
        shouldShowAgentActivationCard ? "activation-card" : "",
      ].join("|"),
    [scopedTranscript, shouldShowAgentActivationCard, visibleMessages],
  );
  const forceScrollSignature = latestVisibleMessageIsOwn
    ? latestVisibleMessage.id
    : null;

  return (
    <>
      <ChatHeader
        actions={shareAction}
        description={defaultAgent?.name ?? "Fizz"}
        mode="chats"
        title={metadata?.title || chat.name}
        transparentChrome
      />

      <MessageScrollerProvider
        autoScroll
        defaultScrollPosition="end"
        key={chat.id}
      >
        <MessageScroller className="bg-background">
          <MessageScrollerViewport aria-label="Chat messages">
            <MessageScrollerContent
              className={cn(CHAT_CONVERSATION_CLASS, "py-6")}
            >
              {isLoadingMessages ? (
                <MessageScrollerItem messageId="chat:loading">
                  <div className="flex items-center gap-2 px-5 py-1 text-sm text-muted-foreground">
                    <Spinner className="h-4 w-4" />
                    Loading messages
                  </div>
                </MessageScrollerItem>
              ) : visibleMessages.length === 0 && !hasTranscriptActivity ? (
                <MessageScrollerItem
                  className="flex flex-1 items-center justify-center"
                  messageId="chat:empty"
                >
                  <div className="px-8 py-12 text-center">
                    <MessageCircle className="mx-auto h-5 w-5 text-muted-foreground" />
                    <p className="mt-3 text-sm font-medium">No messages yet</p>
                    <p className="mt-1 text-sm text-muted-foreground">
                      Send a message and Fizz will respond.
                    </p>
                  </div>
                </MessageScrollerItem>
              ) : (
                <>
                  {visibleMessages.map((message) => {
                    const activityBlocks =
                      chatActivity.blocksByMessageId.get(message.id) ?? [];
                    const isContextMessage = eventHasTag(
                      message,
                      "chat_context",
                      "source",
                    );
                    const isAgentMessage =
                      defaultAgent?.pubkey != null &&
                      normalizePubkey(message.pubkey) ===
                        normalizePubkey(defaultAgent.pubkey);
                    const isOwnMessage =
                      identityPubkey != null &&
                      normalizePubkey(message.pubkey) ===
                        normalizePubkey(identityPubkey);

                    return (
                      <React.Fragment key={message.localKey ?? message.id}>
                        <MessageScrollerItem messageId={message.id}>
                          {isContextMessage ? (
                            <ChatContextRow event={message} />
                          ) : (
                            <ChatMessageRow
                              event={message}
                              isAgent={isAgentMessage}
                              isOwn={isOwnMessage}
                              profiles={profiles}
                            />
                          )}
                        </MessageScrollerItem>
                        {activityBlocks.length > 0 ? (
                          <MessageScrollerItem
                            messageId={`chat:activity:${message.id}`}
                          >
                            <ChatActivityTranscript
                              agent={defaultAgent}
                              blocks={activityBlocks}
                              identityPubkey={identityPubkey}
                              isTurnActive={isChatTurnActive}
                              profiles={profiles}
                            />
                          </MessageScrollerItem>
                        ) : null}
                        {shouldShowAgentActivationCard &&
                        latestVisibleMessage?.id === message.id ? (
                          <MessageScrollerItem
                            messageId={`chat:activate-agent:${message.id}`}
                          >
                            <AgentActivationCard
                              agentName={defaultAgent?.name ?? "Fizz"}
                              isActivating={isActivatingAgent}
                              onActivate={onActivateAgent}
                            />
                          </MessageScrollerItem>
                        ) : null}
                      </React.Fragment>
                    );
                  })}
                  {chatActivity.unplacedBlocks.length > 0 ? (
                    <MessageScrollerItem messageId="chat:activity:unplaced">
                      <ChatActivityTranscript
                        agent={defaultAgent}
                        blocks={chatActivity.unplacedBlocks}
                        identityPubkey={identityPubkey}
                        isTurnActive={isChatTurnActive}
                        profiles={profiles}
                      />
                    </MessageScrollerItem>
                  ) : null}
                </>
              )}
            </MessageScrollerContent>
          </MessageScrollerViewport>
          <MessageScrollerButton />
          <ChatScrollAnchor
            forceSignature={forceScrollSignature}
            signature={scrollSignature}
          />
        </MessageScroller>
      </MessageScrollerProvider>

      <div className="shrink-0 bg-background">
        <MessageComposer
          autoInviteNonMemberMentions
          channelId={chat.id}
          channelName={chat.name}
          channelType="chat"
          containerClassName={cn(CHAT_CONVERSATION_CLASS, "pb-3")}
          disabled={isSending}
          draftKey={`chat:${chat.id}`}
          isSending={isSending}
          onSend={onSend}
          placeholder="Message Fizz..."
          profiles={profiles}
          toolbarControls={{ emoji: false, formatting: false, spoiler: false }}
          toolbarExtraActions={
            <ProjectPicker
              isNoProjectSelected={!selectedProject && metadata !== null}
              onCreateProject={onProjectCreated}
              onSelectProject={handleSelectProject}
              projects={projects}
              selectedProject={selectedProject}
              templates={templates}
            />
          }
        />
      </div>
    </>
  );
}
