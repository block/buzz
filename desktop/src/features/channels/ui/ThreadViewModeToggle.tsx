import { Columns2, PanelRightOpen } from "lucide-react";

import {
  type ThreadViewMode,
  useThreadViewMode,
} from "@/features/channels/lib/threadViewModePreference";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

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
    label: "Show thread beside channel",
    target: "split",
  },
  split: {
    // Viewing the pane → offer the drawer.
    icon: PanelRightOpen,
    label: "Expand thread",
    target: "focus",
  },
} as const;

/**
 * Switches an open thread between the focus drawer and the split pane.
 *
 * Writes straight to the persisted preference, so the control doubles as the
 * setting: choosing a layout here is choosing how threads open from now on. That
 * is the intended behaviour — the place you form the opinion is the place you are
 * looking at the thread, not a settings page — and it is why the label names an
 * action rather than a state.
 *
 * A tooltip is mandatory, not decoration. A lone toggle showing its target is the
 * conventional pattern and still routinely misread as showing the current state;
 * the glyph cannot disambiguate itself, so the label has to.
 */
export function ThreadViewModeToggle({
  onChange,
}: {
  onChange: (mode: ThreadViewMode) => void;
}) {
  const viewMode = useThreadViewMode();
  const { icon: Icon, label, target } = THREAD_VIEW_MODE_TOGGLE[viewMode];

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className="shrink-0"
          data-testid="thread-view-mode-toggle"
          onClick={() => onChange(target)}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
