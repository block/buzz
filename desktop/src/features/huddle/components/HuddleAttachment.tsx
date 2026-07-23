import { useQueryClient } from "@tanstack/react-query";
import { Headphones, MessageSquareText } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { TimelineMessage } from "@/features/messages/types";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";
import { useHuddle } from "../HuddleContext";
import {
  HUDDLE_EVENT_HISTORY_LIMIT,
  type HuddleLifecycleState,
  huddleStalenessDelayMs,
  recordHuddleSubscriptionEvent,
  reconstructHuddleState,
} from "../lib/huddleLifecycleState";

type HuddleAttachmentProps = {
  channelId: string | null;
  className?: string;
  message: TimelineMessage;
  onOpenThread?: (message: TimelineMessage) => void;
};

function parseEphemeralChannelId(content: string): string | null {
  try {
    const parsed = JSON.parse(content) as { ephemeral_channel_id?: unknown };
    return typeof parsed.ephemeral_channel_id === "string"
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

function participantLabel(count: number) {
  return `${count} participant${count === 1 ? "" : "s"}`;
}

function messageLifecycleEvent(message: TimelineMessage): RelayEvent {
  return {
    id: message.id,
    pubkey: message.pubkey ?? "",
    kind: message.kind ?? KIND_HUDDLE_STARTED,
    created_at: message.createdAt,
    content: message.body,
    tags: message.tags ?? [],
    sig: "",
  };
}

export function HuddleAttachment({
  channelId,
  className,
  message,
  onOpenThread,
}: HuddleAttachmentProps) {
  const ephemeralChannelId = React.useMemo(
    () => parseEphemeralChannelId(message.body),
    [message.body],
  );
  const { activeEphemeralChannelId, isStarting, joinHuddle } = useHuddle();
  const isCurrentHuddle =
    Boolean(ephemeralChannelId) &&
    activeEphemeralChannelId === ephemeralChannelId;
  const queryClient = useQueryClient();
  const [isJoining, setIsJoining] = React.useState(false);
  const [lifecycleState, setLifecycleState] =
    React.useState<HuddleLifecycleState>(() =>
      ephemeralChannelId
        ? reconstructHuddleState(
            [messageLifecycleEvent(message)],
            ephemeralChannelId,
            { isCurrentHuddle },
          )
        : {
            ended: true,
            participants: new Set(),
            startCreatedAt: null,
            staleDeadlineMs: null,
          },
    );

  React.useEffect(() => {
    if (!channelId || !ephemeralChannelId) return;

    const huddleChannelId = ephemeralChannelId;
    let disposed = false;
    let cleanup: (() => void) | null = null;
    let staleTimeout: ReturnType<typeof setTimeout> | null = null;
    const seenChannelEventIds = new Set<string>();
    const seenEvents = new Map<string, RelayEvent>([
      [
        message.id,
        {
          id: message.id,
          pubkey: message.pubkey ?? "",
          kind: message.kind ?? KIND_HUDDLE_STARTED,
          created_at: message.createdAt,
          content: message.body,
          tags: message.tags ?? [],
          sig: "",
        },
      ],
    ]);

    function updateState() {
      if (disposed) return;
      if (staleTimeout) clearTimeout(staleTimeout);
      const state = reconstructHuddleState(
        seenEvents.values(),
        huddleChannelId,
        {
          historyMayBeTruncated:
            seenChannelEventIds.size >= HUDDLE_EVENT_HISTORY_LIMIT,
          isCurrentHuddle,
        },
      );
      setLifecycleState(state);
      const staleDelay = state.ended
        ? null
        : huddleStalenessDelayMs(state.staleDeadlineMs);
      if (staleDelay !== null)
        staleTimeout = setTimeout(updateState, staleDelay);
    }

    updateState();
    relayClient
      .subscribeToHuddleEvents(channelId, (event) => {
        if (
          disposed ||
          !recordHuddleSubscriptionEvent(
            seenChannelEventIds,
            seenEvents,
            huddleChannelId,
            event,
          )
        )
          return;
        updateState();
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error("[HuddleAttachment] subscription failed:", error);
      });

    return () => {
      disposed = true;
      if (staleTimeout) clearTimeout(staleTimeout);
      cleanup?.();
    };
  }, [
    channelId,
    ephemeralChannelId,
    isCurrentHuddle,
    message.body,
    message.createdAt,
    message.id,
    message.kind,
    message.pubkey,
    message.tags,
  ]);

  const participantCount = lifecycleState.participants.size;
  const isEnded = lifecycleState.ended;
  const canJoin = Boolean(
    channelId && ephemeralChannelId && !isEnded && !isCurrentHuddle,
  );
  const displayEnded = isEnded;

  async function handleJoin() {
    if (!channelId || !ephemeralChannelId || isJoining || isStarting) return;
    setIsJoining(true);
    try {
      await joinHuddle(channelId, ephemeralChannelId);
      void queryClient.invalidateQueries({ queryKey: ["channels"] });
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Failed to join huddle";
      toast.error(message);
    } finally {
      setIsJoining(false);
    }
  }

  if (!ephemeralChannelId) {
    return (
      <Attachment
        className={cn("w-96 max-w-full shadow-none", className)}
        data-testid="huddle-attachment"
        state="error"
      >
        <AttachmentMedia>
          <Headphones />
        </AttachmentMedia>
        <AttachmentContent>
          <AttachmentTitle>Huddle unavailable</AttachmentTitle>
          <AttachmentDescription>
            This huddle card is missing session details.
          </AttachmentDescription>
        </AttachmentContent>
      </Attachment>
    );
  }

  return (
    <Attachment
      className={cn("w-96 max-w-full shadow-none", className)}
      data-testid="huddle-attachment"
      data-huddle-state={displayEnded ? "ended" : "active"}
    >
      <AttachmentMedia
        className={cn(
          !displayEnded &&
            "bg-primary/10 text-primary ring-1 ring-primary/20 dark:bg-primary/15",
        )}
      >
        <Headphones />
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle>
          Huddle
          <span aria-hidden="true"> · </span>
          {displayEnded ? "Ended" : "In progress"}
        </AttachmentTitle>
        <AttachmentDescription>
          {participantLabel(participantCount)}
        </AttachmentDescription>
      </AttachmentContent>
      <AttachmentActions>
        {canJoin ? (
          <AttachmentAction
            disabled={isJoining || isStarting}
            onClick={() => void handleJoin()}
            size="sm"
            type="button"
            variant="secondary"
          >
            <Headphones className="h-4 w-4" />
            {isJoining || isStarting ? "Joining" : "Join"}
          </AttachmentAction>
        ) : onOpenThread ? (
          <AttachmentAction
            aria-label="View huddle thread"
            onClick={() => onOpenThread(message)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <MessageSquareText className="h-4 w-4" />
            View thread
          </AttachmentAction>
        ) : null}
      </AttachmentActions>
    </Attachment>
  );
}
