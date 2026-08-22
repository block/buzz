import { Columns2, PanelRightOpen } from "lucide-react";

import {
  type ThreadViewMode,
  useThreadViewMode,
} from "@/features/channels/lib/threadViewModePreference";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/** Preserve focus only when activation did not come from a pointer click. */
export function shouldRestoreThreadToggleFocus(clickDetail: number): boolean {
  return clickDetail === 0;
}

/**
 * Both glyphs depict the layout the button switches *to*, never the current one.
 *
 * They also come from one family — each is a picture of a destination, not a verb
 * — because the user watches these two alternate in the same 28px slot. A diagram
 * flipping to an action icon reads as two different controls sharing a position.
 *
 * `columns-2` depicts the split destination, while `panel-right-open` depicts
 * the thread expanding from its right-hand pane into the larger focus surface.
 * The latter preserves the thread's spatial origin without implying browser
 * fullscreen or a separate app window.
 */
const THREAD_VIEW_MODE_TOGGLE = {
  focus: {
    // Viewing the drawer → offer the pane.
    icon: Columns2,
    label: (surface: string) => `Show ${surface} beside channel`,
    target: "split",
  },
  split: {
    // Viewing the pane → offer the drawer.
    icon: PanelRightOpen,
    label: (surface: string) => `Expand ${surface}`,
    target: "focus",
  },
} as const;

type ThreadViewModeToggleProps = {
  /** Accessible noun for the surface sharing the thread layout preference. */
  surfaceLabel?: string;
  onChange: (mode: ThreadViewMode, restoreFocus: boolean) => void;
};

/**
 * Switches an open auxiliary surface between the focus drawer and split pane.
 *
 * Threads and the agent work viewer intentionally share this control and its
 * persisted preference. `surfaceLabel` keeps the action truthful when the
 * current occupant is not a thread.
 */
export function ThreadViewModeToggle({
  onChange,
  surfaceLabel = "thread",
}: ThreadViewModeToggleProps) {
  const viewMode = useThreadViewMode();
  const { icon: Icon, label, target } = THREAD_VIEW_MODE_TOGGLE[viewMode];
  const accessibleLabel = label(surfaceLabel);

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={accessibleLabel}
          className="shrink-0"
          data-testid="thread-view-mode-toggle"
          onClick={(event) =>
            onChange(target, shouldRestoreThreadToggleFocus(event.detail))
          }
          size="icon"
          type="button"
          variant="ghost"
        >
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{accessibleLabel}</TooltipContent>
    </Tooltip>
  );
}
