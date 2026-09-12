import { ArrowUp, Plus, X } from "lucide-react";
import { motion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import { useRenderScopedReactionHydration } from "@/features/messages/lib/useRenderScopedReactionHydration";
import type { TimelineMessage } from "@/features/messages/types";
import { TimelineMessageList } from "@/features/messages/ui/TimelineMessageList";
import { ProtectedMessageActionsBoundary } from "@protected-feature-components";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { useProfileQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, ManagedAgent, PresenceStatus } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { buildBestieMessageContext } from "./bestieMessageContext";
import { useBestie } from "./useBestie";

export function BestieTriggerVisual({
  agent,
  className,
  compact = false,
  imageDraggable,
}: {
  agent: ManagedAgent | null;
  className?: string;
  compact?: boolean;
  imageDraggable?: boolean;
}) {
  if (agent) {
    return (
      <UserAvatar
        avatarUrl={agent.avatarUrl}
        className={cn(compact ? "h-6 w-6" : "h-10 w-10", className)}
        displayName={agent.name}
        fallbackDelayMs={0}
        imageDraggable={imageDraggable}
        size={compact ? "sm" : "md"}
        testId="bestie-trigger-avatar"
      />
    );
  }

  return (
    <Plus
      aria-hidden="true"
      className={cn(compact ? "h-4 w-4" : "h-5 w-5", className)}
      data-testid="bestie-empty-mark"
    />
  );
}

export function BestieAgentLockup({
  agent,
  avatarLayoutId,
  presenceStatus,
  compact = false,
}: {
  agent: ManagedAgent;
  avatarLayoutId?: string;
  presenceStatus: PresenceStatus;
  compact?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <motion.div
        aria-hidden="true"
        className="relative shrink-0"
        data-testid="bestie-agent-avatar"
        layoutId={avatarLayoutId}
      >
        <UserAvatar
          avatarUrl={agent.avatarUrl}
          className={compact ? "h-5 w-5" : "h-8 w-8"}
          displayName={agent.name}
          fallbackDelayMs={0}
          size={compact ? "xs" : "sm"}
        />
        <span
          className={cn(
            "absolute flex items-center justify-center rounded-full",
            compact
              ? "-bottom-0.5 -right-0.5 h-2.5 w-2.5 bg-sidebar"
              : "-bottom-0.5 -right-0.5 h-3.5 w-3.5 bg-popover",
          )}
        >
          <PresenceDot
            className={compact ? "h-1.5 w-1.5" : "h-2 w-2"}
            data-testid="bestie-activity-dot"
            status={presenceStatus}
          />
        </span>
      </motion.div>
      <span className="min-w-0 truncate text-sm font-medium">{agent.name}</span>
      <span className="sr-only">{presenceStatus}</span>
    </div>
  );
}

function EmptyBestie() {
  return (
    <div className="flex min-h-32 flex-col items-center justify-center gap-3 px-4 text-center">
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Plus aria-hidden="true" className="h-5 w-5" />
      </div>
      <div>
        <h2 className="text-sm font-semibold">Choose a Bestie</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          Open one of your local agents and turn on Bestie.
        </p>
      </div>
    </div>
  );
}

function BestieConversationTranscript({
  channel,
  currentPubkey,
  messages,
  onToggleReaction,
  profiles,
}: {
  channel: Channel;
  currentPubkey: string | undefined;
  messages: TimelineMessage[];
  onToggleReaction: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles: UserProfileLookup;
}) {
  const transcriptRef = React.useRef<HTMLDivElement>(null);
  const latestMessageKey = messages.at(-1)?.renderKey ?? messages.at(-1)?.id;
  const mainTimelineEntries = React.useMemo(
    () => buildMainTimelineEntries(messages, undefined, undefined, profiles),
    [messages, profiles],
  );
  useRenderScopedReactionHydration({
    activeChannel: channel,
    mainTimelineEntries,
    threadHeadMessage: null,
    threadMessages: [],
  });

  React.useLayoutEffect(() => {
    if (!latestMessageKey) return;
    const transcript = transcriptRef.current;
    if (!transcript) return;
    transcript.scrollTop = transcript.scrollHeight;
  }, [latestMessageKey]);

  return (
    <div
      aria-live="polite"
      className="h-full min-h-0 max-h-48 overflow-y-auto"
      data-bestie-channel-id={channel.id}
      data-bestie-channel-name={channel.name}
      data-testid="bestie-mini-transcript"
      ref={transcriptRef}
    >
      <ProtectedMessageActionsBoundary>
        <TimelineMessageList
          channelId={channel.id}
          channelName={channel.name}
          channelType={channel.channelType}
          currentPubkey={currentPubkey}
          mainEntries={mainTimelineEntries}
          messages={messages}
          onToggleReaction={onToggleReaction}
          profiles={profiles}
          stickyDayDividers={false}
        />
      </ProtectedMessageActionsBoundary>
    </div>
  );
}

export function BestiePopover({
  avatarLayoutId,
  contextChannelId,
  contextMessage,
  onRequestClose,
}: {
  avatarLayoutId?: string;
  contextChannelId?: string | null;
  contextMessage?: TimelineMessage;
  onRequestClose?: () => void;
}) {
  const bestie = useBestie();
  const [draft, setDraft] = React.useState("");
  const [contextSent, setContextSent] = React.useState(false);
  const [conversationChannel, setConversationChannel] =
    React.useState<Channel | null>(null);
  const [sessionBoundary, setSessionBoundary] = React.useState<{
    baselineMessageIds: ReadonlySet<string>;
    firstMessageCreatedAt: number;
  } | null>(null);
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const conversationQuery = useChannelMessagesQuery(conversationChannel);
  useChannelSubscription(conversationChannel);
  const sendMutation = useSendMessageMutation(
    conversationChannel,
    identityQuery.data,
  );
  const toggleReactionMutation = useToggleReactionMutation();
  const toggleReactionMutateRef = React.useRef(
    toggleReactionMutation.mutateAsync,
  );
  toggleReactionMutateRef.current = toggleReactionMutation.mutateAsync;
  const agent = bestie.assignedAgent;
  const assignedAgentPubkey = agent?.pubkey;
  const conversationPromiseRef = React.useRef<Promise<Channel> | null>(null);
  const resolveConversationForOpen = React.useEffectEvent(() =>
    bestie.resolveConversation(),
  );

  React.useEffect(() => {
    setConversationChannel(null);
    conversationPromiseRef.current = null;
    if (!assignedAgentPubkey) return;

    let cancelled = false;
    const pending = resolveConversationForOpen();
    conversationPromiseRef.current = pending;
    void pending
      .then((channel) => {
        if (!cancelled) setConversationChannel(channel);
      })
      .catch((error) => {
        console.warn("Couldn’t load the Bestie conversation", error);
      })
      .finally(() => {
        if (conversationPromiseRef.current === pending) {
          conversationPromiseRef.current = null;
        }
      });
    return () => {
      cancelled = true;
    };
  }, [assignedAgentPubkey]);

  const currentPubkey = identityQuery.data?.pubkey;
  const currentProfile = profileQuery.data;
  const conversationProfiles = React.useMemo<UserProfileLookup>(() => {
    if (!agent) return {};
    const profiles: UserProfileLookup = {
      [normalizePubkey(agent.pubkey)]: {
        avatarUrl: agent.avatarUrl,
        displayName: agent.name,
        isAgent: true,
        name: agent.name,
        nip05Handle: null,
        ownerPubkey: null,
      },
    };
    if (currentPubkey) {
      profiles[normalizePubkey(currentPubkey)] = {
        avatarUrl: currentProfile?.avatarUrl ?? null,
        displayName: "You",
        isAgent: false,
        name: null,
        nip05Handle: currentProfile?.nip05Handle ?? null,
        ownerPubkey: null,
      };
    }
    return profiles;
  }, [
    agent,
    currentProfile?.avatarUrl,
    currentProfile?.nip05Handle,
    currentPubkey,
  ]);
  const contextEnvelope = React.useMemo(
    () => buildBestieMessageContext(contextChannelId, contextMessage),
    [contextChannelId, contextMessage],
  );
  const allConversationMessages = React.useMemo(() => {
    if (!conversationChannel) return [];
    return formatTimelineMessages(
      conversationQuery.data ?? [],
      conversationChannel,
      currentPubkey,
      currentProfile?.avatarUrl ?? null,
      conversationProfiles,
    )
      .filter(
        (message) =>
          (message.kind === KIND_STREAM_MESSAGE ||
            message.kind === KIND_STREAM_MESSAGE_V2) &&
          !message.parentId,
      )
      .map((message) => {
        if (!contextEnvelope || !message.body.startsWith(contextEnvelope)) {
          return message;
        }
        return {
          ...message,
          body: message.body.slice(contextEnvelope.length).trim(),
        };
      })
      .filter((message) => message.body.length > 0);
  }, [
    contextEnvelope,
    conversationChannel,
    conversationProfiles,
    conversationQuery.data,
    currentProfile?.avatarUrl,
    currentPubkey,
  ]);
  const conversationMessages = React.useMemo(() => {
    if (!sessionBoundary) return [];
    return allConversationMessages
      .filter(
        (message) =>
          message.createdAt >= sessionBoundary.firstMessageCreatedAt &&
          !sessionBoundary.baselineMessageIds.has(message.id),
      )
      .slice(-12);
  }, [allConversationMessages, sessionBoundary]);
  const handleToggleReaction = React.useCallback(
    async (message: TimelineMessage, emoji: string, remove: boolean) => {
      await toggleReactionMutateRef.current({
        emoji,
        eventId: message.id,
        remove,
      });
    },
    [],
  );
  if (bestie.isLoading) {
    return <p className="text-sm text-muted-foreground">Loading Bestie…</p>;
  }
  if (!agent) return <EmptyBestie />;

  const presenceStatus = bestie.presenceStatus ?? "offline";
  const sendMessage = () => {
    const trimmedDraft = draft.trim();
    if (!trimmedDraft || bestie.isOpening || sendMutation.isPending) return;
    void (async () => {
      const baselineMessageIds = new Set(
        allConversationMessages.map((message) => message.id),
      );
      const startResult = bestie.ensureAgentRunning().then(
        () => ({ error: null }),
        (error: unknown) => ({ error }),
      );
      const channel =
        conversationChannel ??
        (await (conversationPromiseRef.current ??
          bestie.resolveConversation()));
      setConversationChannel(channel);
      const sentMessage = await sendMutation.mutateAsync({
        content:
          contextEnvelope && !contextSent
            ? `${contextEnvelope}\n\n${trimmedDraft}`
            : trimmedDraft,
        targetChannel: channel,
      });
      setSessionBoundary(
        (current) =>
          current ?? {
            baselineMessageIds,
            firstMessageCreatedAt: sentMessage.created_at,
          },
      );
      setContextSent(true);
      setDraft("");
      const { error: startError } = await startResult;
      if (startError) throw startError;
    })().catch((error) => {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t message Bestie",
      );
    });
  };

  return (
    <div className="flex max-h-[min(32rem,var(--radix-popover-content-available-height,calc(100vh-2rem)))] flex-col gap-4">
      <div
        className="flex shrink-0 touch-none select-none items-center gap-3 cursor-grab active:cursor-grabbing"
        data-bestie-drag-handle
      >
        <BestieAgentLockup
          agent={agent}
          avatarLayoutId={avatarLayoutId}
          presenceStatus={presenceStatus}
        />
        <div className="flex-1" />
        <Button
          aria-label="Close Bestie"
          onClick={onRequestClose}
          size="icon-xs"
          variant="ghost"
        >
          <X />
        </Button>
      </div>

      {conversationMessages.length > 0 && conversationChannel ? (
        <div className="min-h-0 max-h-48 overflow-hidden">
          <BestieConversationTranscript
            channel={conversationChannel}
            currentPubkey={currentPubkey}
            messages={conversationMessages}
            onToggleReaction={handleToggleReaction}
            profiles={conversationProfiles}
          />
        </div>
      ) : null}

      {contextMessage && !contextSent ? (
        <div
          className="shrink-0 space-y-2"
          data-testid="bestie-message-context"
        >
          <div
            className="max-h-24 max-w-[75%] overflow-hidden rounded-xl border border-border/70 bg-muted/45 p-2.5 shadow-xs"
            data-testid="bestie-message-snapshot"
          >
            <div className="flex min-w-0 items-center gap-2">
              <UserAvatar
                avatarUrl={contextMessage.avatarUrl ?? null}
                className="h-5 w-5"
                displayName={contextMessage.author}
                fallbackDelayMs={0}
                size="xs"
              />
              <span className="truncate text-xs font-semibold">
                {contextMessage.author}
              </span>
            </div>
            <p className="mt-1.5 whitespace-pre-wrap break-words text-xs leading-4 text-foreground/80">
              {contextMessage.body}
            </p>
          </div>
          <div className="w-fit rounded-2xl bg-muted px-3 py-2 text-sm">
            How can I help?
          </div>
        </div>
      ) : null}

      <div className="relative shrink-0">
        <Textarea
          aria-label={`Message ${agent.name}`}
          className="min-h-24 resize-none rounded-2xl pb-11"
          data-bloom-autofocus
          data-testid="bestie-composer"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key !== "Enter" ||
              event.shiftKey ||
              event.altKey ||
              event.ctrlKey ||
              event.metaKey ||
              event.nativeEvent.isComposing
            ) {
              return;
            }
            event.preventDefault();
            sendMessage();
          }}
          placeholder={`Message ${agent.name}`}
          value={draft}
        />
        <Button
          aria-label="Send in Bestie conversation"
          className="absolute bottom-2 right-2 rounded-full"
          disabled={!draft.trim() || bestie.isOpening || sendMutation.isPending}
          onClick={sendMessage}
          size="icon"
        >
          <ArrowUp />
        </Button>
      </div>
    </div>
  );
}
