import * as React from "react";
import { ArrowLeft, Bot, createLucideIcon } from "lucide-react";
import { toast } from "sonner";

import {
  buildAgentConversationMentionPubkeys,
  buildAgentConversationMarkers,
  buildAgentConversationRecap,
  deriveAgentConversationTitle,
  getAutoRoutedAgentConversationPubkeys,
  type AgentConversation,
  publishAgentConversationMarker,
} from "@/features/agents/agentConversations";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useAppShell } from "@/app/AppShellContext";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import {
  collectMessageAuthorPubkeys,
  collectMessageMentionPubkeys,
  formatTimelineMessages,
} from "@/features/messages/lib/formatTimelineMessages";
import {
  buildKnownAgentParticipants,
  collectTimelineMessageAuthorPubkeys,
  flattenConversationMessages,
  formatAgentMentionList,
  formatAgentParticipantNames,
  getKnownAgentPubkeysInMessages,
  getLatestRelayMessageEvent,
  isConversationMessage,
  normalizeRecapTextForComparison,
  stripAgentStatusReactions,
  uniqueMessages,
  type AgentConversationParticipant,
} from "./AgentConversationScreen.helpers";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { useChannelTyping } from "@/features/messages/useChannelTyping";
import {
  MessageAuthorText,
  MessageHeaderRow,
} from "@/features/messages/ui/MessageHeader";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { MessageTimeline } from "@/features/messages/ui/MessageTimeline";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel, Identity, Profile } from "@/shared/api/types";
import { channelContentTopPaddingMeasurement } from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Shimmer } from "@/shared/ui/Shimmer";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const Summary = createLucideIcon("Summary", [
  ["path", { d: "M15 4H7", key: "summary-heading" }],
  ["path", { d: "m18 16 3 3-3 3", key: "summary-arrow" }],
  ["path", { d: "M3 4v13a2 2 0 0 0 2 2h16", key: "summary-page" }],
  ["path", { d: "M7 14h7", key: "summary-line-short" }],
  ["path", { d: "M7 9h12", key: "summary-line-long" }],
]);

type AgentConversationScreenProps = {
  channel: Channel | null;
  conversation: AgentConversation;
  currentIdentity?: Identity;
  currentProfile?: Profile;
  onBackToThread?: (conversation: AgentConversation) => void;
};

function AgentThinkingRow({
  agentName,
  avatarUrl,
}: {
  agentName: string;
  avatarUrl: string | null;
}) {
  return (
    <article
      aria-live="polite"
      className="group/message relative z-10 mx-1 flex items-start gap-2.5 rounded-2xl px-2 py-2 transition-colors hover:bg-muted/50 focus-within:bg-muted/50"
      data-testid="agent-conversation-thinking-row"
    >
      <UserAvatar
        accent
        avatarUrl={avatarUrl}
        className="!h-10 !w-10 shrink-0"
        displayName={agentName}
        testId="message-avatar"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <MessageHeaderRow>
          <MessageAuthorText>{agentName}</MessageAuthorText>
        </MessageHeaderRow>
        <p className="-mt-0.5 max-w-full text-sm text-foreground">
          <Shimmer className="align-baseline">Thinking...</Shimmer>
        </p>
      </div>
    </article>
  );
}

export function AgentConversationScreen({
  channel,
  conversation,
  currentIdentity,
  currentProfile,
  onBackToThread,
}: AgentConversationScreenProps) {
  const screenRef = React.useRef<HTMLElement | null>(null);
  const timelineScrollRef = React.useRef<HTMLDivElement>(null);
  const composerWrapperRef = React.useRef<HTMLDivElement>(null);
  const media = useMediaUpload();
  const messagesQuery = useChannelMessagesQuery(channel);
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  useChannelSubscription(channel);
  const sendMessageMutation = useSendMessageMutation(channel, currentIdentity);

  const relayMessages = messagesQuery.data ?? [];
  const {
    getMessageReadAt,
    isThreadMuted,
    markThreadRead,
    markMessageRead,
    updateAgentConversationTitle,
  } = useAppShell();
  const latestMessageEvent = React.useMemo(
    () => getLatestRelayMessageEvent(relayMessages),
    [relayMessages],
  );
  const typingEntries = useChannelTyping(
    channel,
    currentIdentity?.pubkey,
    latestMessageEvent,
  );
  const knownAgentParticipants = React.useMemo(
    () =>
      buildKnownAgentParticipants({
        conversation,
        managedAgents: managedAgentsQuery.data,
        relayAgents: relayAgentsQuery.data,
      }),
    [conversation, managedAgentsQuery.data, relayAgentsQuery.data],
  );
  const profilePubkeys = React.useMemo(
    () =>
      [
        ...new Set([
          ...collectMessageAuthorPubkeys(relayMessages),
          ...collectMessageMentionPubkeys(relayMessages),
          ...collectTimelineMessageAuthorPubkeys(conversation.contextMessages),
          ...collectMessageMentionPubkeys([...conversation.contextMessages]),
          ...typingEntries.map((entry) => entry.pubkey),
          conversation.agentPubkey,
          currentIdentity?.pubkey ?? "",
        ]),
      ].filter(Boolean),
    [
      conversation.agentPubkey,
      conversation.contextMessages,
      currentIdentity?.pubkey,
      relayMessages,
      typingEntries,
    ],
  );
  const profilesQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });
  const profiles = React.useMemo(
    () =>
      mergeCurrentProfileIntoLookup(
        profilesQuery.data?.profiles,
        currentProfile,
      ) ?? {},
    [currentProfile, profilesQuery.data?.profiles],
  );

  const knownAgentPubkeys = React.useMemo(
    () => new Set(knownAgentParticipants.keys()),
    [knownAgentParticipants],
  );
  const conversationSourceMessages = React.useMemo(() => {
    if (!channel || relayMessages.length === 0) {
      return uniqueMessages(
        conversation.contextMessages.length > 0
          ? conversation.contextMessages
          : ([
              conversation.threadRootMessage,
              conversation.parentMessage,
              conversation.agentReply,
            ].filter(Boolean) as TimelineMessage[]),
      ).map((message) => stripAgentStatusReactions(message, knownAgentPubkeys));
    }

    const formatted = formatTimelineMessages(
      relayMessages,
      channel,
      currentIdentity?.pubkey,
      currentProfile?.avatarUrl ?? null,
      profiles,
    );
    const scoped = formatted.filter((message) =>
      isConversationMessage(message, conversation),
    );
    const sourceMessages =
      scoped.length > 0
        ? scoped
        : uniqueMessages(
            conversation.contextMessages.length > 0
              ? conversation.contextMessages
              : ([
                  conversation.threadRootMessage,
                  conversation.parentMessage,
                  conversation.agentReply,
                ].filter(Boolean) as TimelineMessage[]),
          );

    return sourceMessages.map((message) =>
      stripAgentStatusReactions(message, knownAgentPubkeys),
    );
  }, [
    channel,
    conversation,
    currentIdentity?.pubkey,
    currentProfile?.avatarUrl,
    knownAgentPubkeys,
    profiles,
    relayMessages,
  ]);
  const timelineMessages = React.useMemo(
    () => flattenConversationMessages(conversationSourceMessages),
    [conversationSourceMessages],
  );

  const conversationAgentPubkeys = React.useMemo(() => {
    const pubkeys = getKnownAgentPubkeysInMessages(
      conversationSourceMessages,
      knownAgentParticipants,
    );
    if (
      !pubkeys.some(
        (pubkey) =>
          normalizePubkey(pubkey) === normalizePubkey(conversation.agentPubkey),
      )
    ) {
      pubkeys.unshift(conversation.agentPubkey);
    }

    return pubkeys;
  }, [
    conversation.agentPubkey,
    conversationSourceMessages,
    knownAgentParticipants,
  ]);
  const agentPubkeys = React.useMemo(
    () =>
      new Set(
        conversationAgentPubkeys.map((pubkey) => normalizePubkey(pubkey)),
      ),
    [conversationAgentPubkeys],
  );
  const typingAgentPubkeys = React.useMemo(() => {
    const latestMessage = timelineMessages[timelineMessages.length - 1] ?? null;
    const latestMessagePubkey = latestMessage?.pubkey
      ? normalizePubkey(latestMessage.pubkey)
      : null;
    const pubkeys: string[] = [];
    for (const entry of typingEntries) {
      const normalized = normalizePubkey(entry.pubkey);
      if (
        entry.threadHeadId !== conversation.threadRootId ||
        !agentPubkeys.has(normalized) ||
        latestMessagePubkey === normalized ||
        pubkeys.some((pubkey) => normalizePubkey(pubkey) === normalized)
      ) {
        continue;
      }

      pubkeys.push(
        knownAgentParticipants.get(normalized)?.pubkey ?? entry.pubkey,
      );
    }

    return pubkeys;
  }, [
    conversation.threadRootId,
    agentPubkeys,
    knownAgentParticipants,
    timelineMessages,
    typingEntries,
  ]);
  const agentParticipants = React.useMemo<AgentConversationParticipant[]>(
    () =>
      conversationAgentPubkeys.map((pubkey) => {
        const normalized = normalizePubkey(pubkey);
        const knownAgent = knownAgentParticipants.get(normalized);
        const profile = profiles[normalized];

        return {
          avatarUrl: profile?.avatarUrl ?? null,
          canMessage: knownAgent?.canMessage ?? true,
          displayName:
            profile?.displayName?.trim() ||
            knownAgent?.displayName ||
            (normalized === normalizePubkey(conversation.agentPubkey)
              ? conversation.agentName
              : pubkey),
          pubkey: knownAgent?.pubkey ?? pubkey,
        };
      }),
    [
      conversation.agentName,
      conversation.agentPubkey,
      conversationAgentPubkeys,
      knownAgentParticipants,
      profiles,
    ],
  );
  const typingAgentParticipants = React.useMemo(
    () =>
      typingAgentPubkeys
        .map((pubkey) => {
          const normalized = normalizePubkey(pubkey);
          return agentParticipants.find(
            (participant) => normalizePubkey(participant.pubkey) === normalized,
          );
        })
        .filter(
          (participant): participant is AgentConversationParticipant =>
            participant != null,
        ),
    [agentParticipants, typingAgentPubkeys],
  );
  const participantSubtitle = React.useMemo(
    () => formatAgentParticipantNames(agentParticipants),
    [agentParticipants],
  );
  const lastTitlePublishKeyRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    const threadRootMessage =
      conversationSourceMessages.find(
        (message) => message.id === conversation.threadRootId,
      ) ??
      conversation.threadRootMessage ??
      null;
    const parentMessage =
      conversation.agentReply.parentId != null
        ? (conversationSourceMessages.find(
            (message) => message.id === conversation.agentReply.parentId,
          ) ??
          conversation.parentMessage ??
          null)
        : (conversation.parentMessage ?? null);
    const derivedTitle = deriveAgentConversationTitle({
      agentPubkey: conversation.agentPubkey,
      agentReply: conversation.agentReply,
      contextMessages: conversationSourceMessages,
      parentMessage,
      threadRootId: conversation.threadRootId,
      threadRootMessage,
    });

    if (derivedTitle.status !== "resolved") {
      return;
    }
    if (
      conversation.titleStatus === derivedTitle.status &&
      conversation.title === derivedTitle.title
    ) {
      return;
    }

    const latestContextMessage =
      conversationSourceMessages[conversationSourceMessages.length - 1] ?? null;
    const publishKey = `${conversation.id}:${derivedTitle.status}:${derivedTitle.title}:${latestContextMessage?.id ?? "none"}`;
    if (lastTitlePublishKeyRef.current === publishKey) {
      return;
    }
    lastTitlePublishKeyRef.current = publishKey;

    updateAgentConversationTitle(
      conversation.id,
      derivedTitle.title,
      derivedTitle.status,
    );
    void publishAgentConversationMarker({
      agentName: conversation.agentName,
      agentPubkey: conversation.agentPubkey,
      agentReply: conversation.agentReply,
      channel: {
        id: conversation.channelId,
        name: conversation.channelName,
      },
      contextMessages: conversationSourceMessages,
      parentMessage,
      threadRootMessage,
    }).catch((error) => {
      console.warn("[agentConversations] title marker publish failed:", error);
    });
  }, [conversation, conversationSourceMessages, updateAgentConversationTitle]);
  React.useEffect(() => {
    const latestMessage = timelineMessages[timelineMessages.length - 1] ?? null;
    if (latestMessage) {
      markThreadRead(conversation.threadRootId, latestMessage.createdAt);
    }

    if (isThreadMuted(conversation.threadRootId)) {
      return;
    }

    for (const message of timelineMessages) {
      const readAt = getMessageReadAt(message.id);
      if (readAt === null || readAt < message.createdAt) {
        markMessageRead(message.id, message.createdAt);
      }
    }
  }, [
    conversation.threadRootId,
    getMessageReadAt,
    isThreadMuted,
    markMessageRead,
    markThreadRead,
    timelineMessages,
  ]);
  const routeableAgentPubkeys = React.useMemo(
    () =>
      agentParticipants
        .filter((participant) => participant.canMessage)
        .map((participant) => participant.pubkey),
    [agentParticipants],
  );
  const autoRoutedAgentPubkeys = React.useMemo(
    () => getAutoRoutedAgentConversationPubkeys(agentParticipants),
    [agentParticipants],
  );
  const canMessageAnyAgent = routeableAgentPubkeys.length > 0;
  const restrictedAgentNames = React.useMemo(
    () =>
      agentParticipants
        .filter((participant) => !participant.canMessage)
        .map((participant) => participant.displayName),
    [agentParticipants],
  );
  const restrictedAgentLabel = React.useMemo(
    () => formatAgentMentionList(restrictedAgentNames),
    [restrictedAgentNames],
  );
  const composerPlaceholder = React.useMemo(() => {
    if (!canMessageAnyAgent) {
      return "Reply to conversation";
    }
    if (agentParticipants.length === 1) {
      return `Message ${agentParticipants[0]?.displayName ?? "agent"}`;
    }

    return "Message conversation";
  }, [agentParticipants, canMessageAnyAgent]);
  const emptyDescription =
    agentParticipants.length === 1
      ? "Send a message below to keep working with this agent on the topic."
      : "Send a message below to keep working with these agents on the topic.";
  const [isPublishingThreadSummary, setIsPublishingThreadSummary] =
    React.useState(false);
  const lastPublishedThreadRecapRef = React.useRef<string | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset the cached recap when switching conversations.
  React.useEffect(() => {
    lastPublishedThreadRecapRef.current = null;
  }, [conversation.id]);
  const agentConversationMarkers = React.useMemo(
    () => buildAgentConversationMarkers(relayMessages),
    [relayMessages],
  );
  const currentConversationMarker = React.useMemo(
    () =>
      agentConversationMarkers.find(
        (marker) =>
          marker.channelId === conversation.channelId &&
          marker.agentReplyId === conversation.agentReply.id,
      ) ?? null,
    [
      agentConversationMarkers,
      conversation.agentReply.id,
      conversation.channelId,
    ],
  );
  const generatedThreadRecap = React.useMemo(
    () =>
      buildAgentConversationRecap({
        agentPubkeys,
        conversationTitle: conversation.title,
        messages: timelineMessages,
      }),
    [agentPubkeys, conversation.title, timelineMessages],
  );
  const primaryRecapAgent = agentParticipants[0] ?? null;
  const latestPublishedRecap =
    currentConversationMarker?.summary ??
    lastPublishedThreadRecapRef.current ??
    null;
  const headerChromeRef = useMeasuredCssVariable({
    targetRef: screenRef,
    ...channelContentTopPaddingMeasurement,
    resetKey: conversation.id,
  });
  useComposerHeightPadding(
    timelineScrollRef,
    composerWrapperRef,
    conversation.id,
    16,
  );

  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      await sendMessageMutation.mutateAsync({
        content,
        mediaTags,
        mentionPubkeys: buildAgentConversationMentionPubkeys({
          autoRouteAgentPubkeys: autoRoutedAgentPubkeys,
          mentionPubkeys,
        }),
        parentEventId: conversation.threadRootId,
      });
    },
    [autoRoutedAgentPubkeys, conversation.threadRootId, sendMessageMutation],
  );

  const isComposerDisabled =
    !channel?.isMember ||
    channel.archivedAt !== null ||
    sendMessageMutation.isPending;
  const canSendThreadSummary =
    Boolean(channel?.isMember) &&
    channel?.archivedAt === null &&
    !isPublishingThreadSummary &&
    generatedThreadRecap !== null;
  const markerThreadRootMessage = React.useMemo(
    () =>
      conversationSourceMessages.find(
        (message) => message.id === conversation.threadRootId,
      ) ??
      conversation.threadRootMessage ??
      null,
    [
      conversation.threadRootId,
      conversation.threadRootMessage,
      conversationSourceMessages,
    ],
  );
  const markerParentMessage = React.useMemo(() => {
    if (conversation.agentReply.parentId == null) {
      return conversation.parentMessage ?? null;
    }

    return (
      conversationSourceMessages.find(
        (message) => message.id === conversation.agentReply.parentId,
      ) ??
      conversation.parentMessage ??
      null
    );
  }, [
    conversation.agentReply.parentId,
    conversation.parentMessage,
    conversationSourceMessages,
  ]);
  const handleSendSummaryToThread = React.useCallback(async () => {
    if (!canSendThreadSummary || !generatedThreadRecap) {
      return;
    }

    const nextRecap = generatedThreadRecap.trim();
    if (
      normalizeRecapTextForComparison(nextRecap) ===
      normalizeRecapTextForComparison(latestPublishedRecap)
    ) {
      toast.info("Recap is already up to date");
      return;
    }

    setIsPublishingThreadSummary(true);
    try {
      await publishAgentConversationMarker(
        {
          agentName: conversation.agentName,
          agentPubkey: conversation.agentPubkey,
          agentReply: conversation.agentReply,
          channel: {
            id: conversation.channelId,
            name: conversation.channelName,
          },
          contextMessages: conversationSourceMessages,
          parentMessage: markerParentMessage,
          threadRootMessage: markerThreadRootMessage,
        },
        {
          summary: nextRecap,
          summaryAuthorName:
            primaryRecapAgent?.displayName ?? conversation.agentName,
          summaryAuthorPubkey:
            primaryRecapAgent?.pubkey ?? conversation.agentPubkey,
          summaryCreatedAt: Math.floor(Date.now() / 1_000),
        },
      );
      lastPublishedThreadRecapRef.current = nextRecap;
      toast.success(
        latestPublishedRecap
          ? "Updated recap in thread"
          : "Added recap to thread",
      );
    } catch (error) {
      console.error("[agentConversations] failed to publish recap:", error);
      toast.error("Failed to add recap to thread");
    } finally {
      setIsPublishingThreadSummary(false);
    }
  }, [
    canSendThreadSummary,
    conversation.agentName,
    conversation.agentPubkey,
    conversation.agentReply,
    conversation.channelId,
    conversation.channelName,
    conversationSourceMessages,
    generatedThreadRecap,
    latestPublishedRecap,
    markerThreadRootMessage,
    markerParentMessage,
    primaryRecapAgent?.displayName,
    primaryRecapAgent?.pubkey,
  ]);
  const headerActions = (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label="Send recap to thread"
          className="h-8 gap-1.5 px-2.5 text-sm font-medium"
          data-testid="agent-conversation-send-summary"
          disabled={!canSendThreadSummary}
          onClick={() => void handleSendSummaryToThread()}
          title="Send recap to thread"
          type="button"
          variant="outline"
        >
          <Summary className="h-4 w-4" />
          <span>
            {isPublishingThreadSummary
              ? "Generating recap..."
              : "Send recap to thread"}
          </span>
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        Add a conversation recap to the original thread
      </TooltipContent>
    </Tooltip>
  );
  const headerLeadingContent = onBackToThread ? (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label="Back to source thread"
          className="h-8 w-8 text-muted-foreground hover:text-foreground"
          data-testid="agent-conversation-back-to-thread"
          onClick={() => onBackToThread(conversation)}
          title="Back to source thread"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Back to source thread</TooltipContent>
    </Tooltip>
  ) : (
    false
  );

  return (
    <section
      aria-label={`Conversation: ${conversation.title}`}
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background"
      data-testid="agent-conversation-screen"
      ref={screenRef}
    >
      <ChatHeader
        actions={headerActions}
        animatedTitle
        animatedTitleResetKey={conversation.id}
        belowSystemChrome
        chromeWrapperRef={headerChromeRef}
        channelType="dm"
        compactTitleStack
        leadingContent={headerLeadingContent}
        leadingContentContainerClassName="-ml-2 mr-0.5"
        leadingContentLayout="side"
        title={conversation.title}
      />

      <MessageTimeline
        agentPubkeys={agentPubkeys}
        channelId={channel?.id ?? conversation.channelId}
        channelIntro={{
          channelKindLabel: "agent conversation",
          channelName: conversation.title,
          description: participantSubtitle,
          icon: <Bot aria-hidden className="h-7 w-7" />,
        }}
        channelName={channel?.name ?? conversation.channelName}
        channelType={channel?.channelType ?? "stream"}
        contentTopPadding="chrome"
        currentPubkey={currentIdentity?.pubkey}
        emptyDescription={emptyDescription}
        emptyTitle="No conversation messages yet"
        hasComposerOverlay
        isLoading={messagesQuery.isLoading && timelineMessages.length === 0}
        messageListPlacement="top"
        messages={timelineMessages}
        profiles={profiles}
        scrollContainerRef={timelineScrollRef}
        showInitialDayDivider={false}
        trailingContent={
          typingAgentParticipants.length > 0
            ? typingAgentParticipants.map((participant) => (
                <AgentThinkingRow
                  agentName={participant.displayName}
                  avatarUrl={participant.avatarUrl}
                  key={participant.pubkey}
                />
              ))
            : null
        }
      />

      <div
        className="pointer-events-none absolute inset-x-0 bottom-0 z-20"
        ref={composerWrapperRef}
      >
        <div className="pointer-events-auto">
          <MessageComposer
            channelId={channel?.id ?? conversation.channelId}
            channelName={channel?.name ?? conversation.channelName}
            channelType={channel?.channelType ?? "stream"}
            containerClassName="px-5"
            disabled={isComposerDisabled}
            draftKey={`agent-conversation:${conversation.id}`}
            isSending={sendMessageMutation.isPending}
            mediaController={media}
            onSend={handleSend}
            placeholder={composerPlaceholder}
            composerNotice={
              restrictedAgentNames.length === 0 ? null : (
                <div className="min-w-0">
                  <p className="truncate font-medium text-foreground">
                    You can view and reply to this conversation.
                  </p>
                  <p className="truncate text-muted-foreground/80">
                    You can&apos;t message{" "}
                    <span className="font-medium text-foreground">
                      {restrictedAgentLabel}
                    </span>
                    .
                  </p>
                </div>
              )
            }
            profiles={profiles}
            showTopBorder={false}
            typingParentEventId={conversation.threadRootId}
            typingRootEventId={conversation.threadRootId}
          />
          <div className="h-7 bg-background px-5 pb-1 pt-0" />
        </div>
      </div>
    </section>
  );
}
