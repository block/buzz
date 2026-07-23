import { FileCode2, MessageSquareText } from "lucide-react";

import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import type {
  ProjectPullRequestComment,
  ProjectPullRequestCommentAnchor,
} from "@/features/projects/projectPullRequests.mjs";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";

function commentAuthor(
  pubkey: string,
  profiles: UserProfileLookup | undefined,
) {
  const profile = profiles?.[normalizePubkey(pubkey)];
  return (
    profile?.displayName?.trim() ||
    profile?.nip05Handle?.trim() ||
    truncatePubkey(pubkey)
  );
}

function commentDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  });
}

export function ProjectPullRequestInlineCommentThread({
  activeAnchor,
  comments,
  isSending,
  onCancel,
  onSubmit,
  profiles,
}: {
  activeAnchor: ProjectPullRequestCommentAnchor | null;
  comments: ProjectPullRequestComment[];
  isSending: boolean;
  onCancel: () => void;
  onSubmit: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<unknown>;
  profiles?: UserProfileLookup;
}) {
  if (comments.length === 0 && !activeAnchor) return null;

  return (
    <div
      className="border-border/50 border-y bg-background px-3 py-2 font-sans"
      data-testid="project-inline-comment-thread"
    >
      {comments.length > 0 ? (
        <div className="divide-y divide-border/50 rounded-lg border border-border/60 bg-card">
          {comments.map((comment) => (
            <article
              className="space-y-1.5 px-3 py-2.5"
              data-testid="project-inline-comment"
              key={comment.id}
            >
              <div className="flex min-w-0 items-center gap-2 text-xs">
                <MessageSquareText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate font-medium text-foreground">
                  {commentAuthor(comment.author, profiles)}
                </span>
                <span className="shrink-0 text-muted-foreground">
                  {commentDate(comment.createdAt)}
                </span>
              </div>
              <Markdown
                className="text-sm"
                content={comment.content}
                interactive={false}
              />
            </article>
          ))}
        </div>
      ) : null}
      {activeAnchor ? (
        <ForumComposer
          className="mt-2 border border-border/60 bg-background/70"
          disabled={isSending}
          header={
            <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
              <FileCode2 className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{activeAnchor.path}</span>
              <span className="shrink-0">
                {activeAnchor.side === "new" ? "+" : "-"}
                {activeAnchor.line}
              </span>
            </div>
          }
          isSending={isSending}
          onCancel={onCancel}
          onSubmit={onSubmit}
          placeholder="Leave a comment on this line…"
          profiles={profiles}
        />
      ) : null}
    </div>
  );
}
