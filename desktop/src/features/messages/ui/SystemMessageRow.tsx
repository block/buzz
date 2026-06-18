import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import type { TimelineMessage } from "@/features/messages/types";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import { recordQuickReactionEmoji } from "@/features/messages/ui/useQuickReactionEmojis";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { isPositiveEmojiParticle } from "@/shared/ui/EmojiBurstProvider";
import {
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
  MENTION_CHIP_PREFIX_CLASS,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { MessageTimestamp } from "./MessageTimestamp";

const SYSTEM_ACTION_BUTTON_CLASS = "h-6 w-6 rounded-full p-0";
const SYSTEM_ACTION_ICON_CLASS = "!h-4 !w-4";

type SystemMessagePayload = {
  type: string;
  actor?: string;
  target?: string;
  topic?: string;
  purpose?: string;
};

type SystemMessageDescription = {
  action: React.ReactNode;
  title: React.ReactNode;
};

function resolveLabel(
  pubkey: string | undefined,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string {
  if (!pubkey) {
    return "Someone";
  }
  return resolveUserLabel({ pubkey, currentPubkey, profiles });
}

function resolveAvatarUrl(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string | null {
  if (!pubkey || !profiles) return null;
  return profiles[pubkey.toLowerCase()]?.avatarUrl ?? null;
}

function resolveDisplayLabel(
  pubkey: string | undefined,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string {
  return resolveLabel(pubkey, currentPubkey, profiles);
}

function ProfileName({
  children,
  highlight = false,
  isAgent = false,
  pubkey,
}: {
  children: React.ReactNode;
  highlight?: boolean;
  isAgent?: boolean;
  pubkey: string | undefined;
}) {
  const isAgentMention = highlight && isAgent;
  const node = (
    <span
      data-mention={highlight ? "" : undefined}
      className={cn(
        pubkey && "cursor-pointer",
        highlight
          ? cn(
              MENTION_CHIP_BASE_CLASSES,
              MENTION_CHIP_HOVER_CLASSES,
              isAgentMention && "agent-mention-highlight",
            )
          : "rounded-xs transition-colors hover:text-foreground",
      )}
    >
      {highlight && !isAgentMention ? (
        <span className={MENTION_CHIP_PREFIX_CLASS}>@</span>
      ) : null}
      {children}
    </span>
  );

  return pubkey ? (
    <UserProfilePopover pubkey={pubkey} triggerElement="span">
      {node}
    </UserProfilePopover>
  ) : (
    node
  );
}

function SystemMessageAvatar({
  actorPubkey,
  currentPubkey,
  profiles,
  targetPubkey,
}: {
  actorPubkey: string | undefined;
  currentPubkey: string | undefined;
  profiles: UserProfileLookup | undefined;
  targetPubkey: string | undefined;
}) {
  const hasActorAndTarget =
    actorPubkey && targetPubkey && actorPubkey !== targetPubkey;
  const actorLabel = actorPubkey
    ? resolveUserLabel({
        pubkey: actorPubkey,
        currentPubkey,
        profiles,
        preferResolvedSelfLabel: true,
      })
    : "Someone";

  const singlePubkey = actorPubkey ?? targetPubkey;

  if (!hasActorAndTarget) {
    const avatar = (
      <UserAvatar
        avatarUrl={resolveAvatarUrl(singlePubkey, profiles)}
        className="!h-9 !w-9 shrink-0 text-2xs"
        displayName={actorLabel}
        testId="system-message-avatar"
      />
    );

    if (singlePubkey) {
      return (
        <UserProfilePopover pubkey={singlePubkey}>
          <button
            className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            type="button"
          >
            {avatar}
          </button>
        </UserProfilePopover>
      );
    }

    return avatar;
  }

  const targetLabel = resolveUserLabel({
    pubkey: targetPubkey,
    currentPubkey,
    profiles,
    preferResolvedSelfLabel: true,
  });

  const dualAvatar = (
    <div
      className="relative h-9 w-9 shrink-0"
      data-testid="system-message-avatar"
    >
      <UserAvatar
        avatarUrl={resolveAvatarUrl(actorPubkey, profiles)}
        className="!h-7 !w-7 border-2 border-background text-2xs"
        displayName={actorLabel}
      />
      <UserAvatar
        avatarUrl={resolveAvatarUrl(targetPubkey, profiles)}
        className="!absolute !bottom-0 !right-0 !h-7 !w-7 border-2 border-background text-2xs"
        displayName={targetLabel}
      />
    </div>
  );

  return (
    <UserProfilePopover pubkey={actorPubkey}>
      <button
        className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        type="button"
      >
        {dualAvatar}
      </button>
    </UserProfilePopover>
  );
}

function describeSystemEvent(
  payload: SystemMessagePayload,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  personaLookup?: Map<string, string>,
  agentPubkeys?: ReadonlySet<string>,
): SystemMessageDescription | null {
  const isTargetAgent =
    payload.target !== undefined &&
    (agentPubkeys?.has(normalizePubkey(payload.target)) === true ||
      profiles?.[normalizePubkey(payload.target)]?.isAgent === true ||
      personaLookup?.has(normalizePubkey(payload.target)) === true);
  const actorLabel = resolveDisplayLabel(
    payload.actor,
    currentPubkey,
    profiles,
  );
  const targetLabel = resolveDisplayLabel(
    payload.target,
    currentPubkey,
    profiles,
  );
  const actorName = (
    <ProfileName pubkey={payload.actor}>{actorLabel}</ProfileName>
  );
  const targetName = (
    <ProfileName highlight isAgent={isTargetAgent} pubkey={payload.target}>
      {targetLabel}
    </ProfileName>
  );

  switch (payload.type) {
    case "member_joined": {
      if (payload.actor === payload.target) {
        return {
          title: targetName,
          action: "joined the channel",
        };
      }
      return {
        title: actorName,
        action: <>added {targetName} to the channel</>,
      };
    }
    case "member_left":
      return {
        title: actorName,
        action: "left the channel",
      };
    case "member_removed":
      return {
        title: actorName,
        action: <>removed {targetName} from the channel</>,
      };
    case "topic_changed":
      return {
        title: actorName,
        action: <>changed the topic to &ldquo;{payload.topic}&rdquo;</>,
      };
    case "purpose_changed":
      return {
        title: actorName,
        action: <>changed the purpose to &ldquo;{payload.purpose}&rdquo;</>,
      };
    case "channel_created":
      return {
        title: actorName,
        action: "created this channel",
      };
    case "channel_archived":
      return {
        title: actorName,
        action: "archived this channel",
      };
    case "channel_unarchived":
      return {
        title: actorName,
        action: "unarchived this channel",
      };
    default:
      return null;
  }
}

export const SystemMessageRow = React.memo(function SystemMessageRow({
  message,
  currentPubkey,
  agentPubkeys,
  profiles,
  personaLookup,
  onToggleReaction,
}: {
  message: TimelineMessage;
  currentPubkey?: string;
  agentPubkeys?: ReadonlySet<string>;
  profiles?: UserProfileLookup;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
  const [badgeBurstEmoji, setBadgeBurstEmoji] = React.useState<string | null>(
    null,
  );
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const {
    reactions,
    canToggle: canToggleReactions,
    pending: reactionPending,
    errorMessage: reactionErrorMessage,
    select: handleReactionSelect,
  } = useReactionHandler(message, onToggleReaction);

  let payload: SystemMessagePayload;
  try {
    payload = JSON.parse(message.body);
  } catch {
    return null;
  }

  const description = describeSystemEvent(
    payload,
    currentPubkey,
    profiles,
    personaLookup,
    agentPubkeys,
  );
  if (!description) {
    return null;
  }

  const wouldAddReaction = (emoji: string) =>
    !reactions.some(
      (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
    );

  return (
    <div
      className="group/message relative mx-1 rounded-2xl px-2 py-2 transition-colors hover:bg-muted/50 focus-within:bg-muted/50"
      data-testid="system-message-row"
    >
      <div className="flex items-start gap-2.5">
        <SystemMessageAvatar
          actorPubkey={payload.actor}
          currentPubkey={currentPubkey}
          profiles={profiles}
          targetPubkey={payload.target}
        />
        <div className={cn(MESSAGE_MARKDOWN_CLASS, "min-w-0 flex-1")}>
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <div className="truncate text-sm font-semibold leading-none tracking-tight text-foreground">
              {description.title}
            </div>
            <MessageTimestamp
              createdAt={message.createdAt}
              time={message.time}
            />
          </div>
          <p className="mt-1 text-sm leading-snug text-foreground">
            {description.action}
          </p>
          <div>
            <MessageReactions
              messageId={message.id}
              reactions={reactions}
              canToggle={canToggleReactions}
              pending={reactionPending}
              className="mt-0.5 pt-0.5"
              burstEmojiOnRender={badgeBurstEmoji}
              onBurstEmojiRendered={(emoji) => {
                setBadgeBurstEmoji((current) =>
                  current === emoji ? null : current,
                );
              }}
              onSelect={(emoji) => {
                void handleReactionSelect(emoji);
              }}
            />
            {reactionErrorMessage ? (
              <p className="mt-1.5 text-xs text-destructive">
                {reactionErrorMessage}
              </p>
            ) : null}
          </div>
        </div>
        <div className="absolute right-2 top-1 z-10 sm:top-0 sm:-translate-y-1/2">
          {canToggleReactions ? (
            <div
              className={cn(
                "overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-xs backdrop-blur-sm supports-[backdrop-filter]:bg-background/85 transition-all duration-150 ease-out",
                "max-w-0 border-0 shadow-none translate-y-1 opacity-0",
                "group-hover/message:max-w-9 group-hover/message:border group-hover/message:border-border/70 group-hover/message:shadow-xs group-hover/message:translate-y-0 group-hover/message:opacity-100",
                "group-focus-within/message:max-w-9 group-focus-within/message:border group-focus-within/message:border-border/70 group-focus-within/message:shadow-xs group-focus-within/message:translate-y-0 group-focus-within/message:opacity-100",
                isReactionPickerOpen
                  ? "max-w-9 border border-border/70 shadow-xs translate-y-0 opacity-100"
                  : "",
              )}
            >
              <div className="flex items-center gap-1 p-1">
                <Popover
                  onOpenChange={setIsReactionPickerOpen}
                  open={isReactionPickerOpen}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <PopoverTrigger asChild>
                        <Button
                          aria-label="Open reactions"
                          className={SYSTEM_ACTION_BUTTON_CLASS}
                          size="sm"
                          type="button"
                          variant={isReactionPickerOpen ? "secondary" : "ghost"}
                        >
                          <SmilePlus className={SYSTEM_ACTION_ICON_CLASS} />
                        </Button>
                      </PopoverTrigger>
                    </TooltipTrigger>
                    <TooltipContent>React</TooltipContent>
                  </Tooltip>
                  <PopoverContent
                    align="end"
                    className="w-auto p-0 rounded-2xl overflow-hidden border-0 bg-transparent shadow-none"
                    side="top"
                    sideOffset={10}
                  >
                    {reactionErrorMessage ? (
                      <div className="px-3 pt-3 pb-0">
                        <p className="text-xs text-destructive">
                          {reactionErrorMessage}
                        </p>
                      </div>
                    ) : null}
                    <EmojiPicker
                      onSelect={(value) => {
                        if (
                          !reactionPending &&
                          wouldAddReaction(value) &&
                          isPositiveEmojiParticle(value)
                        ) {
                          setBadgeBurstEmoji(value);
                        }
                        void handleReactionSelect(value)
                          .then(() => {
                            recordQuickReactionEmoji(value);
                          })
                          .catch(() => {})
                          .finally(() => {
                            setIsReactionPickerOpen(false);
                          });
                      }}
                    />
                  </PopoverContent>
                </Popover>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
});
