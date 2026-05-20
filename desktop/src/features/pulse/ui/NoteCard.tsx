import {
  ALargeSmall,
  AtSign,
  Bookmark,
  Bot,
  MessageCircle,
  PenSquare,
  Paperclip,
  SmilePlus,
  SquareArrowOutUpRight,
  ThumbsUp,
} from "lucide-react";
import * as React from "react";

import type { UserNote } from "@/shared/api/socialTypes";
import type { UserProfileSummary } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type NoteCardProps = {
  note: UserNote;
  profile?: UserProfileSummary | null;
  currentUserDisplayName?: string;
  currentUserProfile?: UserProfileSummary | null;
  isAgent?: boolean;
  isOwnNote: boolean;
  isFollowing: boolean;
  onFollow?: (pubkey: string) => void;
  onReply?: (note: UserNote) => void;
  onShare?: (note: UserNote) => void;
  onUnfollow?: (pubkey: string) => void;
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
  isAgent,
  isOwnNote,
  isFollowing,
  onFollow,
  onReply,
  onShare,
  onUnfollow,
}: NoteCardProps) {
  const displayName = profile?.displayName ?? `${note.pubkey.slice(0, 8)}...`;
  const avatarUrl = profile?.avatarUrl ?? null;
  const [isUpvoted, setIsUpvoted] = React.useState(false);
  const [isBookmarked, setIsBookmarked] = React.useState(false);
  const [isReplyComposerOpen, setIsReplyComposerOpen] = React.useState(false);
  const [replyDraft, setReplyDraft] = React.useState("");
  const replyInputRef = React.useRef<HTMLTextAreaElement>(null);
  const actionButtonClass =
    "inline-flex min-w-7 items-center gap-1.5 text-muted-foreground/60 transition-colors hover:text-foreground focus-visible:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
  const activeActionClass = "text-foreground";
  const countPlaceholder = <span aria-hidden className="w-2.5" />;
  const currentUserAvatarUrl = currentUserProfile?.avatarUrl ?? null;

  React.useEffect(() => {
    if (!isReplyComposerOpen) return;
    replyInputRef.current?.focus();
  }, [isReplyComposerOpen]);

  const handleReplySubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (replyDraft.trim().length === 0) return;
    setReplyDraft("");
    setIsReplyComposerOpen(false);
  };

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
            <button
              aria-label={isUpvoted ? "Remove upvote" : "Upvote"}
              aria-pressed={isUpvoted}
              className={`${actionButtonClass} ${isUpvoted ? activeActionClass : ""}`}
              onClick={() => setIsUpvoted((current) => !current)}
              type="button"
            >
              <ThumbsUp
                className={`h-4 w-4 ${isUpvoted ? "fill-current" : ""}`}
              />
              {countPlaceholder}
            </button>
            <button
              aria-label="Reply"
              aria-expanded={isReplyComposerOpen}
              className={actionButtonClass}
              onClick={() => {
                setIsReplyComposerOpen(true);
                onReply?.(note);
              }}
              type="button"
            >
              <MessageCircle className="h-4 w-4" />
              {countPlaceholder}
            </button>
            <button
              aria-label="Share"
              className={actionButtonClass}
              onClick={() => onShare?.(note)}
              type="button"
            >
              <SquareArrowOutUpRight className="h-4 w-4" />
              {countPlaceholder}
            </button>
            <button
              aria-label="Start direct message"
              className={actionButtonClass}
              type="button"
            >
              <PenSquare className="h-4 w-4" />
            </button>
            {!isOwnNote ? (
              isFollowing ? (
                <button
                  className="text-muted-foreground/60 transition-colors hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => onUnfollow?.(note.pubkey)}
                  type="button"
                >
                  Unfollow
                </button>
              ) : (
                <button
                  className="text-muted-foreground/60 transition-colors hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => onFollow?.(note.pubkey)}
                  type="button"
                >
                  Follow
                </button>
              )
            ) : null}
          </div>
          <button
            aria-label={isBookmarked ? "Remove bookmark" : "Save"}
            aria-pressed={isBookmarked}
            className={`${actionButtonClass} ${isBookmarked ? activeActionClass : ""}`}
            onClick={() => setIsBookmarked((current) => !current)}
            type="button"
          >
            <Bookmark
              className={`h-4 w-4 ${isBookmarked ? "fill-current" : ""}`}
            />
          </button>
        </div>
        {isReplyComposerOpen ? (
          <form
            className="mt-4 flex gap-2 rounded-2xl border border-border/60 bg-background/60 p-3"
            onSubmit={handleReplySubmit}
          >
            <UserAvatar
              avatarUrl={currentUserAvatarUrl}
              className="!h-8 !w-8 shrink-0"
              displayName={currentUserDisplayName}
            />
            <div className="min-w-0 flex-1">
              <textarea
                className="min-h-16 w-full resize-none bg-transparent text-sm text-foreground placeholder:text-muted-foreground/70 focus:outline-none"
                onChange={(event) => setReplyDraft(event.target.value)}
                placeholder="Post your reply"
                ref={replyInputRef}
                value={replyDraft}
              />
              <div className="mt-2 flex items-center justify-between gap-3">
                <div className="flex items-center gap-1 text-muted-foreground">
                  <button
                    aria-label="Mention someone"
                    className="inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    type="button"
                  >
                    <AtSign className="h-4 w-4" />
                  </button>
                  <button
                    aria-label="Attach media"
                    className="inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    type="button"
                  >
                    <Paperclip className="h-4 w-4" />
                  </button>
                  <button
                    aria-label="Add emoji"
                    className="inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    type="button"
                  >
                    <SmilePlus className="h-4 w-4" />
                  </button>
                  <button
                    aria-label="Formatting"
                    className="inline-flex h-8 min-w-8 items-center justify-center rounded-md px-2 text-sm font-medium transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    type="button"
                  >
                    <ALargeSmall className="h-4 w-4" />
                  </button>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    className="text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    onClick={() => {
                      setReplyDraft("");
                      setIsReplyComposerOpen(false);
                    }}
                    type="button"
                  >
                    Cancel
                  </button>
                  <button
                    className="rounded-full bg-foreground px-3 py-1 text-xs font-semibold text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
                    disabled={replyDraft.trim().length === 0}
                    type="submit"
                  >
                    Reply
                  </button>
                </div>
              </div>
            </div>
          </form>
        ) : null}
      </div>
    </article>
  );
}
