import { ChevronDown, ChevronUp, History, MessageSquare } from "lucide-react";
import * as React from "react";

import type { ProjectIssue } from "@/features/projects/hooks";
import {
  formatExactTimestamp,
  pluralize,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { ProfileAuthorName } from "./ProjectProfileIdentity";
import { ProjectRichContent } from "./ProjectRichContent";

const COLLAPSED_COMMENT_COUNT = 3;

export function ProjectIssueCommentTimeline({
  comments,
  profiles,
}: {
  comments: ProjectIssue["comments"];
  profiles?: UserProfileLookup;
}) {
  const [isCollapsed, setIsCollapsed] = React.useState(false);
  const [isExpanded, setIsExpanded] = React.useState(false);
  const orderedComments = React.useMemo(
    () =>
      [...comments].sort(
        (left, right) =>
          left.createdAt - right.createdAt || left.id.localeCompare(right.id),
      ),
    [comments],
  );
  const earlierCommentCount = Math.max(
    0,
    orderedComments.length - COLLAPSED_COMMENT_COUNT,
  );
  const visibleComments =
    isExpanded || earlierCommentCount === 0
      ? orderedComments
      : orderedComments.slice(-COLLAPSED_COMMENT_COUNT);
  const displayedComments = isCollapsed ? [] : visibleComments;

  if (orderedComments.length === 0) {
    return <p className="text-sm text-muted-foreground">No comments yet.</p>;
  }

  return (
    <div className="-mx-4 overflow-hidden border-border/50 border-b">
      <button
        aria-expanded={!isCollapsed}
        className="flex min-h-10 w-full items-center gap-2 px-3 py-2.5 text-sm font-semibold text-muted-foreground transition-colors hover:text-foreground"
        data-testid="project-issue-comment-history-toggle"
        onClick={() => setIsCollapsed((current) => !current)}
        type="button"
      >
        <span className="relative flex w-5 shrink-0 justify-center self-stretch">
          {!isCollapsed ? (
            <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
          ) : null}
          <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-primary/10 text-primary ring-1 ring-primary/35">
            <History className="h-3 w-3" />
          </span>
        </span>
        <span className="flex min-h-5 min-w-0 flex-1 items-center text-left">
          {isCollapsed
            ? `Show ${pluralize(orderedComments.length, "earlier comment")}`
            : "Collapse comment history"}
        </span>
        {isCollapsed ? (
          <ChevronDown className="mt-0.5 h-3.5 w-3.5" />
        ) : (
          <ChevronUp className="mt-0.5 h-3.5 w-3.5" />
        )}
      </button>

      {!isCollapsed && earlierCommentCount > 0 && !isExpanded ? (
        <button
          className="flex min-h-10 w-full items-center gap-2 px-3 py-2.5 text-sm font-semibold text-muted-foreground transition-colors hover:text-foreground"
          data-testid="project-issue-earlier-comments"
          onClick={() => setIsExpanded(true)}
          type="button"
        >
          <span className="relative flex w-5 shrink-0 justify-center self-stretch">
            <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
            <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-background ring-1 ring-border/70">
              <ChevronDown className="h-3 w-3" />
            </span>
          </span>
          <span className="min-w-0 flex-1 text-left">
            Show {pluralize(earlierCommentCount, "earlier comment")}
          </span>
        </button>
      ) : null}

      {displayedComments.map((comment, index) => (
        <div
          className="flex min-h-10 min-w-0 items-start gap-2 px-3 py-2.5 text-sm text-muted-foreground"
          data-testid="project-issue-comment-timeline-row"
          key={comment.id}
        >
          <div className="relative flex w-5 shrink-0 justify-center self-stretch">
            {index < displayedComments.length - 1 ? (
              <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
            ) : null}
            <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-background ring-1 ring-border/70">
              <MessageSquare className="h-3 w-3" />
            </span>
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center">
              <span className="min-w-0 flex-1 truncate">
                <ProfileAuthorName pubkey={comment.author}>
                  {resolveUserLabel({ profiles, pubkey: comment.author })}
                </ProfileAuthorName>
              </span>
              <span
                className="ml-auto w-20 shrink-0 text-right text-xs text-muted-foreground/70"
                title={formatExactTimestamp(comment.createdAt)}
              >
                {relativeTime(comment.createdAt)}
              </span>
            </div>
            <ProjectRichContent
              className="mt-1 text-sm text-foreground/90"
              content={comment.content}
              tags={comment.tags}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
