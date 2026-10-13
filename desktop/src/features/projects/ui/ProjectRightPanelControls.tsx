import { FolderGit2, MessageSquareText } from "lucide-react";

import { usePhoneLayout } from "@/shared/lib/phoneLayout";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";

export type ProjectRightPanelMode = "chat" | "repository";

export function ProjectRightPanelControls({
  collapsed,
  mode,
  onCollapse,
  onExpand,
  onModeChange,
}: {
  collapsed: boolean;
  mode: ProjectRightPanelMode;
  onCollapse: () => void;
  onExpand: () => void;
  onModeChange: (mode: ProjectRightPanelMode) => void;
}) {
  const repositoryOpen = !collapsed && mode === "repository";
  const phone = usePhoneLayout();
  // Phone: binary toggle between the chat/README side and the repo side, each full width. The icon names the side
  // you'll switch to.
  const phoneLabel = repositoryOpen ? "Show chat" : "Show repository";

  return (
    <Button
      aria-label={
        phone
          ? phoneLabel
          : repositoryOpen
            ? "Hide project context"
            : "Show project context"
      }
      aria-pressed={repositoryOpen}
      className={
        phone
          ? "h-7 w-auto gap-1 rounded-full border border-border px-2.5 text-xs text-sidebar-foreground hover:bg-sidebar-accent"
          : "h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
      }
      data-phone-target={phone ? (repositoryOpen ? "chat" : "repository") : undefined}
      data-testid="project-right-panel-repository-tab"
      onClick={() => {
        if (repositoryOpen) {
          onCollapse();
          return;
        }
        onModeChange("repository");
        onExpand();
      }}
      size={phone ? "sm" : "icon"}
      title={phone ? phoneLabel : "Project context"}
      type="button"
      variant="ghost"
    >
      {phone ? (
        repositoryOpen ? (
          <>
            <MessageSquareText aria-hidden className="h-3.5 w-3.5" />
            <span>Chat</span>
          </>
        ) : (
          <>
            <FolderGit2 aria-hidden className="h-3.5 w-3.5" />
            <span>Repo</span>
          </>
        )
      ) : (
      <DrawerPanelIcon
        className="-scale-x-100"
        side={repositoryOpen ? "left" : "right"}
        testId="project-right-panel-repository-icon"
      />
      )}
    </Button>
  );
}
