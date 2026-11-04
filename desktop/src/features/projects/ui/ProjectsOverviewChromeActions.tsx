// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { MessageCircle } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";

export function ProjectsOverviewChromeActions({
  chatOpen,
  contextOpen,
  onToggleChat,
  onToggleContext,
  sectionTitle,
}: {
  chatOpen: boolean;
  contextOpen: boolean;
  onToggleChat: () => void;
  onToggleContext: () => void;
  sectionTitle: string;
}) {
  return (
    <>
      <Button
        aria-label={uiT("Chat with an agent about {0}", { p0: sectionTitle })}
        aria-pressed={chatOpen}
        className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
        data-testid="projects-overview-chat-toggle"
        onClick={onToggleChat}
        size="icon"
        title={uiT("Chat with an agent about {0}", { p0: sectionTitle })}
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
      <Button
        aria-label={
          contextOpen
            ? uiT("Hide project context")
            : uiT("Show project context")
        }
        aria-pressed={contextOpen}
        className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
        data-testid="projects-overview-context-toggle"
        onClick={onToggleContext}
        size="icon"
        title={
          contextOpen
            ? uiT("Hide project context")
            : uiT("Show project context")
        }
        type="button"
        variant="ghost"
      >
        <DrawerPanelIcon
          className="-scale-x-100"
          side={contextOpen ? "left" : "right"}
          testId="projects-overview-context-icon"
        />
      </Button>
    </>
  );
}
