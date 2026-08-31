import { MessageCircle } from "lucide-react";

import {
  toggleTerminalPanel,
  useTerminalPanel,
} from "@/features/terminal/terminalPanelStore";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";
import { TerminalPanelIcon } from "@/shared/ui/TerminalPanelIcon";

export type ProjectRightPanelMode = "chat" | "repository";

export function ProjectChatPanelControl({
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
  const chatOpen = !collapsed && mode === "chat";
  return (
    <Button
      aria-label={chatOpen ? "Hide project chat" : "Show project chat"}
      aria-pressed={chatOpen}
      className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
      data-testid="project-right-panel-chat-tab"
      onClick={() => {
        if (chatOpen) {
          onCollapse();
          return;
        }
        onModeChange("chat");
        onExpand();
      }}
      size="icon"
      title="Project chat"
      type="button"
      variant="ghost"
    >
      <MessageCircle
        className={cn(
          "h-4 w-4 transition-opacity duration-200 ease-linear",
          chatOpen ? "opacity-100" : "opacity-60",
        )}
      />
    </Button>
  );
}

export function ProjectRightPanelControls({
  collapsed,
  mode,
  onCollapse,
  onExpand,
  onModeChange,
  terminalAvailable,
}: {
  collapsed: boolean;
  mode: ProjectRightPanelMode;
  onCollapse: () => void;
  onExpand: () => void;
  onModeChange: (mode: ProjectRightPanelMode) => void;
  terminalAvailable: boolean;
}) {
  const terminalPanel = useTerminalPanel();
  const terminalOpen = terminalPanel.mode !== "closed";
  const repositoryOpen = !collapsed && mode === "repository";

  return (
    <div className="flex items-center gap-0.5">
      <Button
        aria-label={terminalOpen ? "Hide Buzz Term" : "Open Buzz Term"}
        aria-pressed={terminalOpen}
        className={cn(
          "h-7 w-7 text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
          terminalOpen && "bg-sidebar-accent text-sidebar-accent-foreground",
        )}
        data-testid="project-terminal-toggle"
        disabled={!terminalAvailable}
        onClick={toggleTerminalPanel}
        size="icon"
        title="Buzz Term (⌘J)"
        type="button"
        variant="ghost"
      >
        <TerminalPanelIcon
          className="w-[1.1rem]"
          data-testid="project-terminal-icon"
          open={terminalOpen}
        />
      </Button>
      <ProjectChatPanelControl
        collapsed={collapsed}
        mode={mode}
        onCollapse={onCollapse}
        onExpand={onExpand}
        onModeChange={onModeChange}
      />
      <Button
        aria-label={
          repositoryOpen ? "Hide project context" : "Show project context"
        }
        aria-pressed={repositoryOpen}
        className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
        data-testid="project-right-panel-repository-tab"
        onClick={() => {
          if (repositoryOpen) {
            onCollapse();
            return;
          }
          onModeChange("repository");
          onExpand();
        }}
        size="icon"
        title="Project context"
        type="button"
        variant="ghost"
      >
        <DrawerPanelIcon
          className="-scale-x-100"
          side={repositoryOpen ? "left" : "right"}
          testId="project-right-panel-repository-icon"
        />
      </Button>
    </div>
  );
}
