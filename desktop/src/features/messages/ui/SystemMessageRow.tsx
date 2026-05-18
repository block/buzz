import { SmilePlus } from "lucide-react";
import Picker from "@emoji-mart/react";
import data from "@emoji-mart/data";
import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { MessageTimestamp } from "./MessageTimestamp";

type SystemMessagePayload = {
  type: string;
  actor?: string;
  target?: string;
  topic?: string;
  purpose?: string;
};

type SystemMessageDescription = {
  action: React.ReactNode;
  title: string;
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

function resolvePersonaSuffix(
  pubkey: string | undefined,
  personaLookup: Map<string, string> | undefined,
): string {
  if (!pubkey || !personaLookup) return "";
  const personaName = personaLookup.get(pubkey.toLowerCase());
  return personaName ? ` (${personaName})` : "";
}

function resolveAvatarUrl(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): string | null {
  if (!pubkey || !profiles) return null;
  return profiles[pubkey.toLowerCase()]?.avatarUrl ?? null;
}

function labelWithSuffix(
  pubkey: string | undefined,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  suffix = "",
): string {
  return `${resolveLabel(pubkey, currentPubkey, profiles)}${suffix}`;
}

function describeSystemEvent(
  payload: SystemMessagePayload,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  personaLookup?: Map<string, string>,
): SystemMessageDescription | null {
  const personaSuffix = resolvePersonaSuffix(payload.target, personaLookup);
  const actorLabel = labelWithSuffix(payload.actor, currentPubkey, profiles);
  const targetLabel = labelWithSuffix(
    payload.target,
    currentPubkey,
    profiles,
    personaSuffix,
  );

  switch (payload.type) {
    case "member_joined": {
      if (payload.actor === payload.target) {
        return {
          title: targetLabel,
          action: "joined the channel",
        };
      }
      return {
        title: actorLabel,
        action: (
          <>
            added <span className="font-medium">{targetLabel}</span> to the
            channel
          </>
        ),
      };
    }
    case "member_left":
      return {
        title: actorLabel,
        action: "left the channel",
      };
    case "member_removed":
      return {
        title: actorLabel,
        action: (
          <>
            removed <span className="font-medium">{targetLabel}</span> from the
            channel
          </>
        ),
      };
    case "topic_changed":
      return {
        title: actorLabel,
        action: <>changed the topic to &ldquo;{payload.topic}&rdquo;</>,
      };
    case "purpose_changed":
      return {
        title: actorLabel,
        action: <>changed the purpose to &ldquo;{payload.purpose}&rdquo;</>,
      };
    case "channel_created":
      return {
        title: actorLabel,
        action: "created this channel",
      };
    case "channel_archived":
      return {
        title: actorLabel,
        action: "archived this channel",
      };
    case "channel_unarchived":
      return {
        title: actorLabel,
        action: "unarchived this channel",
      };
    default:
      return null;
  }
}

export const SystemMessageRow = React.memo(function SystemMessageRow({
  message,
  currentPubkey,
  profiles,
  personaLookup,
  onToggleReaction,
}: {
  message: TimelineMessage;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
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
  );
  if (!description) {
    return null;
  }

  const avatarPubkey = payload.actor ?? payload.target;
  const avatarLabel = avatarPubkey
    ? resolveUserLabel({
        pubkey: avatarPubkey,
        currentPubkey,
        profiles,
        preferResolvedSelfLabel: true,
      })
    : "Someone";

  return (
    <div
      className="group/message relative rounded-2xl px-2 py-1 transition-colors"
      data-testid="system-message-row"
    >
      <div className="flex items-start gap-2.5">
        <UserAvatar
          avatarUrl={resolveAvatarUrl(avatarPubkey, profiles)}
          className="!h-9 !w-9 shrink-0 text-[10px]"
          displayName={avatarLabel}
          testId="system-message-avatar"
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <p className="truncate text-sm font-semibold leading-none tracking-tight text-foreground/90">
              {description.title}
            </p>
            <MessageTimestamp
              createdAt={message.createdAt}
              time={message.time}
            />
          </div>
          <p className="mt-1 text-sm leading-snug text-muted-foreground/70">
            {description.action}
          </p>
          <div>
            <MessageReactions
              messageId={message.id}
              reactions={reactions}
              canToggle={canToggleReactions}
              pending={reactionPending}
              className="mt-0.5 pt-0.5"
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
        <div className="absolute right-2 top-1 z-10">
          {canToggleReactions ? (
            <div
              className={cn(
                "overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-sm backdrop-blur supports-[backdrop-filter]:bg-background/85 transition-all duration-150 ease-out",
                "max-w-0 border-0 shadow-none translate-y-1 opacity-0",
                "group-hover/message:max-w-9 group-hover/message:border group-hover/message:border-border/70 group-hover/message:shadow-sm group-hover/message:translate-y-0 group-hover/message:opacity-100",
                "group-focus-within/message:max-w-9 group-focus-within/message:border group-focus-within/message:border-border/70 group-focus-within/message:shadow-sm group-focus-within/message:translate-y-0 group-focus-within/message:opacity-100",
                isReactionPickerOpen
                  ? "max-w-9 border border-border/70 shadow-sm translate-y-0 opacity-100"
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
                          className="h-6 w-6 rounded-full p-0"
                          disabled={reactionPending}
                          size="sm"
                          type="button"
                          variant={isReactionPickerOpen ? "secondary" : "ghost"}
                        >
                          {reactionPending ? (
                            <Spinner className="h-3 w-3" />
                          ) : (
                            <SmilePlus className="h-3 w-3" />
                          )}
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
                    <Picker
                      data={data}
                      onEmojiSelect={(emoji: { native: string }) => {
                        void handleReactionSelect(emoji.native).finally(() => {
                          setIsReactionPickerOpen(false);
                        });
                      }}
                      theme="auto"
                      previewPosition="none"
                      skinTonePosition="search"
                      set="native"
                      maxFrequentRows={2}
                      perLine={8}
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
