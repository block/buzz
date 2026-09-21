import { MessageCircle } from "lucide-react";

import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
  const chatLabel = t("projects.overview-chrome.chat-aria", {
    section: sectionTitle,
  });
  const contextLabel = contextOpen
    ? t("projects.overview-chrome.context-hide")
    : t("projects.overview-chrome.context-show");
  return (
    <>
      <Button
        aria-label={chatLabel}
        aria-pressed={chatOpen}
        className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
        data-testid="projects-overview-chat-toggle"
        onClick={onToggleChat}
        size="icon"
        title={chatLabel}
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
        aria-label={contextLabel}
        aria-pressed={contextOpen}
        className="h-7 w-7 text-sidebar-foreground hover:bg-sidebar-accent"
        data-testid="projects-overview-context-toggle"
        onClick={onToggleContext}
        size="icon"
        title={contextLabel}
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
