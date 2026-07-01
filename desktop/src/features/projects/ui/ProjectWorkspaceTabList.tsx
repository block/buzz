import type { ProjectPullRequest } from "@/features/projects/hooks";
import { TabsList, TabsTrigger } from "@/shared/ui/tabs";

const PROJECT_TAB_TRIGGER_CLASS =
  "h-8 gap-1.5 rounded-full px-3 text-foreground hover:bg-accent hover:text-accent-foreground data-[state=active]:bg-secondary data-[state=active]:text-secondary-foreground data-[state=active]:shadow-xs";

export function ProjectTabsList() {
  return (
    <TabsList className="h-9 w-fit justify-start gap-0.5 bg-transparent p-0">
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="overview">
        Overview
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="activity">
        Activity
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="prs">
        PRs
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="files">
        Files
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="contributors">
        Contributors
      </TabsTrigger>
    </TabsList>
  );
}

export function PullRequestTabsList({
  filesCount,
  pullRequest,
}: {
  filesCount: number;
  pullRequest: ProjectPullRequest;
}) {
  const commitCount = Math.max(1, pullRequest.updateCount + 1);
  return (
    <TabsList className="h-9 w-fit justify-start gap-0.5 bg-transparent p-0">
      <TabsTrigger
        className={PROJECT_TAB_TRIGGER_CLASS}
        value="pr-conversation"
      >
        Conversation
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {pullRequest.comments.length}
        </span>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="pr-commits">
        Commits
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {commitCount}
        </span>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="pr-checks">
        Checks
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">0</span>
      </TabsTrigger>
      <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="pr-files">
        Files changed
        <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs">
          {filesCount}
        </span>
      </TabsTrigger>
    </TabsList>
  );
}
