import { ArrowLeft, MessageSquare } from "lucide-react";
import * as React from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { ForumThreadResponse, ThreadReply } from "@/shared/api/types";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { parseImetaTags } from "@/features/messages/lib/parseImeta";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";

import { formatRelativeTime } from "../lib/time";
import { DeleteActionMenu } from "./DeleteActionMenu";
import { ForumComposer } from "./ForumComposer";

type ForumThreadPanelProps = {
  thread: ForumThreadResponse | undefined;
  isLoading: boolean;
  isSendingReply: boolean;
  channelId: string;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  onBack: () => void;
  onReply: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => undefined | Promise<unknown>;
  onDeletePost?: (eventId: string) => void;
  onDeleteReply?: (eventId: string) => void;
  onTargetReached?: (eventId: string) => void;
  canDeletePost?: boolean;
  isDeletingPost?: boolean;
  targetEventId?: string | null;
};

function canDeleteReply(
  reply: ThreadReply,
  currentPubkey: string | undefined,
): boolean {
  if (!currentPubkey) return false;
  return reply.pubkey.toLowerCase() === currentPubkey.toLowerCase();
}

function ReplyRow({
  reply,
  currentPubkey,
  profiles,
  channelNames,
  onDelete,
}: {
  reply: ThreadReply;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  channelNames?: string[];
  onDelete?: (eventId: string) => void;
}) {
  const replyAuthorLabel = resolveUserLabel({
    pubkey: reply.pubkey,
    currentPubkey,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const replyAvatarUrl =
    profiles?.[reply.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const showDelete = onDelete && canDeleteReply(reply, currentPubkey);
  const replyMentionNames = resolveMentionNames(reply.tags, profiles);

  return (
    <div
      className="group content-visibility-auto px-4 py-3"
      data-forum-event-id={reply.eventId}
    >
      <div className="flex items-center gap-2">
        <UserProfilePopover pubkey={reply.pubkey}>
          <button
            className="flex items-center gap-2 rounded-lg focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            type="button"
          >
            <UserAvatar
              avatarUrl={replyAvatarUrl}
              displayName={replyAuthorLabel}
              size="sm"
            />
            <span className="text-sm font-medium text-foreground hover:underline">
              {replyAuthorLabel}
            </span>
          </button>
        </UserProfilePopover>
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(reply.createdAt)}
        </span>

        {showDelete ? (
          <DeleteActionMenu
            iconSize="sm"
            label="reply"
            onConfirm={() => onDelete(reply.eventId)}
          />
        ) : null}
      </div>
      <div className="mt-1.5 pl-8">
        <Markdown
          channelNames={channelNames}
          className="text-sm"
          content={reply.content}
          imetaByUrl={parseImetaTags(reply.tags)}
          mentionNames={replyMentionNames}
        />
      </div>
    </div>
  );
}

export function ForumThreadPanel({
  thread,
  isLoading,
  isSendingReply,
  channelId,
  currentPubkey,
  profiles,
  onBack,
  onReply,
  onDeletePost,
  onDeleteReply,
  onTargetReached,
  canDeletePost,
  isDeletingPost,
  targetEventId,
}: ForumThreadPanelProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const { channels } = useChannelNavigation();
  const channelNames = React.useMemo(
    () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
    [channels],
  );

  React.useEffect(() => {
    if (!thread || !targetEventId) {
      return;
    }

    const targetElement =
      scrollRef.current?.querySelector<HTMLElement>(
        `[data-forum-event-id="${targetEventId}"]`,
      ) ?? null;
    if (!targetElement) {
      return;
    }

    targetElement.scrollIntoView({ block: "center" });
    onTargetReached?.(targetEventId);
  }, [onTargetReached, targetEventId, thread]);

  if (isLoading || !thread) {
    return (
      <div className={cn("flex h-full flex-col", channelChrome.contentPadding)}>
        <div className="border-b border-border/60 px-4 py-3">
          <Button
            className="gap-1.5 text-muted-foreground"
            onClick={onBack}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to posts
          </Button>
        </div>
        <div className="flex-1 space-y-4 p-4">
          <Skeleton className="h-8 w-3/4" />
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      </div>
    );
  }

  const { post, replies } = thread;
  const postMentionNames = resolveMentionNames(post.tags, profiles);
  const postAuthorLabel = resolveUserLabel({
    pubkey: post.pubkey,
    currentPubkey,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const postAvatarUrl =
    profiles?.[post.pubkey.toLowerCase()]?.avatarUrl ?? null;

  return (
    <div className={cn("flex h-full flex-col", channelChrome.contentPadding)}>
      <div className="border-b border-border/60 px-4 py-3">
        <Button
          className="gap-1.5 text-muted-foreground"
          onClick={onBack}
          size="sm"
          variant="ghost"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to posts
        </Button>
      </div>

      <div
        className="flex-1 overflow-y-auto"
        data-scroll-restoration-id={`forum-thread:${channelId}`}
        ref={scrollRef}
      >
        <div
          className={cn(
            "group border-b border-border/60 p-4",
            isDeletingPost && "pointer-events-none opacity-50",
          )}
          data-forum-event-id={post.eventId}
        >
          <div className="flex items-center gap-2">
            <UserProfilePopover pubkey={post.pubkey}>
              <button
                className="flex items-center gap-2 rounded-xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                type="button"
              >
                <UserAvatar
                  avatarUrl={postAvatarUrl}
                  displayName={postAuthorLabel}
                />
                <span className="text-sm font-semibold text-foreground hover:underline">
                  {postAuthorLabel}
                </span>
              </button>
            </UserProfilePopover>
            <span className="text-xs text-muted-foreground">
              {formatRelativeTime(post.createdAt)}
            </span>

            {canDeletePost && onDeletePost ? (
              <DeleteActionMenu
                label="post"
                onConfirm={() => onDeletePost(post.eventId)}
              />
            ) : null}
          </div>
          <div className="mt-3">
            <Markdown
              channelNames={channelNames}
              className="text-sm"
              content={post.content}
              imetaByUrl={parseImetaTags(post.tags)}
              mentionNames={postMentionNames}
            />
          </div>
        </div>

        <div className="flex items-center gap-1.5 border-b border-border/60 px-4 py-2.5 text-sm font-medium text-muted-foreground">
          <MessageSquare className="h-4 w-4" />
          {replies.length} {replies.length === 1 ? "reply" : "replies"}
        </div>

        <div className="divide-y divide-border/40">
          {replies.map((reply) => (
            <ReplyRow
              channelNames={channelNames}
              currentPubkey={currentPubkey}
              key={reply.eventId}
              onDelete={onDeleteReply}
              profiles={profiles}
              reply={reply}
            />
          ))}

          {replies.length === 0 ? (
            <div className="px-4 py-6 text-center text-sm text-muted-foreground">
              No replies yet. Be the first to respond.
            </div>
          ) : null}
        </div>
      </div>

      <div className="border-t border-border/60 p-4">
        <ForumComposer
          channelId={channelId}
          isSending={isSendingReply}
          onSubmit={onReply}
          placeholder="Reply to this post..."
          profiles={profiles}
        />
      </div>
    </div>
  );
}
