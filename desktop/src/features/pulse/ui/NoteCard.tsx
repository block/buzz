import {
  Bot,
  MessageCircle,
  PenSquare,
  SquareArrowOutUpRight,
  ThumbsUp,
} from "lucide-react";
import * as React from "react";

import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import type { UserNote } from "@/shared/api/socialTypes";
import type { ChannelMember, UserProfileSummary } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type NoteCardActions = {
  follow?: (pubkey: string) => void;
  reply?: (
    note: UserNote,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<unknown>;
  share?: (note: UserNote) => void;
  startDm?: (pubkey: string) => void;
  toggleUpvote?: (note: UserNote, remove: boolean) => Promise<unknown>;
  unfollow?: (pubkey: string) => void;
};

type NoteCardProps = {
  note: UserNote;
  profile?: UserProfileSummary | null;
  currentUserDisplayName?: string;
  currentUserProfile?: UserProfileSummary | null;
  composerProfiles?: Record<string, UserProfileSummary>;
  isReplySending?: boolean;
  isUpvotePending?: boolean;
  isUpvoted?: boolean;
  members?: ChannelMember[];
  isAgent?: boolean;
  isOwnNote: boolean;
  isFollowing: boolean;
  actions?: NoteCardActions;
};

function formatRelativeTime(unixSeconds: number): string {
  const now = Date.now() / 1_000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3_600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86_400) return `${Math.floor(diff / 3_600)}h`;
  if (diff < 604_800) return `${Math.floor(diff / 86_400)}d`;

  return new Date(unixSeconds * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function NoteCard({
  note,
  profile,
  currentUserDisplayName = "You",
  currentUserProfile,
  composerProfiles = {},
  isAgent,
  isOwnNote,
  isFollowing,
  isReplySending = false,
  isUpvotePending = false,
  isUpvoted = false,
  members = [],
  actions,
}: NoteCardProps) {
  const displayName = profile?.displayName ?? `${note.pubkey.slice(0, 8)}...`;
  const avatarUrl = profile?.avatarUrl ?? null;
  const [isReplyComposerOpen, setIsReplyComposerOpen] = React.useState(false);
  const actionButtonClass =
    "inline-flex min-w-7 items-center gap-1.5 text-muted-foreground/60 transition-colors hover:text-foreground focus-visible:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";
  const activeActionClass = "text-foreground";
  const countPlaceholder = <span aria-hidden className="w-2.5" />;
  const currentUserAvatarUrl = currentUserProfile?.avatarUrl ?? null;

  return (
    <article className="flex items-start gap-2.5 rounded-2xl px-1 pb-1 pt-4 sm:px-2">
      <div className="relative shrink-0">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="!h-9 !w-9 shrink-0"
          displayName={displayName}
        />
        {isAgent ? (
          <Bot className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full bg-background p-0.5 text-muted-foreground" />
        ) : null}
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
          <span className="truncate text-sm font-semibold leading-none tracking-tight">
            {displayName}
          </span>
          {isAgent ? (
            <span className="inline-flex h-4 items-center rounded bg-muted px-1 text-[10px] font-medium text-muted-foreground">
              bot
            </span>
          ) : null}
          {profile?.nip05Handle ? (
            <span className="truncate text-xs text-muted-foreground">
              {profile.nip05Handle}
            </span>
          ) : null}
          <span className="shrink-0 text-xs text-muted-foreground/70">
            {formatRelativeTime(note.createdAt)}
          </span>
        </div>

        <div className="mt-0.5 pb-3 text-sm text-foreground">
          <Markdown content={note.content} tight />
        </div>

        <div className="flex flex-wrap items-center gap-5 text-xs font-medium">
          <div className="flex flex-wrap items-center gap-5">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  aria-label={isUpvoted ? "Remove upvote" : "Upvote"}
                  aria-pressed={isUpvoted}
                  className={`${actionButtonClass} ${isUpvoted ? activeActionClass : ""} disabled:cursor-not-allowed disabled:opacity-45`}
                  disabled={isUpvotePending}
                  onClick={() => {
                    if (!isUpvotePending) {
                      void actions?.toggleUpvote?.(note, isUpvoted);
                    }
                  }}
                  type="button"
                >
                  <ThumbsUp
                    className={`h-4 w-4 ${isUpvoted ? "fill-current" : ""}`}
                  />
                  {countPlaceholder}
                </button>
              </TooltipTrigger>
              <TooltipContent>
                {isUpvoted ? "Remove upvote" : "Upvote"}
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  aria-label="Reply"
                  aria-expanded={isReplyComposerOpen}
                  className={actionButtonClass}
                  onClick={() => setIsReplyComposerOpen((current) => !current)}
                  type="button"
                >
                  <MessageCircle className="h-4 w-4" />
                  {countPlaceholder}
                </button>
              </TooltipTrigger>
              <TooltipContent>Reply</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  aria-label="Share"
                  className={actionButtonClass}
                  onClick={() => actions?.share?.(note)}
                  type="button"
                >
                  <SquareArrowOutUpRight className="h-4 w-4" />
                  {countPlaceholder}
                </button>
              </TooltipTrigger>
              <TooltipContent>Share</TooltipContent>
            </Tooltip>
            {!isOwnNote ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label="Start direct message"
                    className={actionButtonClass}
                    onClick={() => actions?.startDm?.(note.pubkey)}
                    type="button"
                  >
                    <PenSquare className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent>Start direct message</TooltipContent>
              </Tooltip>
            ) : null}
            {!isOwnNote ? (
              isFollowing ? (
                <button
                  className="text-muted-foreground/60 transition-colors hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => actions?.unfollow?.(note.pubkey)}
                  type="button"
                >
                  Unfollow
                </button>
              ) : (
                <button
                  className="text-muted-foreground/60 transition-colors hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => actions?.follow?.(note.pubkey)}
                  type="button"
                >
                  Follow
                </button>
              )
            ) : null}
          </div>
        </div>
        {isReplyComposerOpen ? (
          <div className="mt-4 rounded-2xl border border-border/60 bg-background/60 p-3">
            <ForumComposer
              compact
              className="pulse-reply-composer border-0 bg-transparent p-0 shadow-none"
              disabled={!actions?.reply}
              header={
                <div className="flex min-w-0 items-center gap-2">
                  <UserAvatar
                    avatarUrl={currentUserAvatarUrl}
                    className="!h-8 !w-8 shrink-0"
                    displayName={currentUserDisplayName}
                  />
                  <span className="max-w-32 truncate text-sm font-medium text-foreground">
                    {currentUserDisplayName}
                  </span>
                </div>
              }
              isSending={isReplySending}
              members={members}
              onCancel={() => setIsReplyComposerOpen(false)}
              onSubmit={(content, mentionPubkeys, mediaTags) =>
                actions
                  ?.reply?.(note, content, mentionPubkeys, mediaTags)
                  ?.then(() => {
                    setIsReplyComposerOpen(false);
                  })
              }
              placeholder="Post your reply"
              profiles={composerProfiles}
            />
          </div>
        ) : null}
      </div>
    </article>
  );
}
