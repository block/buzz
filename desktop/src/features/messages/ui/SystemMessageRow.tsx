import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import { recordQuickReactionEmoji } from "@/features/messages/ui/useQuickReactionEmojis";
import {
  formatOwnerLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
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
import { MessageAgentOwner } from "./MessageAgentOwner";
import { MessageAuthorText, MessageHeaderRow } from "./MessageHeader";
import { MessageTimestamp } from "./MessageTimestamp";

const SYSTEM_ACTION_BUTTON_CLASS = "h-6 w-6 rounded-full p-0";
const SYSTEM_ACTION_ICON_CLASS = "!h-4 !w-4";

type SystemMessagePayload = {
  type: string;
  actor?: string;
  target?: string;
  targets?: string[];
  topic?: string;
  purpose?: string;
  // Moderation tombstone fields (kind:40099 "message_deleted"). All optional and
  // moderator-authored — present when a moderator removed the message, absent for
  // a plain member self-delete. Reporter identity/evidence never appears here.
  public_reason?: string;
  reason_code?: string;
  action_id?: string;
};

type SystemMessageDescription = {
  action: React.ReactNode;
  title: React.ReactNode;
};

const MAX_VISIBLE_ADDITIONAL_MEMBER_NAMES = 3;

function parseSystemMessagePayload(
  message: TimelineMessage,
): SystemMessagePayload | null {
  try {
    return JSON.parse(message.body) as SystemMessagePayload;
  } catch {
    return null;
  }
}

function buildGroupedMembershipPayload(
  messages: readonly TimelineMessage[],
): SystemMessagePayload | null {
  if (messages.length < 2) return null;

  const payloads = messages.map(parseSystemMessagePayload);
  const firstPayload = payloads[0];
  const actor = firstPayload?.actor
    ? normalizePubkey(firstPayload.actor)
    : null;
  const firstTarget = firstPayload?.target
    ? normalizePubkey(firstPayload.target)
    : null;
  if (!actor || !firstTarget) return null;
  const isSelfJoinGroup = actor === firstTarget;

  const targets: string[] = [];
  for (const payload of payloads) {
    const payloadActor = payload?.actor ? normalizePubkey(payload.actor) : null;
    const payloadTarget = payload?.target
      ? normalizePubkey(payload.target)
      : null;
    if (
      payload?.type !== "member_joined" ||
      !payloadActor ||
      !payloadTarget ||
      (isSelfJoinGroup
        ? payloadActor !== payloadTarget
        : payloadActor !== actor || payloadActor === payloadTarget)
    ) {
      return null;
    }
    targets.push(payloadTarget);
  }

  if (isSelfJoinGroup) {
    return {
      type: "members_joined",
      target: targets[0],
      targets,
    };
  }

  return {
    type: "members_added",
    actor,
    target: targets[0],
    targets,
  };
}

function aggregateGroupedReactions(
  messages: readonly TimelineMessage[],
): TimelineReaction[] {
  const reactionsByEmoji = new Map<
    string,
    TimelineReaction & {
      usersByKey: Map<string, TimelineReaction["users"][number]>;
    }
  >();

  for (const message of messages) {
    for (const reaction of message.reactions ?? []) {
      const existing = reactionsByEmoji.get(reaction.emoji) ?? {
        emoji: reaction.emoji,
        emojiUrl: reaction.emojiUrl,
        count: 0,
        reactedByCurrentUser: false,
        users: [],
        usersByKey: new Map(),
      };
      existing.reactedByCurrentUser ||= reaction.reactedByCurrentUser === true;
      for (const user of reaction.users) {
        const userKey = normalizePubkey(user.pubkey) || user.displayName;
        existing.usersByKey.set(userKey, user);
      }
      reactionsByEmoji.set(reaction.emoji, existing);
    }
  }

  return [...reactionsByEmoji.values()].map(({ usersByKey, ...reaction }) => {
    const users = [...usersByKey.values()];
    return { ...reaction, count: users.length, users };
  });
}

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

function isKnownAgentPubkey(
  pubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  personaLookup?: Map<string, string>,
  agentPubkeys?: ReadonlySet<string>,
) {
  if (!pubkey) {
    return false;
  }

  const normalizedPubkey = normalizePubkey(pubkey);
  return (
    agentPubkeys?.has(normalizedPubkey) === true ||
    profiles?.[normalizedPubkey]?.isAgent === true ||
    personaLookup?.has(normalizedPubkey) === true
  );
}

function ProfileName({
  children,
  highlight = false,
  isAgent = false,
  pubkey,
  underlineOnHover = false,
}: {
  children: React.ReactNode;
  highlight?: boolean;
  isAgent?: boolean;
  pubkey: string | undefined;
  underlineOnHover?: boolean;
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
        underlineOnHover && "hover:underline",
      )}
    >
      {highlight && !isAgentMention ? (
        <span className={MENTION_CHIP_PREFIX_CLASS}>@</span>
      ) : null}
      {children}
    </span>
  );

  const botIdenticonValue = typeof children === "string" ? children : undefined;

  return pubkey ? (
    <UserProfilePopover
      botIdenticonValue={botIdenticonValue}
      pubkey={pubkey}
      role={isAgent ? "bot" : undefined}
      triggerElement="span"
    >
      {node}
    </UserProfilePopover>
  ) : (
    node
  );
}

function SystemMessageAvatar({
  actorPubkey,
  agentPubkeys,
  currentPubkey,
  personaLookup,
  profiles,
  targetPubkey,
}: {
  actorPubkey: string | undefined;
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey: string | undefined;
  personaLookup?: Map<string, string>;
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
    const isSingleAgent = isKnownAgentPubkey(
      singlePubkey,
      profiles,
      personaLookup,
      agentPubkeys,
    );
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
        <UserProfilePopover
          botIdenticonValue={isSingleAgent ? actorLabel : undefined}
          pubkey={singlePubkey}
          role={isSingleAgent ? "bot" : undefined}
        >
          <button
            className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="system-message-avatar"
            type="button"
          >
            {avatar}
          </button>
        </UserProfilePopover>
      );
    }

    return avatar;
  }

  const isActorAgent = isKnownAgentPubkey(
    actorPubkey,
    profiles,
    personaLookup,
    agentPubkeys,
  );
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
    <UserProfilePopover
      botIdenticonValue={isActorAgent ? actorLabel : undefined}
      pubkey={actorPubkey}
      role={isActorAgent ? "bot" : undefined}
    >
      <button
        className="shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        type="button"
      >
        {dualAvatar}
      </button>
    </UserProfilePopover>
  );
}

function MembershipPersonName({
  agentPubkeys,
  currentPubkey,
  personaLookup,
  profiles,
  pubkey,
}: {
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey: string | undefined;
  personaLookup?: Map<string, string>;
  profiles: UserProfileLookup | undefined;
  pubkey: string;
}) {
  return (
    <ProfileName
      isAgent={isKnownAgentPubkey(
        pubkey,
        profiles,
        personaLookup,
        agentPubkeys,
      )}
      pubkey={pubkey}
      underlineOnHover
    >
      {resolveDisplayLabel(pubkey, currentPubkey, profiles)}
    </ProfileName>
  );
}

function MemberNamesInlineList({
  agentPubkeys,
  currentPubkey,
  personaLookup,
  profiles,
  targets,
}: {
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey: string | undefined;
  personaLookup?: Map<string, string>;
  profiles: UserProfileLookup | undefined;
  targets: string[];
}) {
  const visibleTargets = targets.slice(0, MAX_VISIBLE_ADDITIONAL_MEMBER_NAMES);
  const hiddenTargets = targets.slice(MAX_VISIBLE_ADDITIONAL_MEMBER_NAMES);
  const renderName = (pubkey: string) => (
    <MembershipPersonName
      agentPubkeys={agentPubkeys}
      currentPubkey={currentPubkey}
      personaLookup={personaLookup}
      profiles={profiles}
      pubkey={pubkey}
    />
  );

  return (
    <>
      {visibleTargets.map((pubkey, index) => {
        const isLast = index === visibleTargets.length - 1;
        const separator =
          index === 0
            ? null
            : isLast && hiddenTargets.length === 0
              ? visibleTargets.length === 2
                ? " and "
                : ", and "
              : ", ";
        return (
          <React.Fragment key={pubkey}>
            {separator}
            {renderName(pubkey)}
          </React.Fragment>
        );
      })}
      {hiddenTargets.length > 0 ? (
        <>
          , and{" "}
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                className="cursor-help rounded-xs hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                type="button"
              >
                {hiddenTargets.length} others
              </button>
            </TooltipTrigger>
            <TooltipContent className="max-w-72 p-2 text-left" side="top">
              <div className="max-h-56 space-y-1 overflow-y-auto pr-1">
                {hiddenTargets.map((pubkey) => (
                  <div className="flex items-center gap-2" key={pubkey}>
                    <UserAvatar
                      avatarUrl={resolveAvatarUrl(pubkey, profiles)}
                      className="!h-5 !w-5 shrink-0 text-3xs"
                      displayName={resolveDisplayLabel(
                        pubkey,
                        currentPubkey,
                        profiles,
                      )}
                    />
                    <span className="min-w-0 truncate">
                      {resolveDisplayLabel(pubkey, currentPubkey, profiles)}
                    </span>
                  </div>
                ))}
              </div>
            </TooltipContent>
          </Tooltip>
        </>
      ) : null}
    </>
  );
}

function describeSystemEvent(
  payload: SystemMessagePayload,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
  personaLookup?: Map<string, string>,
  agentPubkeys?: ReadonlySet<string>,
): SystemMessageDescription | null {
  const isTargetAgent = isKnownAgentPubkey(
    payload.target,
    profiles,
    personaLookup,
    agentPubkeys,
  );
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
  const membershipTitle = (
    <ProfileName
      isAgent={isTargetAgent}
      pubkey={payload.target}
      underlineOnHover
    >
      {targetLabel}
    </ProfileName>
  );

  switch (payload.type) {
    case "members_added":
      if (!payload.actor || !payload.targets?.length) return null;
      return {
        title: membershipTitle,
        action: (
          <>
            was added by{" "}
            <ProfileName pubkey={payload.actor} underlineOnHover>
              {resolveDisplayLabel(payload.actor, currentPubkey, profiles)}
            </ProfileName>
            , along with{" "}
            <MemberNamesInlineList
              agentPubkeys={agentPubkeys}
              currentPubkey={currentPubkey}
              personaLookup={personaLookup}
              profiles={profiles}
              targets={payload.targets.slice(1)}
            />
          </>
        ),
      };
    case "members_joined":
      if (!payload.targets?.length) return null;
      return {
        title: membershipTitle,
        action: (
          <>
            joined the channel along with{" "}
            <MemberNamesInlineList
              agentPubkeys={agentPubkeys}
              currentPubkey={currentPubkey}
              personaLookup={personaLookup}
              profiles={profiles}
              targets={payload.targets.slice(1)}
            />
          </>
        ),
      };
    case "member_joined": {
      if (!payload.actor || !payload.target) return null;
      if (normalizePubkey(payload.actor) === normalizePubkey(payload.target)) {
        return {
          title: membershipTitle,
          action: "joined the channel",
        };
      }
      return {
        title: membershipTitle,
        action: (
          <>
            was added by{" "}
            <ProfileName pubkey={payload.actor} underlineOnHover>
              {resolveDisplayLabel(payload.actor, currentPubkey, profiles)}
            </ProfileName>
          </>
        ),
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
    case "message_deleted": {
      // Room-facing tombstone. When a moderator removed the message, the relay
      // stamps a sanitized public_reason; a plain self-delete carries none. The
      // content and the reporter are never disclosed here.
      if (payload.public_reason) {
        return {
          title: "Removed by community moderators",
          action: payload.public_reason,
        };
      }
      return {
        title: actorName,
        action: "removed a message",
      };
    }
    default:
      return null;
  }
}

export const SystemMessageRow = React.memo(function SystemMessageRow({
  message,
  groupedMessages,
  currentPubkey,
  agentPubkeys,
  profiles,
  ownerProfiles,
  personaLookup,
  onToggleReaction,
}: {
  message: TimelineMessage;
  groupedMessages?: TimelineMessage[];
  currentPubkey?: string;
  agentPubkeys?: ReadonlySet<string>;
  profiles?: UserProfileLookup;
  ownerProfiles?: UserProfileLookup;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
  const sourceMessages = React.useMemo(
    () => groupedMessages ?? [message],
    [groupedMessages, message],
  );
  const groupedPayload = React.useMemo(
    () => buildGroupedMembershipPayload(sourceMessages),
    [sourceMessages],
  );
  const reactionMessage = React.useMemo(
    () =>
      groupedPayload
        ? {
            ...message,
            pending: sourceMessages.some((source) => source.pending),
            reactions: aggregateGroupedReactions(sourceMessages),
          }
        : message,
    [groupedPayload, message, sourceMessages],
  );
  const handleGroupedReaction = React.useCallback(
    async (_groupMessage: TimelineMessage, emoji: string, remove: boolean) => {
      if (!onToggleReaction) return;
      if (!remove) {
        await onToggleReaction(message, emoji, false);
        return;
      }

      const reactedMessages = sourceMessages.filter((source) =>
        source.reactions?.some(
          (reaction) =>
            reaction.emoji === emoji && reaction.reactedByCurrentUser,
        ),
      );
      await Promise.all(
        reactedMessages.map((source) => onToggleReaction(source, emoji, true)),
      );
    },
    [message, onToggleReaction, sourceMessages],
  );
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
  } = useReactionHandler(
    reactionMessage,
    groupedPayload && onToggleReaction
      ? handleGroupedReaction
      : onToggleReaction,
  );

  const payload = groupedPayload ?? parseSystemMessagePayload(message);
  if (!payload) return null;

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
  const isMembershipArrival =
    payload.type === "member_joined" ||
    payload.type === "members_added" ||
    payload.type === "members_joined";
  const displayedIdentityPubkey = isMembershipArrival
    ? payload.target
    : payload.actor;
  const displayedIdentityProfile = displayedIdentityPubkey
    ? profiles?.[normalizePubkey(displayedIdentityPubkey)]
    : undefined;
  const displayedTimelineIdentity = displayedIdentityPubkey
    ? sourceMessages.find(
        (source) =>
          source.pubkey &&
          normalizePubkey(source.pubkey) ===
            normalizePubkey(displayedIdentityPubkey),
      )
    : undefined;
  const displayedIdentityIsAgent = Boolean(
    displayedIdentityProfile?.isAgent ||
      displayedTimelineIdentity?.isAgent ||
      (displayedIdentityPubkey &&
        agentPubkeys?.has(normalizePubkey(displayedIdentityPubkey))),
  );
  const displayedOwnerPubkey =
    displayedIdentityProfile?.ownerPubkey ??
    displayedTimelineIdentity?.ownerPubkey ??
    null;
  const displayedOwnerLabel =
    displayedTimelineIdentity?.ownerLabel ??
    formatOwnerLabel(displayedOwnerPubkey, currentPubkey, ownerProfiles);

  const wouldAddReaction = (emoji: string) =>
    !reactions.some(
      (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
    );

  return (
    <div
      className="group/message relative mx-1 rounded-2xl px-2 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50"
      data-testid="system-message-row"
    >
      <div className="flex items-start gap-2.5">
        <SystemMessageAvatar
          actorPubkey={isMembershipArrival ? payload.target : payload.actor}
          agentPubkeys={agentPubkeys}
          currentPubkey={currentPubkey}
          personaLookup={personaLookup}
          profiles={profiles}
          targetPubkey={isMembershipArrival ? undefined : payload.target}
        />
        <div
          className={cn(
            MESSAGE_MARKDOWN_CLASS,
            "flex min-w-0 flex-1 flex-col gap-0.5",
          )}
        >
          <MessageHeaderRow>
            <MessageAuthorText as="div" className="text-foreground">
              {description.title}
            </MessageAuthorText>
            {displayedIdentityIsAgent ? (
              <MessageAgentOwner
                ownerLabel={displayedOwnerLabel}
                ownerPubkey={displayedOwnerPubkey}
              />
            ) : null}
            <MessageTimestamp
              createdAt={message.createdAt}
              time={message.time}
            />
          </MessageHeaderRow>
          <p className="-mt-0.5 text-sm leading-snug text-foreground">
            {description.action}
          </p>
          <div>
            <MessageReactions
              messageId={reactionMessage.id}
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
