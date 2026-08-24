import { ListTree, PanelRightOpen } from "lucide-react";

import {
  setThreadTimelineMode,
  type ThreadTimelineMode,
  useThreadTimelineMode,
} from "@/features/channels/lib/threadTimelineModePreference";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const THREAD_TIMELINE_MODE_TOGGLE: Record<
  ThreadTimelineMode,
  {
    icon: typeof ListTree;
    label: string;
    target: ThreadTimelineMode;
  }
> = {
  inline: {
    icon: PanelRightOpen,
    label: "Use thread panel",
    target: "panel",
  },
  panel: {
    icon: ListTree,
    label: "Show thread replies inline",
    target: "inline",
  },
};

export function ThreadTimelineModeToggle() {
  const mode = useThreadTimelineMode();
  const { icon: Icon, label, target } = THREAD_TIMELINE_MODE_TOGGLE[mode];

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className="shrink-0"
          data-testid="thread-timeline-mode-toggle"
          onClick={() => setThreadTimelineMode(target)}
          size="icon"
          title={label}
          type="button"
          variant={mode === "inline" ? "secondary" : "outline"}
        >
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
