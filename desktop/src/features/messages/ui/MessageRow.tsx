import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { HuddleAttachment } from "@/features/huddle/components/HuddleAttachment";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import {
  getThreadReplyAvatarCenterRem,
  getThreadReplyAvatarCenterYRem,
  getThreadReplyDescendantRailStartYRem,
  getThreadReplyConnectorLayout,
  getThreadReplyIndentRem,
  threadReplyLength,
  THREAD_REPLY_LINE_WIDTH_REM,
} from "@/features/messages/lib/threadTreeLayout";
import {
  KIND_HUDDLE_STARTED,
  KIND_STREAM_MESSAGE_DIFF,
} from "@/shared/constants/kinds";
import { getConfigNudgeAuthorPubkey } from "@/features/messages/ui/configNudgeAuthPubkey";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { parseImetaTags } from "@/features/messages/lib/parseImeta";
import { useMessageEmoji } from "@/features/messages/lib/useMessageEmoji";
import { parseWaveMessageContent } from "@/features/messages/lib/waveMessage";
import {
  resolveMentionNames,
  resolveMentionPubkeysByName,
} from "@/shared/lib/resolveMentionNames";
import { Markdown } from "@/shared/ui/markdown";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";
import { MessageActionBar } from "./MessageActionBar";
import { MessageAuthorText, MessageHeaderRow } from "./MessageHeader";
import { MessageTimestamp } from "./MessageTimestamp";
import { WaveMessageAttachment } from "./WaveMessageAttachment";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const DiffMessage = React.lazy(() => import("./DiffMessage"));
const DiffMessageExpanded = React.lazy(() => import("./DiffMessageExpanded"));

export type ThreadDepthGuideAction = {
  active?: boolean;
  depth: number;
  label: string;
  message: TimelineMessage;
};

export const MessageRow = React.memo(
  function MessageRow({
    channelId = null,
    collapseDepthGuideActions,
    connectDescendants = false,
    depthGuideDepths,
    highlighted = false,
    highlightDescendantRail = false,
    highlightReplyConnector = false,
    highlightThreadLineDepths,
    hoverBackground = true,
    huddleMemberPubkeys,
    huddleMemberPubkeysPending = false,
    actionBarPlacement = "floating",
    collapseDescendantsLabel,
    isFollowingThread,
    isContinuation = false,
    isUnread,
    layoutVariant = "default",
    message,
    onCollapseDepthGuide,
    onCollapseDepthGuideHoverChange,
    onCollapseDescendants,
    onCollapseDescendantsHoverChange,
    onDelete,
    onEdit,
    onFollowThread,
    onMarkUnread,
    onMarkRead,
    onToggleReaction,
    onReply,
    onUnfollowThread,
    profiles,
    searchQuery,
    showDepthGuides = true,
    agentPubkeys,
    videoReviewContext,
  }: {
    agentPubkeys?: ReadonlySet<string>;
    channelId?: string | null;
    collapseDepthGuideActions?: ReadonlyArray<ThreadDepthGuideAction>;
    connectDescendants?: boolean;
    depthGuideDepths?: ReadonlyArray<number>;
    highlighted?: boolean;
    highlightDescendantRail?: boolean;
    highlightReplyConnector?: boolean;
    highlightThreadLineDepths?: ReadonlyArray<number>;
    hoverBackground?: boolean;
    huddleMemberPubkeys?: readonly string[];
    huddleMemberPubkeysPending?: boolean;
    actionBarPlacement?: "floating" | "inside";
    collapseDescendantsLabel?: string;
    isFollowingThread?: boolean;
    isContinuation?: boolean;
    isUnread?: boolean;
    layoutVariant?: "default" | "thread-reply";
    message: TimelineMessage;
    onCollapseDepthGuide?: (message: TimelineMessage) => void;
    onCollapseDepthGuideHoverChange?: (
      message: TimelineMessage,
      hovered: boolean,
    ) => void;
    onCollapseDescendants?: (message: TimelineMessage) => void;
    onCollapseDescendantsHoverChange?: (
      message: TimelineMessage,
      hovered: boolean,
    ) => void;
    onDelete?: (message: TimelineMessage) => void;
    onEdit?: (message: TimelineMessage) => void;
    onFollowThread?: (message: TimelineMessage) => void;
    onMarkUnread?: (message: TimelineMessage) => void;
    onMarkRead?: (message: TimelineMessage) => void;
    onToggleReaction?: (
      message: TimelineMessage,
      emoji: string,
      remove: boolean,
    ) => Promise<void>;
    onReply?: (message: TimelineMessage) => void;
    onUnfollowThread?: (message: TimelineMessage) => void;
    profiles?: UserProfileLookup;
    searchQuery?: string;
    showDepthGuides?: boolean;
    videoReviewContext?: VideoReviewContext;
  }) {
    const [expandedDiffId, setExpandedDiffId] = React.useState<string | null>(
      null,
    );
    const [badgeBurstEmoji, setBadgeBurstEmoji] = React.useState<string | null>(
      null,
    );
    const {
      reactions,
      canToggle: canToggleReactions,
      pending: reactionPending,
      errorMessage: reactionErrorMessage,
      select: handleReactionSelect,
    } = useReactionHandler(message, onToggleReaction);
    const { openReminder, activeReminderEventIds } = useRemindLater();
    const hasActiveReminder = activeReminderEventIds.has(message.id);
    const mentionNames = React.useMemo(
      () => resolveMentionNames(message.tags, profiles),
      [profiles, message.tags],
    );
    const mentionPubkeysByName = React.useMemo(
      () => resolveMentionPubkeysByName(message.tags, profiles),
      [profiles, message.tags],
    );
    const resolvedAgentPubkeys = React.useMemo(() => {
      const pubkeys = new Set(agentPubkeys ?? []);

      for (const [pubkey, profile] of Object.entries(profiles ?? {})) {
        if (profile.isAgent) {
          pubkeys.add(normalizePubkey(pubkey));
        }
      }

      return pubkeys;
    }, [agentPubkeys, profiles]);
    const profilePopoverRole =
      message.role === "bot" ||
      (message.pubkey &&
        resolvedAgentPubkeys.has(normalizePubkey(message.pubkey)))
        ? "bot"
        : message.role;
    const agentMentionPubkeysByName = React.useMemo(() => {
      if (!mentionPubkeysByName) {
        return undefined;
      }

      const values: Record<string, string> = {};
      for (const [name, pubkey] of Object.entries(mentionPubkeysByName)) {
        if (resolvedAgentPubkeys.has(normalizePubkey(pubkey))) {
          values[name] = pubkey;
        }
      }

      return Object.keys(values).length > 0 ? values : undefined;
    }, [resolvedAgentPubkeys, mentionPubkeysByName]);

    const imetaByUrl = React.useMemo(
      () => (message.tags ? parseImetaTags(message.tags) : undefined),
      [message.tags],
    );

    const { customEmoji, emojiOnly } = useMessageEmoji(
      message.body,
      message.tags,
    );
    const bodyOffsetClass = emojiOnly ? "mt-1" : "-mt-0.5";

    const { channels } = useChannelNavigation();
    const channelNames = React.useMemo(
      () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
      [channels],
    );

    const indentRem = getThreadReplyIndentRem(message.depth);
    const descendantGuideOffsetRem = connectDescendants
      ? getThreadReplyAvatarCenterRem(message.depth)
      : null;
    const replyConnector = React.useMemo(() => {
      return getThreadReplyConnectorLayout(message.depth);
    }, [message.depth]);
    const depthGuideItems = React.useMemo(() => {
      const depths =
        depthGuideDepths ??
        Array.from(
          { length: Math.max(0, message.depth - 1) },
          (_, index) => index + 1,
        );

      return depths.map((depth) => ({
        depth,
        offset: getThreadReplyAvatarCenterRem(depth),
      }));
    }, [depthGuideDepths, message.depth]);
    const handleCollapseDescendants = React.useCallback(
      (event: React.MouseEvent<HTMLButtonElement>) => {
        event.preventDefault();
        event.stopPropagation();
        onCollapseDescendants?.(message);
      },
      [message, onCollapseDescendants],
    );
    const handleCollapseDescendantsHoverChange = React.useCallback(
      (hovered: boolean) => {
        onCollapseDescendantsHoverChange?.(message, hovered);
      },
      [message, onCollapseDescendantsHoverChange],
    );
    const handleCollapseDepthGuide = React.useCallback(
      (
        event: React.MouseEvent<HTMLButtonElement>,
        targetMessage: TimelineMessage,
      ) => {
        event.preventDefault();
        event.stopPropagation();
        onCollapseDepthGuide?.(targetMessage);
      },
      [onCollapseDepthGuide],
    );
    const handleCollapseDepthGuideHoverChange = React.useCallback(
      (targetMessage: TimelineMessage, hovered: boolean) => {
        onCollapseDepthGuideHoverChange?.(targetMessage, hovered);
      },
      [onCollapseDepthGuideHoverChange],
    );
    const collapseDepthGuideActionsByDepth = React.useMemo(() => {
      if (!collapseDepthGuideActions?.length) {
        return new Map<number, ThreadDepthGuideAction>();
      }

      return new Map(
        collapseDepthGuideActions.map((action) => [action.depth, action]),
      );
    }, [collapseDepthGuideActions]);
    const getTag = (name: string) =>
      message.tags?.find((tag) => tag[0] === name)?.[1];

    const renderBody = () => {
      switch (message.kind) {
        case KIND_STREAM_MESSAGE_DIFF:
          return (
            <React.Suspense
              fallback={
                <div className="p-3 text-sm text-muted-foreground">
                  Loading diff…
                </div>
              }
            >
              <DiffMessage
                commitSha={getTag("commit")}
                content={message.body}
                description={getTag("description")}
                filePath={getTag("file")}
                onExpand={() => {
                  setExpandedDiffId(message.id);
                }}
                repoUrl={getTag("repo")}
                truncated={getTag("truncated") === "true"}
              />
            </React.Suspense>
          );
        case KIND_HUDDLE_STARTED:
          return (
            <HuddleAttachment
              channelId={channelId}
              message={message}
              onOpenThread={onReply}
            />
          );
        default:
          {
            const waveMessage = parseWaveMessageContent(message.body);
            if (waveMessage) {
              return (
                <WaveMessageAttachment
                  channelId={channelId}
                  fallbackText={waveMessage.fallbackText}
                  huddleMemberPubkeys={huddleMemberPubkeys}
                  huddleMemberPubkeysPending={huddleMemberPubkeysPending}
                />
              );
            }
          }

          return (
            <Markdown
              channelNames={channelNames}
              className={cn(
                "max-w-full text-sm",
                emojiOnly &&
                  "text-4xl leading-tight [&_p]:leading-tight [&_img[data-custom-emoji]]:h-[1.45em] [&_img[data-custom-emoji]]:align-middle [&_button:has(img[data-custom-emoji])]:align-middle",
              )}
              // Only pass the author pubkey for agent-authored messages so
              // config-nudge cards can authenticate the sender. Uses the
              // raw event signer (signerPubkey) — not the tag-attributed
              // display author — to prevent actor/p-tag spoofing.
              configNudgeAuthorPubkey={getConfigNudgeAuthorPubkey(
                message,
                resolvedAgentPubkeys,
              )}
              content={message.body}
              customEmoji={customEmoji}
              imetaByUrl={imetaByUrl}
              agentMentionPubkeysByName={agentMentionPubkeysByName}
              mentionNames={mentionNames}
              mentionPubkeysByName={mentionPubkeysByName}
              searchQuery={searchQuery}
              videoReviewContext={videoReviewContext}
            />
          );
      }
    };

    const isThreadReplyLayout = layoutVariant === "thread-reply";
    const guideBleedRem = isThreadReplyLayout ? 0.25 : 0;
    const avatarButtonRadiusClass = "rounded-full";

    const respondToDotColor =
      message.respondTo === "anyone"
        ? "bg-emerald-500"
        : message.respondTo === "allowlist"
          ? "bg-amber-500"
          : null;

    const avatarNode = (
      <div className="relative shrink-0">
        <UserAvatar
          accent={message.accent}
          avatarUrl={message.avatarUrl ?? null}
          className="shrink-0"
          displayName={message.author}
          testId="message-avatar"
        />
        {respondToDotColor && !isThreadReplyLayout ? (
          <span
            className={cn(
              "absolute -bottom-0.5 -right-0.5 flex h-3 w-3 items-center justify-center rounded-full bg-background",
            )}
            title={
              message.respondTo === "anyone"
                ? "Responds to anyone"
                : "Responds to allowlist"
            }
          >
            <span className={cn("h-2 w-2 rounded-full", respondToDotColor)} />
          </span>
        ) : null}
      </div>
    );

    const continuationTimestampGutter = (
      <div
        aria-hidden="true"
        className={cn(
          "flex w-9 shrink-0 items-start justify-end pt-0.5",
          isThreadReplyLayout ? "min-h-9 self-start" : "self-stretch",
        )}
      >
        <MessageTimestamp
          className="opacity-0 transition-opacity group-hover/message:opacity-100 group-focus-within/message:opacity-100"
          createdAt={message.createdAt}
          hideDayPeriod
          time={message.time}
        />
      </div>
    );

    const avatarGutterNode = isContinuation ? (
      continuationTimestampGutter
    ) : message.pubkey ? (
      <UserProfilePopover
        pubkey={message.pubkey}
        role={profilePopoverRole}
        botIdenticonValue={message.author}
      >
        <button
          className={cn(
            "flex shrink-0 items-start focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            avatarButtonRadiusClass,
          )}
          type="button"
        >
          {avatarNode}
        </button>
      </UserProfilePopover>
    ) : (
      <div className="flex shrink-0 items-start">{avatarNode}</div>
    );

    const authorNode = message.pubkey ? (
      <MessageAuthorText hoverUnderline>{message.author}</MessageAuthorText>
    ) : (
      <MessageAuthorText as="h3">{message.author}</MessageAuthorText>
    );

    const actionBarNode = (
      <div
        className={cn(
          "absolute right-2 top-1 z-10 sm:pointer-events-none",
          actionBarPlacement === "floating"
            ? "sm:top-0 sm:-translate-y-1/2"
            : "sm:top-1 sm:translate-y-0",
        )}
      >
        <MessageActionBar
          channelId={channelId}
          isFollowingThread={isFollowingThread}
          isUnread={isUnread}
          message={message}
          onDelete={onDelete}
          onEdit={onEdit}
          onFollowThread={onFollowThread}
          onMarkUnread={onMarkUnread}
          onMarkRead={onMarkRead}
          onReactionBadgeBurstRequest={
            reactionPending ? undefined : setBadgeBurstEmoji
          }
          onReactionSelect={
            canToggleReactions ? handleReactionSelect : undefined
          }
          onRemindLater={(msg) => {
            openReminder({
              eventId: msg.id,
              channelId: channelId ?? "",
              preview: msg.body.slice(0, 100),
              authorPubkey: msg.pubkey ?? "",
            });
          }}
          onReply={onReply}
          onUnfollowThread={onUnfollowThread}
          reactionErrorMessage={reactionErrorMessage}
          reactions={reactions}
        />
      </div>
    );

    const statusMetadataNode =
      message.pending || message.edited ? (
        <>
          {message.pending ? (
            <p className="font-medium uppercase tracking-[0.14em] text-primary/80">
              Sending
            </p>
          ) : null}
          {message.edited ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <p className="text-muted-foreground/70">(edited)</p>
              </TooltipTrigger>
              <TooltipContent>This message has been edited</TooltipContent>
            </Tooltip>
          ) : null}
        </>
      ) : null;

    const inlineMetadataNode = (
      <div className="flex shrink-0 items-baseline gap-2 text-xs">
        <MessageTimestamp createdAt={message.createdAt} time={message.time} />
        {statusMetadataNode}
      </div>
    );

    const continuationMetadataNode =
      isContinuation && statusMetadataNode ? (
        <div className="mt-0.5 flex items-baseline gap-2 text-xs">
          {statusMetadataNode}
        </div>
      ) : null;

    const headerNode = isContinuation ? null : (
      <MessageHeaderRow>
        {message.pubkey ? (
          <UserProfilePopover
            pubkey={message.pubkey}
            role={profilePopoverRole}
            botIdenticonValue={message.author}
          >
            <button
              className="truncate rounded leading-4 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              type="button"
            >
              {authorNode}
            </button>
          </UserProfilePopover>
        ) : (
          authorNode
        )}
        {inlineMetadataNode}
        {message.personaDisplayName &&
        message.personaDisplayName !== message.author ? (
          <span className="text-xs text-muted-foreground">
            {message.personaDisplayName}
          </span>
        ) : null}
      </MessageHeaderRow>
    );
    const bodyContainerClass = isContinuation ? "mt-0" : bodyOffsetClass;

    const messageBodyNode = (
      <>
        {renderBody()}
        {continuationMetadataNode}
        <MessageReactions
          messageId={message.id}
          reactions={reactions}
          canToggle={canToggleReactions}
          pending={reactionPending}
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
        {expandedDiffId === message.id ? (
          <React.Suspense
            fallback={
              <div className="p-3 text-sm text-muted-foreground">
                Loading diff viewer…
              </div>
            }
          >
            <DiffMessageExpanded
              content={message.body}
              filePath={getTag("file")}
              onClose={() => {
                setExpandedDiffId(null);
              }}
            />
          </React.Suspense>
        ) : null}
      </>
    );

    return (
      <div
        className="relative"
        style={
          indentRem > 0
            ? { paddingLeft: threadReplyLength(indentRem) }
            : undefined
        }
      >
        {showDepthGuides && depthGuideItems.length > 0 ? (
          <div
            aria-hidden={
              collapseDepthGuideActionsByDepth.size > 0 ? undefined : true
            }
            className={cn(
              "absolute left-0",
              collapseDepthGuideActionsByDepth.size === 0 &&
                "pointer-events-none",
            )}
            style={{
              bottom: threadReplyLength(-guideBleedRem),
              top: threadReplyLength(-guideBleedRem),
            }}
          >
            {depthGuideItems.map(({ depth, offset }) => {
              const collapseAction =
                collapseDepthGuideActionsByDepth.get(depth);
              const isHighlighted =
                Boolean(collapseAction?.active) ||
                Boolean(highlightThreadLineDepths?.includes(depth));
              if (collapseAction) {
                return (
                  <React.Fragment key={`${message.id}-depth-guide-${offset}`}>
                    <div
                      aria-hidden
                      className={cn(
                        "pointer-events-none absolute bottom-0 top-0 border-l transition-[border-color]",
                        isHighlighted ? "border-primary" : "border-border/45",
                      )}
                      style={{
                        borderLeftWidth: threadReplyLength(
                          THREAD_REPLY_LINE_WIDTH_REM,
                        ),
                        left: threadReplyLength(offset),
                      }}
                    />
                    <button
                      aria-label={collapseAction.label}
                      className="absolute bottom-0 top-0 z-20 w-5 -translate-x-1/2 cursor-pointer rounded-full focus-visible:outline-hidden"
                      data-thread-head-id={collapseAction.message.id}
                      data-testid="thread-collapse-guide"
                      onBlur={() =>
                        handleCollapseDepthGuideHoverChange(
                          collapseAction.message,
                          false,
                        )
                      }
                      onClick={(event) =>
                        handleCollapseDepthGuide(event, collapseAction.message)
                      }
                      onFocus={() =>
                        handleCollapseDepthGuideHoverChange(
                          collapseAction.message,
                          true,
                        )
                      }
                      onMouseEnter={() =>
                        handleCollapseDepthGuideHoverChange(
                          collapseAction.message,
                          true,
                        )
                      }
                      onMouseLeave={() =>
                        handleCollapseDepthGuideHoverChange(
                          collapseAction.message,
                          false,
                        )
                      }
                      style={{ left: threadReplyLength(offset) }}
                      type="button"
                    />
                  </React.Fragment>
                );
              }

              return (
                <div
                  aria-hidden
                  className={cn(
                    "pointer-events-none absolute bottom-0 top-0 border-l transition-[border-color]",
                    isHighlighted ? "border-primary" : "border-border/45",
                  )}
                  key={`${message.id}-depth-guide-${offset}`}
                  style={{
                    borderLeftWidth: threadReplyLength(
                      THREAD_REPLY_LINE_WIDTH_REM,
                    ),
                    left: threadReplyLength(offset),
                  }}
                />
              );
            })}
          </div>
        ) : null}
        {showDepthGuides && descendantGuideOffsetRem !== null ? (
          <>
            <div
              aria-hidden
              className={cn(
                "pointer-events-none absolute bottom-0 z-0 border-l transition-[border-color]",
                highlightDescendantRail ? "border-primary" : "border-border/45",
              )}
              style={{
                bottom: threadReplyLength(-guideBleedRem),
                borderLeftWidth: threadReplyLength(THREAD_REPLY_LINE_WIDTH_REM),
                left: threadReplyLength(descendantGuideOffsetRem),
                top: threadReplyLength(getThreadReplyDescendantRailStartYRem()),
              }}
            />
            {onCollapseDescendants ? (
              <button
                aria-label={
                  collapseDescendantsLabel ?? "Collapse replies to this message"
                }
                className="absolute bottom-0 z-20 w-5 -translate-x-1/2 cursor-pointer rounded-full p-0 focus-visible:outline-hidden"
                data-thread-head-id={message.id}
                data-testid="thread-collapse-rail"
                onBlur={() => handleCollapseDescendantsHoverChange(false)}
                onClick={handleCollapseDescendants}
                onFocus={() => handleCollapseDescendantsHoverChange(true)}
                onMouseEnter={() => handleCollapseDescendantsHoverChange(true)}
                onMouseLeave={() => handleCollapseDescendantsHoverChange(false)}
                style={{
                  left: threadReplyLength(descendantGuideOffsetRem),
                  top: threadReplyLength(getThreadReplyAvatarCenterYRem()),
                }}
                type="button"
              />
            ) : null}
          </>
        ) : null}
        {showDepthGuides && replyConnector ? (
          <div
            aria-hidden
            className={cn(
              "pointer-events-none absolute left-0 top-0 rounded-bl-2xl border-b border-l transition-[border-color]",
              highlightReplyConnector ? "border-primary" : "border-border/45",
            )}
            style={{
              borderBottomWidth: threadReplyLength(THREAD_REPLY_LINE_WIDTH_REM),
              borderLeftWidth: threadReplyLength(THREAD_REPLY_LINE_WIDTH_REM),
              height: threadReplyLength(
                replyConnector.heightRem + guideBleedRem,
              ),
              left: threadReplyLength(replyConnector.parentOffsetRem),
              top: threadReplyLength(-guideBleedRem),
              width: threadReplyLength(replyConnector.widthRem),
            }}
          />
        ) : null}

        <article
          className={cn(
            "group/message relative z-10 rounded-2xl transition-colors",
            "py-1",
            hoverBackground
              ? "mx-1 px-2 hover:bg-muted/50 focus-within:bg-muted/50"
              : isThreadReplyLayout
                ? "mx-1 px-2"
                : "px-2",
            "flex gap-2.5",
            isContinuation ? "items-center" : "items-start",
            hasActiveReminder ? "bg-blue-500/10" : "",
            highlighted
              ? "-mx-4 rounded-none px-6 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-8"
              : "",
          )}
          data-message-id={message.id}
          data-testid="message-row"
        >
          {isThreadReplyLayout ? (
            <>
              {avatarGutterNode}
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                {headerNode}
                <div className={bodyContainerClass}>{messageBodyNode}</div>
              </div>
            </>
          ) : (
            <>
              {avatarGutterNode}
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                {headerNode}
                <div className={bodyContainerClass}>{messageBodyNode}</div>
              </div>
            </>
          )}
          {actionBarNode}
        </article>
      </div>
    );
    // Callbacks (onReply, onToggleReaction) intentionally excluded: inline arrows
    // from parent create new refs every render — including them defeats memo.
  },
  (prev, next) =>
    prev.message.id === next.message.id &&
    prev.message.pubkey === next.message.pubkey &&
    prev.message.body === next.message.body &&
    prev.message.author === next.message.author &&
    prev.message.avatarUrl === next.message.avatarUrl &&
    prev.message.accent === next.message.accent &&
    prev.message.time === next.message.time &&
    prev.message.depth === next.message.depth &&
    prev.message.kind === next.message.kind &&
    prev.message.pending === next.message.pending &&
    prev.message.edited === next.message.edited &&
    prev.message.reactions === next.message.reactions &&
    prev.message.tags === next.message.tags &&
    prev.message.role === next.message.role &&
    prev.message.personaDisplayName === next.message.personaDisplayName &&
    prev.agentPubkeys === next.agentPubkeys &&
    prev.collapseDepthGuideActions === next.collapseDepthGuideActions &&
    prev.collapseDescendantsLabel === next.collapseDescendantsLabel &&
    prev.connectDescendants === next.connectDescendants &&
    prev.depthGuideDepths === next.depthGuideDepths &&
    prev.highlightDescendantRail === next.highlightDescendantRail &&
    prev.highlighted === next.highlighted &&
    prev.highlightReplyConnector === next.highlightReplyConnector &&
    prev.highlightThreadLineDepths === next.highlightThreadLineDepths &&
    prev.hoverBackground === next.hoverBackground &&
    prev.huddleMemberPubkeys === next.huddleMemberPubkeys &&
    prev.huddleMemberPubkeysPending === next.huddleMemberPubkeysPending &&
    prev.isContinuation === next.isContinuation &&
    prev.isFollowingThread === next.isFollowingThread &&
    prev.isUnread === next.isUnread &&
    prev.layoutVariant === next.layoutVariant &&
    prev.onCollapseDepthGuide === next.onCollapseDepthGuide &&
    prev.onCollapseDepthGuideHoverChange ===
      next.onCollapseDepthGuideHoverChange &&
    prev.onCollapseDescendants === next.onCollapseDescendants &&
    prev.onCollapseDescendantsHoverChange ===
      next.onCollapseDescendantsHoverChange &&
    prev.profiles === next.profiles &&
    prev.searchQuery === next.searchQuery &&
    prev.videoReviewContext === next.videoReviewContext,
);

MessageRow.displayName = "MessageRow";
