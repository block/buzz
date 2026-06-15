import { MessageSquare } from "lucide-react";
import { useMemo } from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { ForumPost } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { parseImetaTags } from "@/features/messages/lib/parseImeta";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { Markdown } from "@/shared/ui/markdown";

import { formatRelativeTime } from "../lib/time";
import { DeleteActionMenu } from "./DeleteActionMenu";

type ForumPostCardProps = {
  post: ForumPost;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  isActive?: boolean;
  canDelete?: boolean;
  isDeleting?: boolean;
  onClick: (post: ForumPost) => void;
  onDelete?: (eventId: string) => void;
};

export function ForumPostCard({
  post,
  currentPubkey,
  profiles,
  isActive,
  canDelete,
  isDeleting,
  onClick,
  onDelete,
}: ForumPostCardProps) {
  const authorLabel = resolveUserLabel({
    pubkey: post.pubkey,
    currentPubkey,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const avatarUrl = profiles?.[post.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const mentionNames = resolveMentionNames(post.tags, profiles);
  // Memoize the imeta map: `parseImetaTags` builds a fresh object each render,
  // and the `Markdown` memo compares `imetaByUrl` by reference. Without this,
  // the post's Markdown (and the FileCard <button> it renders) is rebuilt on
  // every ForumPostCard render, swapping the live DOM node. A click that lands
  // across one of those swaps splits mousedown/mouseup onto different nodes, so
  // the browser never fires `click` and a file download is silently dropped.
  const imetaByUrl = useMemo(() => parseImetaTags(post.tags), [post.tags]);
  const summary = post.threadSummary;
  const previewContent =
    post.content.length > 200
      ? `${post.content.slice(0, 200)}...`
      : post.content;

  return (
    // biome-ignore lint/a11y/useSemanticElements: Cannot use <button> because DeleteActionMenu renders a nested <button> via DropdownMenuTrigger, which is invalid HTML
    <div
      role="button"
      tabIndex={0}
      className={cn(
        "group w-full cursor-pointer rounded-xl border border-border/60 bg-card p-4 text-left transition-colors hover:border-border hover:bg-accent/40",
        isActive && "border-primary/40 bg-accent/60",
        isDeleting && "pointer-events-none opacity-50",
      )}
      onClick={() => onClick(post)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick(post);
        }
      }}
    >
      <div className="flex items-center gap-2">
        {/* biome-ignore lint/a11y/noStaticElementInteractions: presentation wrapper stops click propagation to parent card */}
        <div onClick={(e) => e.stopPropagation()} role="presentation">
          <UserProfilePopover pubkey={post.pubkey}>
            <button
              className="flex items-center gap-2 rounded-lg focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              type="button"
            >
              <UserAvatar
                avatarUrl={avatarUrl}
                displayName={authorLabel}
                size="sm"
              />
              <span className="truncate text-sm font-medium text-foreground hover:underline">
                {authorLabel}
              </span>
            </button>
          </UserProfilePopover>
        </div>
        <span className="text-xs text-muted-foreground">
          {formatRelativeTime(post.createdAt)}
        </span>

        {canDelete && onDelete ? (
          // biome-ignore lint/a11y/noStaticElementInteractions: presentation wrapper only stops click propagation to parent card link
          <div
            className="ml-auto"
            onClick={(e) => e.stopPropagation()}
            role="presentation"
          >
            <DeleteActionMenu
              label="post"
              onConfirm={() => onDelete(post.eventId)}
            />
          </div>
        ) : null}
      </div>

      <div className="mt-2">
        <Markdown
          compact
          content={previewContent}
          imetaByUrl={imetaByUrl}
          mentionNames={mentionNames}
        />
      </div>

      {summary && summary.replyCount > 0 ? (
        <div className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
          <MessageSquare className="h-4 w-4" />
          <span>
            {summary.replyCount}{" "}
            {summary.replyCount === 1 ? "reply" : "replies"}
          </span>
          {summary.lastReplyAt ? (
            <>
              <span className="text-muted-foreground/50">·</span>
              <span>last {formatRelativeTime(summary.lastReplyAt)}</span>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
