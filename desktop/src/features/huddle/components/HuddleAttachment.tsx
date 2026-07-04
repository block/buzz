import { useQueryClient } from "@tanstack/react-query";
import { Headphones, MessageSquareText } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { TimelineMessage } from "@/features/messages/types";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
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
import { isHuddleStartStale } from "../lib/huddleCardState";

type HuddleAttachmentProps = {
  channelId: string | null;
  className?: string;
  message: TimelineMessage;
  onOpenThread?: (message: TimelineMessage) => void;
};

type HuddleLifecycleState = {
  ended: boolean;
  participants: Set<string>;
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

function lifecycleEventChannelId(event: RelayEvent): string | null {
  return parseEphemeralChannelId(event.content);
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return (
    event.tags.find(
      (tag) => tag[0] === "p" && typeof tag[1] === "string",
    )?.[1] ??
    event.pubkey ??
    null
  );
}

function reconstructHuddleLifecycle(
  events: Iterable<RelayEvent>,
  fallbackCreatorPubkey: string | undefined,
  ephemeralChannelId: string,
): HuddleLifecycleState {
  const sorted = [...events]
    .filter((event) => lifecycleEventChannelId(event) === ephemeralChannelId)
    .sort(
      (left, right) =>
        left.created_at - right.created_at ||
        left.kind - right.kind ||
        left.id.localeCompare(right.id),
    );
  const participants = new Set<string>();
  let ended = false;

  if (fallbackCreatorPubkey) {
    participants.add(fallbackCreatorPubkey);
  }

  for (const event of sorted) {
    switch (event.kind) {
      case KIND_HUDDLE_STARTED:
        ended = false;
        if (event.pubkey) participants.add(event.pubkey);
        break;
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        if (ended) break;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.add(pubkey);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        if (ended) break;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.delete(pubkey);
        break;
      }
      case KIND_HUDDLE_ENDED:
        ended = true;
        break;
    }
  }

  return { ended, participants };
}

function participantLabel(count: number) {
  return `${count} participant${count === 1 ? "" : "s"}`;
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
  const queryClient = useQueryClient();
  const [isJoining, setIsJoining] = React.useState(false);
  const [lifecycleState, setLifecycleState] =
    React.useState<HuddleLifecycleState>(() => ({
      ended: false,
      participants: new Set(message.pubkey ? [message.pubkey] : []),
    }));

  React.useEffect(() => {
    if (!channelId || !ephemeralChannelId) return;

    const huddleChannelId = ephemeralChannelId;
    let disposed = false;
    let cleanup: (() => void) | null = null;
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
      setLifecycleState(
        reconstructHuddleLifecycle(
          seenEvents.values(),
          message.pubkey,
          huddleChannelId,
        ),
      );
    }

    updateState();
    relayClient
      .subscribeToHuddleEvents(channelId, (event) => {
        if (disposed || seenEvents.has(event.id)) return;
        if (lifecycleEventChannelId(event) !== huddleChannelId) return;
        seenEvents.set(event.id, event);
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
      cleanup?.();
    };
  }, [
    channelId,
    ephemeralChannelId,
    message.body,
    message.createdAt,
    message.id,
    message.kind,
    message.pubkey,
    message.tags,
  ]);

  const participantCount = Math.max(1, lifecycleState.participants.size);
  const isEnded = lifecycleState.ended;
  const isCurrentHuddle =
    Boolean(ephemeralChannelId) &&
    activeEphemeralChannelId === ephemeralChannelId;
  const isStaleUnconfirmedHuddle =
    !isCurrentHuddle && isHuddleStartStale(message.createdAt);
  const canJoin = Boolean(
    channelId &&
      ephemeralChannelId &&
      !isEnded &&
      !isCurrentHuddle &&
      !isStaleUnconfirmedHuddle,
  );
  const displayEnded = isEnded || isStaleUnconfirmedHuddle;

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
