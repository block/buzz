import {
  Check,
  CircleDot,
  FolderGit2,
  GitCommitHorizontal,
  GitPullRequest,
  MessageSquare,
  UserPlus,
  X,
} from "lucide-react";
import type { ComponentType } from "react";

import { cn } from "@/shared/lib/cn";

export type ProjectEventKind =
  | "repository"
  | "commit"
  | "pull-request"
  | "issue"
  | "comment"
  | "approval"
  | "changes-requested"
  | "review-request";

export const PROJECT_EVENT_VISUALS: Record<
  ProjectEventKind,
  {
    icon: ComponentType<{ className?: string }>;
    iconClassName: string;
    badgeClassName: string;
    detailClassName: string;
  }
> = {
  repository: {
    icon: FolderGit2,
    iconClassName: "text-primary",
    badgeClassName: "bg-primary/10 text-primary",
    detailClassName: "border-primary/30 text-primary",
  },
  commit: {
    icon: GitCommitHorizontal,
    iconClassName: "text-primary",
    badgeClassName: "bg-primary/10 text-primary",
    detailClassName: "border-primary/30 text-primary",
  },
  "pull-request": {
    icon: GitPullRequest,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
    detailClassName:
      "border-green-600/30 text-green-700 dark:border-green-500/30 dark:text-green-400",
  },
  issue: {
    icon: CircleDot,
    iconClassName: "text-orange-500",
    badgeClassName: "bg-orange-500/10 text-orange-700 dark:text-orange-300",
    detailClassName:
      "border-orange-500/30 text-orange-700 dark:border-orange-500/30 dark:text-orange-300",
  },
  comment: {
    icon: MessageSquare,
    iconClassName: "text-muted-foreground",
    badgeClassName: "bg-muted text-muted-foreground",
    detailClassName: "border-border/60 text-muted-foreground",
  },
  approval: {
    icon: Check,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
    detailClassName:
      "border-green-600/30 text-green-700 dark:border-green-500/30 dark:text-green-400",
  },
  "changes-requested": {
    icon: X,
    iconClassName: "text-destructive",
    badgeClassName: "bg-destructive/10 text-destructive",
    detailClassName: "border-destructive/40 text-destructive",
  },
  "review-request": {
    icon: UserPlus,
    iconClassName: "text-blue-600 dark:text-blue-400",
    badgeClassName:
      "bg-blue-600/10 text-blue-700 dark:bg-blue-500/10 dark:text-blue-300",
    detailClassName:
      "border-blue-600/30 text-blue-700 dark:border-blue-500/30 dark:text-blue-300",
  },
};

export function ProjectEventTypeIcon({
  className,
  kind,
}: {
  className?: string;
  kind: ProjectEventKind;
}) {
  const visual = PROJECT_EVENT_VISUALS[kind];
  const Icon = visual.icon;

  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full ring-1 ring-border/60",
        visual.badgeClassName,
        className,
      )}
    >
      <Icon className={cn("h-3 w-3", visual.iconClassName)} />
    </span>
  );
}
