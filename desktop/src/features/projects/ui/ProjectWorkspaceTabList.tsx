import { ArrowLeft } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { TabsList, TabsTrigger } from "@/shared/ui/tabs";

export const PROJECT_TAB_TRIGGER_CLASS =
  "h-7 shrink-0 rounded-full bg-muted/30 px-3 text-xs font-medium leading-5 tracking-tight text-muted-foreground shadow-none transition-colors hover:bg-muted/55 hover:text-foreground data-[state=active]:bg-muted data-[state=active]:text-foreground data-[state=active]:shadow-none";

export const PROJECT_TAB_SELECTED_CLASS = "bg-muted text-foreground";
const PROJECT_TAB_ICON_BUTTON_CLASS =
  "h-7 w-7 shrink-0 rounded-full bg-muted/30 p-1.5 text-muted-foreground shadow-none transition-colors hover:bg-muted/55 hover:text-foreground";

function ProjectTabLabel({ children }: { children: string }) {
  return <span>{children}</span>;
}

export function ProjectTabsList({
  onBack,
  prsActive,
}: {
  onBack: () => void;
  prsActive?: boolean;
}) {
  return (
    <div className="flex h-full min-w-0 max-w-full flex-none items-center gap-1.5 overflow-x-auto scrollbar-none">
      <button
        aria-label="Back"
        className={PROJECT_TAB_ICON_BUTTON_CLASS}
        data-testid="project-workspace-back"
        onClick={onBack}
        title="Back"
        type="button"
      >
        <ArrowLeft className="h-full w-full" strokeWidth={2} />
      </button>
      <TabsList className="h-full min-w-0 max-w-full flex-none justify-start gap-1.5 overflow-x-auto bg-transparent p-0 scrollbar-none">
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="overview">
          <ProjectTabLabel>Overview</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="files">
          <ProjectTabLabel>Files</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="activity">
          <ProjectTabLabel>Commits</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="issues">
          <ProjectTabLabel>Tasks</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger
          aria-current={prsActive ? "page" : undefined}
          className={cn(
            PROJECT_TAB_TRIGGER_CLASS,
            prsActive && PROJECT_TAB_SELECTED_CLASS,
          )}
          value="prs"
        >
          <ProjectTabLabel>Review</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="channels">
          <ProjectTabLabel>Channels</ProjectTabLabel>
        </TabsTrigger>
        <TabsTrigger className={PROJECT_TAB_TRIGGER_CLASS} value="contributors">
          <ProjectTabLabel>Contributors</ProjectTabLabel>
        </TabsTrigger>
      </TabsList>
    </div>
  );
}
