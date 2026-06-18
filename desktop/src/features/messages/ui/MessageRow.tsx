import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import { KIND_STREAM_MESSAGE_DIFF } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { parseImetaTags } from "@/features/messages/lib/parseImeta";
import { customEmojiFromTags } from "@/shared/api/customEmoji";
import { isEmojiOnlyMessage } from "@/shared/lib/emojiOnly";
import {
  resolveMentionNames,
  resolveMentionPubkeysByName,
} from "@/shared/lib/resolveMentionNames";
import { Markdown } from "@/shared/ui/markdown";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";
import { MessageActionBar } from "./MessageActionBar";
import { MessageAuthorText, MessageHeaderRow } from "./MessageHeader";
import { MessageTimestamp } from "./MessageTimestamp";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const DiffMessage = React.lazy(() => import("./DiffMessage"));
const DiffMessageExpanded = React.lazy(() => import("./DiffMessageExpanded"));

const MESSAGE_TEXT_OFFSET_PX = 54;
const NESTED_REPLY_OFFSET_PX = 28;

export const MessageRow = React.memo(
  function MessageRow({
    channelId = null,
    highlighted = false,
    hoverBackground = true,
    actionBarPlacement = "floating",
    isFollowingThread,
    layoutVariant = "default",
    message,
    onDelete,
    onEdit,
    onFollowThread,
    onMarkUnread,
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
    highlighted?: boolean;
    hoverBackground?: boolean;
    actionBarPlacement?: "floating" | "inside";
    isFollowingThread?: boolean;
    layoutVariant?: "default" | "thread-reply";
    message: TimelineMessage;
    onDelete?: (message: TimelineMessage) => void;
    onEdit?: (message: TimelineMessage) => void;
    onFollowThread?: (message: TimelineMessage) => void;
    onMarkUnread?: (message: TimelineMessage) => void;
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

    const customEmoji = React.useMemo(
      () => (message.tags ? customEmojiFromTags(message.tags) : undefined),
      [message.tags],
    );
    const emojiOnly = React.useMemo(
      () => isEmojiOnlyMessage(message.body, customEmoji),
      [message.body, customEmoji],
    );
    const bodyOffsetClass = emojiOnly ? "mt-1" : "-mt-0.5";

    const { channels } = useChannelNavigation();
    const channelNames = React.useMemo(
      () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
      [channels],
    );

    const visibleDepth = Math.min(message.depth, 6);
    const indentPx =
      visibleDepth > 0
        ? MESSAGE_TEXT_OFFSET_PX + (visibleDepth - 1) * NESTED_REPLY_OFFSET_PX
        : 0;
    const depthGuideOffsets = React.useMemo(() => {
      if (visibleDepth === 0) {
        return [];
      }

      return Array.from({ length: visibleDepth }, (_, index) =>
        index === 0
          ? MESSAGE_TEXT_OFFSET_PX / 2
          : MESSAGE_TEXT_OFFSET_PX +
            NESTED_REPLY_OFFSET_PX / 2 +
            (index - 1) * NESTED_REPLY_OFFSET_PX,
      );
    }, [visibleDepth]);
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
        default:
          return (
            <Markdown
              channelNames={channelNames}
              className={cn(
                "max-w-full text-sm",
                emojiOnly &&
                  "text-4xl leading-tight [&_p]:leading-tight [&_img[data-custom-emoji]]:h-[1.45em] [&_img[data-custom-emoji]]:align-middle [&_button:has(img[data-custom-emoji])]:align-middle",
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
    const guideBleedPx = isThreadReplyLayout ? 4 : 0;
    const avatarSizeClass = "!h-10 !w-10";
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
          className={cn("shrink-0", avatarSizeClass)}
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

    const authorNode = message.pubkey ? (
      <MessageAuthorText hoverUnderline>{message.author}</MessageAuthorText>
    ) : (
      <MessageAuthorText as="h3">{message.author}</MessageAuthorText>
    );

    const actionBarNode = (
      <div
        className={cn(
          "absolute right-2 top-1 z-10",
          actionBarPlacement === "floating"
            ? "sm:top-0 sm:-translate-y-1/2"
            : "sm:top-1 sm:translate-y-0",
        )}
      >
        <MessageActionBar
          channelId={channelId}
          isFollowingThread={isFollowingThread}
          message={message}
          onDelete={onDelete}
          onEdit={onEdit}
          onFollowThread={onFollowThread}
          onMarkUnread={onMarkUnread}
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

    const inlineMetadataNode = (
      <div className="flex shrink-0 items-baseline gap-2 text-xs">
        <MessageTimestamp createdAt={message.createdAt} time={message.time} />
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
      </div>
    );

    const messageBodyNode = (
      <>
        {renderBody()}
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
        style={indentPx > 0 ? { paddingLeft: `${indentPx}px` } : undefined}
      >
        {showDepthGuides && depthGuideOffsets.length > 0 ? (
          <div
            aria-hidden
            className="pointer-events-none absolute left-0"
            style={{
              bottom: `${-guideBleedPx}px`,
              top: `${-guideBleedPx}px`,
            }}
          >
            {depthGuideOffsets.map((offset, index) => (
              <div
                className="absolute bottom-0 top-0 border-l border-border/70"
                key={`${message.id}-depth-guide-${offset}`}
                style={{
                  left: `${offset}px`,
                  opacity: index === depthGuideOffsets.length - 1 ? 0.9 : 0.55,
                }}
              />
            ))}
          </div>
        ) : null}

        <article
          className={cn(
            "group/message relative rounded-2xl py-2 transition-colors",
            hoverBackground
              ? "mx-1 px-2 hover:bg-muted/50 focus-within:bg-muted/50"
              : "px-2",
            "flex items-start gap-2.5",
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
              {message.pubkey ? (
                <UserProfilePopover
                  pubkey={message.pubkey}
                  role={message.role}
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
              )}
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <MessageHeaderRow>
                  {message.pubkey ? (
                    <UserProfilePopover
                      pubkey={message.pubkey}
                      role={message.role}
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
                <div className={bodyOffsetClass}>{messageBodyNode}</div>
              </div>
            </>
          ) : (
            <>
              {message.pubkey ? (
                <UserProfilePopover
                  pubkey={message.pubkey}
                  role={message.role}
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
              )}
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <MessageHeaderRow>
                  {message.pubkey ? (
                    <UserProfilePopover
                      pubkey={message.pubkey}
                      role={message.role}
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
                <div className={bodyOffsetClass}>{messageBodyNode}</div>
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
    prev.highlighted === next.highlighted &&
    prev.hoverBackground === next.hoverBackground &&
    prev.isFollowingThread === next.isFollowingThread &&
    prev.layoutVariant === next.layoutVariant &&
    prev.profiles === next.profiles &&
    prev.searchQuery === next.searchQuery &&
    prev.videoReviewContext === next.videoReviewContext,
);

MessageRow.displayName = "MessageRow";
