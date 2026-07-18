import { BookOpen } from "lucide-react";

import type { ProjectPullRequest } from "@/features/projects/hooks";
import { TabsList, TabsTrigger } from "@/shared/ui/tabs";

const PROJECT_TAB_TRIGGER_BASE_CLASS =
  "rounded-full text-foreground hover:bg-accent hover:text-accent-foreground data-[state=active]:bg-sidebar-active data-[state=active]:text-sidebar-active-foreground data-[state=active]:shadow-xs data-[state=active]:hover:bg-sidebar-active data-[state=active]:hover:text-sidebar-active-foreground";

const PROJECT_TAB_TRIGGER_CLASS = `${PROJECT_TAB_TRIGGER_BASE_CLASS} h-8 gap-1.5 px-3 text-xs xl:px-3.5 xl:text-sm`;

const PROJECT_ICON_TAB_TRIGGER_CLASS = `${PROJECT_TAB_TRIGGER_BASE_CLASS} h-8 w-8 shrink-0 border border-border/60 p-2 data-[state=active]:border-transparent`;

const PROJECT_TAB_SELECTED_CLASS =
  " bg-sidebar-active text-sidebar-active-foreground shadow-xs hover:bg-sidebar-active hover:text-sidebar-active-foreground";

export function ProjectTabsList({ prsActive }: { prsActive?: boolean }) {
  return (
    <TabsList className="h-9 w-fit justify-start gap-1 bg-transparent p-0">
      <TabsTrigger
        aria-label="Overview"
        className={PROJECT_ICON_TAB_TRIGGER_CLASS}
        title="Overview"
        value="overview"
      >
        <BookOpen className="h-full w-full" strokeWidth={2} />
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="files">
        Files
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="activity">
        Commits
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="issues">
        Issues
      </TabsTrigger>
      <TabsTrigger
        className={`${PROJECT_TAB_TRIGGER_CLASS}${
          prsActive ? PROJECT_TAB_SELECTED_CLASS : ""
        }`}
        value="prs"
      >
        PRs
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="contributors">
        Contributors
      </TabsTrigger>
    </TabsList>
  );
}

const PR_TAB_TRIGGER_CLASS =
  "h-9 gap-1.5 rounded-none border-b-2 border-transparent px-0 text-muted-foreground hover:text-foreground data-[state=active]:border-foreground data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none";

export function PullRequestTabsList({
  filesCount,
  pullRequest,
}: {
  filesCount: number;
  pullRequest: ProjectPullRequest;
}) {
  const commitCount = Math.max(1, pullRequest.updateCount + 1);
  return (
    <TabsList className="h-9 w-fit justify-start gap-6 bg-transparent p-0">
      <TabsTrigger className={PR_TAB_TRIGGER_CLASS} value="pr-conversation">
        Conversation
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {pullRequest.comments.length}
        </span>
      </TabsTrigger>
      <TabsTrigger className={PR_TAB_TRIGGER_CLASS} value="pr-commits">
        Commits
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {commitCount}
        </span>
      </TabsTrigger>
      <TabsTrigger className={PR_TAB_TRIGGER_CLASS} value="pr-checks">
        Checks
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">0</span>
      </TabsTrigger>
      <TabsTrigger className={PR_TAB_TRIGGER_CLASS} value="pr-files">
        Files changed
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {filesCount}
        </span>
      </TabsTrigger>
    </TabsList>
  );
}
