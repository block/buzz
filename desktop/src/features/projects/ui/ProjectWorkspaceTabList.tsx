import { BookOpen } from "lucide-react";

import type { ProjectPullRequest } from "@/features/projects/hooks";
import { cn } from "@/shared/lib/cn";
import { TabsList, TabsTrigger } from "@/shared/ui/tabs";

const PROJECT_TAB_TRIGGER_CLASS =
  "relative h-full shrink-0 rounded-none px-2.5 text-base leading-5 tracking-tight text-muted-foreground shadow-none after:absolute after:inset-x-2.5 after:bottom-0 after:h-0.5 after:bg-current after:opacity-0 after:transition-opacity after:content-[''] hover:bg-transparent hover:font-semibold hover:text-foreground hover:after:opacity-100 data-[state=active]:bg-transparent data-[state=active]:font-semibold data-[state=active]:text-foreground data-[state=active]:shadow-none data-[state=active]:after:opacity-100";

const PROJECT_TAB_SELECTED_CLASS =
  "font-semibold text-foreground after:opacity-100";
const PROJECT_OVERVIEW_TAB_CLASS =
  "h-8 w-8 shrink-0 rounded-md p-2 text-muted-foreground shadow-none hover:bg-muted/50 hover:text-foreground data-[state=active]:bg-muted/50 data-[state=active]:text-foreground data-[state=active]:shadow-none";

function ProjectTabLabel({ children }: { children: string }) {
  return (
    <span className="grid">
      <span
        aria-hidden="true"
        className="invisible col-start-1 row-start-1 font-semibold"
      >
        {children}
      </span>
      <span className="col-start-1 row-start-1">{children}</span>
    </span>
  );
}

export function ProjectTabsList({ prsActive }: { prsActive?: boolean }) {
  return (
    <TabsList className="h-full min-w-0 max-w-full flex-none justify-start gap-1 overflow-x-auto bg-transparent p-0 scrollbar-none">
      <TabsTrigger
        aria-label="Overview"
        className={PROJECT_OVERVIEW_TAB_CLASS}
        title="README"
        value="overview"
      >
        <BookOpen className="h-full w-full" strokeWidth={2} />
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="files">
        <ProjectTabLabel>Files</ProjectTabLabel>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="activity">
        <ProjectTabLabel>Commits</ProjectTabLabel>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="issues">
        <ProjectTabLabel>Issues</ProjectTabLabel>
      </TabsTrigger>
      <TabsTrigger
        aria-current={prsActive ? "page" : undefined}
        className={cn(
          PROJECT_TAB_TRIGGER_CLASS,
          prsActive && PROJECT_TAB_SELECTED_CLASS,
        )}
        value="prs"
      >
        <ProjectTabLabel>Pull Request</ProjectTabLabel>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="contributors">
        <ProjectTabLabel>Contributors</ProjectTabLabel>
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
